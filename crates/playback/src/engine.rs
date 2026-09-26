//! The playback engine (§47a).
//!
//! Turns "what time is it?" into "which pixels and which samples?".
//!
//! # The clock runs the show
//!
//! §20a.1: the audio device advances the clock, the clock chooses the video
//! frame, and late frames are dropped rather than queued. Nothing here consults
//! a wall-clock timer to decide what to show.
//!
//! # Where frames come from
//!
//! Three places, checked in order: the frame cache (§18), then §47a.3's
//! decode-ahead ring, then an inline decode. The ring is filled by a thread
//! started when playback starts and stopped when it stops — a paused editor has
//! nothing to run ahead of, and holding a decode thread for it works against
//! §81's idle budget.
//!
//! Scrubbing still decodes inline, which is correct: the user is jumping around
//! rather than moving forward, so there is nothing to predict. That path is
//! what §13.1's all-intra proxies make cheap.

use std::sync::Arc;

use bettercut_foundation::{ClipId, MediaId, MediaTime, TimelineTime, TrackId};
use bettercut_media::{MediaAsset, NeverCancelled, SeekMode, VideoFrame};
use bettercut_project_format::Project;
use bettercut_timeline::{Clip, Sequence, Transform, TransitionKind, VideoClip};

use crate::cache::{FrameCache, FrameKey};
use crate::error::PlaybackError;
use crate::frame_source::FrameSource;

/// One video layer to draw, resolved for a given instant.
pub struct ResolvedLayer {
    pub clip: ClipId,
    pub track: TrackId,
    pub frame: Arc<VideoFrame>,
    /// How it is drawn, carried whole from the request (§46).
    ///
    /// One value rather than a field each, so the hop from a request to a
    /// resolved layer and on to a renderer layer cannot lose one on the way.
    pub look: bettercut_timeline::ClipLook,
}

/// One layer to draw, resolved except for the frame itself.
///
/// The part of "what is on screen at this instant" that does not depend on how
/// the frame is obtained: which clips, where in their source, and what they
/// look like there.
#[derive(Debug, Clone, Copy)]
pub struct LayerRequest {
    pub clip: ClipId,
    pub track: TrackId,
    pub source: LayerSource,
    pub source_time: MediaTime,
    /// §24: animation applied, so the static fields are already overridden.
    pub look: bettercut_timeline::ClipLook,
    /// For a title typing itself in: how many characters to draw. `None` for
    /// everything, and always `None` for media.
    pub reveal: Option<usize>,
    /// Which angle of a multicam clip this layer asks for (`crate::compound`).
    /// `None` on everything else, which is all but one clip in a thousand.
    pub angle: Option<usize>,
}

/// Where a layer's picture comes from.
///
/// Two kinds, and the difference is only *how the picture is obtained*: a
/// decoded frame is read from a file, a title is drawn from its own text. What
/// happens afterwards — transform, opacity, colour, blur, compositing order —
/// is identical, which is why they share [`LayerRequest`] rather than the
/// renderer growing a second path (§46, §26.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerSource {
    Media(MediaId),
    /// §26: rasterized from the text clip with this id.
    Text(ClipId),
    /// The timecode and file name over the picture (`crate::burn_in`). The
    /// text itself is built where it is drawn, from the instant the layer is
    /// read at, so one line of code decides what it says for the preview and
    /// the export alike (§46).
    BurnIn,
    /// A flat colour, generated rather than read from anywhere (§25's flash).
    ///
    /// sRGB bytes, which is what the texture upload expects. It carries no
    /// picture, so it has no shape of its own either — see [`layer_transform`],
    /// which stretches it to whatever the frame is.
    Solid {
        rgb: [u8; 3],
    },
}

impl LayerSource {
    pub fn media(self) -> Option<MediaId> {
        match self {
            Self::Media(id) => Some(id),
            Self::Text(_) | Self::Solid { .. } | Self::BurnIn => None,
        }
    }
}

/// Everything visible at `position`, bottom track first (§22).
///
/// A free function, and **the only place this rule lives** (§46). Preview
/// decodes through a cache and a decode-ahead ring; export walks the file
/// sequentially at full quality from the original media. Those are genuinely
/// different strategies, but *which clips are visible, where in their source
/// they are reading, and what they look like there* must not be — and the way
/// to guarantee that is for both to call this rather than each to re-derive it.
pub fn layer_requests(
    project: &Project,
    sequence: &Sequence,
    position: TimelineTime,
) -> Vec<LayerRequest> {
    plan(project, sequence, position, 0)
}

/// [`layer_requests`], with how many compounds deep this plan already is.
///
/// A compound clip is not decoded: its layers are the layers of the sequence
/// inside it, which is what the expansion at the end of this function does
/// (`crate::compound`). The depth is what stops a compound that contains
/// itself from planning frames forever.
pub(crate) fn plan(
    project: &Project,
    sequence: &Sequence,
    position: TimelineTime,
    depth: u8,
) -> Vec<LayerRequest> {
    // Render in place: a baked stretch plays as the one file it was written
    // to, rather than as the twenty layers that made it
    // (`bettercut_timeline::render`). The overlays are in the file as well —
    // they were on when it was baked, and changing one changes the hash that
    // says whether the file is still the edit.
    if let Some(render) = crate::rendered::usable(project, sequence, position) {
        return vec![baked_layer(render, position)];
    }

    let mut requests = Vec::new();

    // §20a.4: while any picture track is soloed, only those are on screen.
    // Asked once rather than per track — it is a fact about the lane.
    let soloed = sequence.video_tracks.iter().any(|track| track.solo);
    // And while any picture *clip* is soloed, only those clips are on screen.
    let picture_soloed = sequence.picture_clip_soloed();

    for track in &sequence.video_tracks {
        if !bettercut_timeline::track_plays(track.enabled, track.solo, soloed) {
            continue; // hidden, or not the one being soloed (§8)
        }
        let clips = track.clips();
        let index = clips.partition_point(|c| c.timeline().end <= position);
        let Some(clip) = clips.get(index).filter(|c| c.timeline().contains(position)) else {
            continue;
        };
        if !sequence.clip_plays(clip.id, true, picture_soloed) {
            continue; // another clip is soloed, and this is not it
        }

        match transition_at(clips, index, position) {
            // §25: two clips on screen at once. The outgoing one keeps reading
            // past its out-point and the incoming one starts before its
            // in-point, which is what the handles checked by
            // `Transition::max_duration` are for.
            Some(Cut {
                outgoing,
                incoming,
                progress,
                kind: TransitionKind::Crossfade,
            }) => {
                // Outgoing first: within a track, later layers draw over
                // earlier ones, and a crossfade is the incoming picture coming
                // up *over* the outgoing one (§22).
                push_layer(&mut requests, project, track.id, outgoing, position, 1.0);
                push_layer(
                    &mut requests,
                    project,
                    track.id,
                    incoming,
                    position,
                    progress,
                );
            }
            // §25: one clip at a time, dipping to black at the cut. Nothing is
            // ever read outside a clip's own range, so this works on any cut.
            Some(Cut {
                progress,
                kind: TransitionKind::FadeThroughBlack,
                ..
            }) => {
                let fade = (2.0 * progress - 1.0).abs();
                push_layer(&mut requests, project, track.id, clip, position, fade);
            }
            // One shot at a time, like a fade through black, breaking up into
            // glitches towards the cut and snapping back together after it.
            Some(Cut {
                progress,
                kind: TransitionKind::Glitch,
                ..
            }) => {
                let before = requests.len();
                push_layer(&mut requests, project, track.id, clip, position, 1.0);
                let (amount, shake) = transition_glitch(progress);
                for request in &mut requests[before..] {
                    request.look.glitch = request.look.glitch.max(amount);
                    request.look.rgb_split = request.look.rgb_split.max(amount);
                    request.look.transform.position.x += shake;
                }
            }
            // §25's blur dissolve. Both shots are on screen, as in a
            // crossfade, and both go soft together — so the change-over
            // happens where there is least detail to notice it changing.
            // Needs no new compositing: blur is already a per-layer amount.
            Some(Cut {
                outgoing,
                incoming,
                progress,
                kind: TransitionKind::Blur,
            }) => {
                let softness = transition_blur(progress);
                push_blurred(
                    &mut requests,
                    project,
                    track.id,
                    outgoing,
                    position,
                    1.0,
                    softness,
                );
                push_blurred(
                    &mut requests,
                    project,
                    track.id,
                    incoming,
                    position,
                    progress,
                    softness,
                );
            }
            // §25's flash: the same one-clip-at-a-time shape, but what covers
            // the cut is white laid *over* the shot rather than the shot being
            // taken away. Fading the clip out instead would dip through black
            // on its way to white, which is the opposite of the effect.
            Some(Cut {
                progress,
                kind: TransitionKind::Flash,
                ..
            }) => {
                push_layer(&mut requests, project, track.id, clip, position, 1.0);
                push_flash(&mut requests, track.id, clip.id, flash_alpha(progress));
            }
            // §25's moving transitions. All three show both clips at once, like
            // a crossfade, and differ only in where the two are drawn — which
            // is why they are a handful of numbers here rather than a shader
            // each: the compositor already applies a transform per layer, and
            // preview and export both come through this one function (§46).
            Some(Cut {
                outgoing,
                incoming,
                progress,
                kind,
            }) => {
                let (out_move, in_move) = moving_transition(kind, progress);
                push_moving(
                    &mut requests,
                    project,
                    track.id,
                    outgoing,
                    position,
                    out_move,
                );
                let before = requests.len();
                push_moving(
                    &mut requests,
                    project,
                    track.id,
                    incoming,
                    position,
                    in_move,
                );
                // A wipe or an iris cuts the incoming shot to its growing
                // shape, in place of any mask of its own while it runs.
                if let Some(mask) = transition_mask(kind, progress)
                    && let Some(request) = requests.get_mut(before)
                {
                    request.look.mask = Some(mask);
                }
            }
            None => {
                // §36: something behind a clip that does not fill the frame,
                // drawn first so the clip itself lands on top of it.
                push_backdrop(&mut requests, project, sequence, track.id, clip, position);
                let before = requests.len();
                push_layer(&mut requests, project, track.id, clip, position, 1.0);
                // The punch on the beat, on every copy the smear just drew.
                let pulse = beat_pulse_at(clip.beat_pulse, &sequence.markers, position);
                if pulse != 1.0 {
                    for request in &mut requests[before..] {
                        request.look.transform.scale.x *= pulse;
                        request.look.transform.scale.y *= pulse;
                    }
                }
                push_light_leak(&mut requests, track.id, clip, position);
                push_lens_flare(&mut requests, track.id, clip, position);
            }
        }
    }

    add_shadows(&mut requests, sequence);

    // §26: text composites over every video track. Last in the list, because
    // §22 draws later layers over earlier ones.
    let text_soloed = sequence.text_tracks.iter().any(|track| track.solo);
    for track in &sequence.text_tracks {
        if !bettercut_timeline::track_plays(track.enabled, track.solo, text_soloed) {
            continue;
        }
        let Some(clip) = track.clip_at(position) else {
            continue;
        };
        // Nothing typed yet: a title is added before it says anything, and an
        // empty one is not a failure to report — there is simply no picture.
        if clip.is_blank() || !sequence.clip_plays(clip.id, clip.enabled, picture_soloed) {
            continue;
        }

        // Entrance and exit applied, in the timeline crate, once (§46).
        let look = clip.look_at(position);

        // A title is the thing most likely to be moving fast — a spin or a pop
        // is over in a third of a second — so it smears the same way a shot
        // does, from the same function.
        let copies = if clip.motion_blur {
            let earlier = TimelineTime::from_ticks(
                (position.ticks() - bettercut_foundation::TICKS_PER_SECOND / 60)
                    .max(clip.timeline.start.ticks()),
            );
            smear(
                look.transform,
                clip.look_at(earlier).transform,
                look.opacity,
            )
        } else {
            vec![(look.transform, look.opacity)]
        };

        for (transform, share) in copies {
            requests.push(LayerRequest {
                clip: clip.id,
                track: track.id,
                source: LayerSource::Text(clip.id),
                // Zero-based within the clip's own span; see `timeline::text`.
                source_time: source_time_of(clip.timeline.start, clip.source.start, position),
                look: bettercut_timeline::ClipLook {
                    corner_pin: Default::default(),
                    old_film: 0.0,
                    glow: 0.0,
                    shadow: Default::default(),
                    border: Default::default(),
                    sharpen: 0.0,
                    // A title's colours are chosen, not filmed: nothing to grade.
                    lut: None,
                    rgb_split: 0.0,
                    glitch: 0.0,
                    pixelate: 0.0,
                    zoom_blur: 0.0,
                    lens: 0.0,
                    tilt_band: 0.0,
                    tilt_centre: 0.5,
                    posterise: 0.0,
                    smooth_skin: 0.0,
                    vignette: 0.0,
                    reflection: bettercut_timeline::Reflection::None,
                    // A title is generated at exactly the size it is drawn at;
                    // there is no surplus source to crop away.
                    crop: bettercut_timeline::Crop::NONE,
                    transform,
                    opacity: share,
                    color: bettercut_timeline::ColorAdjust::default(),
                    blur: 0.0,
                    // A title is drawn, not filmed: there is no screen behind it,
                    // and nothing was drawn on it to mask.
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    // §26: a title covers what is beneath it.
                    blend: bettercut_timeline::BlendMode::Normal,
                },
                reveal: look.reveal,
                angle: None,
            });
        }
    }

    push_burn_in(&mut requests, sequence, position);
    push_visualizer(&mut requests, sequence, position);
    push_progress_bar(&mut requests, sequence, position);
    push_watermark(&mut requests, project, sequence);

    crate::compound::expand(project, requests, depth, plan)
}

/// The one layer a baked stretch plays: the file, read at the instant inside
/// it, with nothing done to it.
///
/// On the lane of none, like the overlays: an adjustment lane must not grade a
/// picture that already has that lane's grade baked into it.
fn baked_layer(render: &bettercut_timeline::RenderedRange, position: TimelineTime) -> LayerRequest {
    LayerRequest {
        clip: render.clip,
        track: TrackId::from_u128(0),
        source: LayerSource::Media(render.media),
        source_time: MediaTime::from_ticks(position.ticks() - render.range.start.ticks()),
        look: bettercut_timeline::ClipLook {
            corner_pin: Default::default(),
            old_film: 0.0,
            glow: 0.0,
            shadow: Default::default(),
            border: Default::default(),
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            pixelate: 0.0,
            zoom_blur: 0.0,
            lens: 0.0,
            tilt_band: 0.0,
            tilt_centre: 0.5,
            posterise: 0.0,
            smooth_skin: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform::default(),
            opacity: 1.0,
            color: bettercut_timeline::ColorAdjust::IDENTITY,
            blur: 0.0,
            chroma_key: None,
            luma_key: None,
            mask: None,
            blend: bettercut_timeline::BlendMode::Normal,
        },
        reveal: None,
        angle: None,
    }
}

/// The timecode and file name over the picture, for a copy sent out for notes.
///
/// On the lane of none, like the watermark and the progress bar: it is about
/// the export rather than about any clip, and no adjustment should grade it.
fn push_burn_in(requests: &mut Vec<LayerRequest>, sequence: &Sequence, position: TimelineTime) {
    let burn = sequence.master.burn_in.clamped();
    if !burn.is_visible() {
        return;
    }
    let (place, anchor) = burn.placement();
    requests.push(LayerRequest {
        clip: ClipId::from_u128(0),
        track: TrackId::from_u128(0),
        source: LayerSource::BurnIn,
        // Where on the timeline it is being drawn: what the text is made from.
        source_time: MediaTime::from_ticks(position.ticks()),
        look: bettercut_timeline::ClipLook {
            corner_pin: Default::default(),
            old_film: 0.0,
            glow: 0.0,
            shadow: Default::default(),
            border: Default::default(),
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            pixelate: 0.0,
            zoom_blur: 0.0,
            lens: 0.0,
            tilt_band: 0.0,
            tilt_centre: 0.5,
            posterise: 0.0,
            smooth_skin: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform {
                position: bettercut_timeline::Vec2::new(place[0], place[1]),
                anchor: bettercut_timeline::Vec2::new(anchor[0], anchor[1]),
                ..Transform::default()
            },
            opacity: 1.0,
            color: bettercut_timeline::ColorAdjust::IDENTITY,
            blur: 0.0,
            chroma_key: None,
            luma_key: None,
            mask: None,
            blend: bettercut_timeline::BlendMode::Normal,
        },
        reveal: None,
        angle: None,
    });
}

/// What the burn-in says at `position`, and the title clip that draws it.
///
/// `None` when there is nothing to say. Shared by the preview and the export,
/// so the copy sent out carries exactly what the editor was looking at (§46).
pub fn burn_in_clip(
    project: &Project,
    sequence: &Sequence,
    position: TimelineTime,
) -> Option<bettercut_timeline::TextClip> {
    let burn = sequence.master.burn_in.clamped();
    if !burn.is_visible() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if burn.timecode {
        parts.push((sequence.start_timecode + position).format_timecode());
    }
    if burn.file_name {
        // The file the topmost picture at this instant came from — the one the
        // viewer is actually looking at.
        let name = sequence
            .video_tracks
            .iter()
            .rev()
            .find_map(|track| {
                track
                    .clips()
                    .iter()
                    .find(|clip| clip.timeline.contains(position))
                    .and_then(|clip| project.media_asset(clip.media_id))
                    .map(|asset| asset.display_name().to_owned())
            })
            .unwrap_or_default();
        if !name.is_empty() {
            parts.push(name);
        }
    }
    if parts.is_empty() {
        return None;
    }
    let mut clip = bettercut_timeline::TextClip::new(parts.join("   "), TimelineTime::ZERO).ok()?;
    // Plain, monospaced-looking and readable over anything: white with a dark
    // box behind it, which is what every burn-in in every editor looks like
    // and for the same reason.
    clip.style.size = burn.size * sequence.resolution.height.max(1) as f32;
    clip.style.stroke = None;
    clip.style.background = Some(bettercut_text::Background {
        color: bettercut_text::Rgba::new(0, 0, 0, 160),
        padding: clip.style.size * 0.25,
        corner_radius: clip.style.size * 0.15,
    });
    Some(clip)
}

/// The progress bar, over everything, filled as far as `position` is through
/// the sequence. On a lane of its own (a track id belonging to no track), so
/// no adjustment grades it.
fn push_progress_bar(
    requests: &mut Vec<LayerRequest>,
    sequence: &Sequence,
    position: TimelineTime,
) {
    let bar = sequence.master.progress_bar.clamped();
    let Some(filled) = progress_bar_fill(sequence, position) else {
        return;
    };
    if !bar.is_visible() || filled <= 0.0 {
        return;
    }
    requests.push(LayerRequest {
        clip: ClipId::from_u128(0),
        track: TrackId::from_u128(0),
        source: LayerSource::Solid { rgb: bar.colour },
        source_time: MediaTime::ZERO,
        look: bettercut_timeline::ClipLook {
            corner_pin: Default::default(),
            old_film: 0.0,
            glow: 0.0,
            shadow: Default::default(),
            border: Default::default(),
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            pixelate: 0.0,
            zoom_blur: 0.0,
            lens: 0.0,
            tilt_band: 0.0,
            tilt_centre: 0.5,
            posterise: 0.0,
            smooth_skin: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            // From the frame's top-left or bottom-left corner, as wide as the
            // video is played and as tall as the bar.
            transform: Transform {
                position: bettercut_timeline::Vec2::new(-0.5, if bar.top { -0.5 } else { 0.5 }),
                scale: bettercut_timeline::Vec2::new(filled, bar.height),
                anchor: bettercut_timeline::Vec2::new(0.0, if bar.top { 0.0 } else { 1.0 }),
                ..Transform::default()
            },
            opacity: 1.0,
            color: bettercut_timeline::ColorAdjust::IDENTITY,
            blur: 0.0,
            chroma_key: None,
            luma_key: None,
            mask: None,
            blend: bettercut_timeline::BlendMode::Normal,
        },
        reveal: None,
        angle: None,
    });
}

/// The watermark, last of all: the logo in its corner, sized as a share of the
/// frame's width whatever the picture's own shape. On the lane of none, so no
/// adjustment grades it.
fn push_watermark(requests: &mut Vec<LayerRequest>, project: &Project, sequence: &Sequence) {
    let Some(mark) = sequence
        .watermark
        .map(bettercut_timeline::watermark::Watermark::clamped)
    else {
        return;
    };
    let Some(asset) = project.media_asset(mark.media) else {
        return; // §66: a missing logo leaves the frame as it is
    };
    if asset.width == 0 || asset.height == 0 {
        return;
    }
    let output = sequence.resolution.width.max(1) as f32 / sequence.resolution.height.max(1) as f32;
    let (fit_x, _) =
        bettercut_timeline::fit_scale(asset.width as f32 / asset.height as f32, output);
    let scale = mark.size / fit_x.max(0.0001);
    let (position, anchor) = mark.placement(output);
    requests.push(LayerRequest {
        clip: ClipId::from_u128(0),
        track: TrackId::from_u128(0),
        source: LayerSource::Media(mark.media),
        source_time: MediaTime::ZERO,
        look: bettercut_timeline::ClipLook {
            corner_pin: Default::default(),
            old_film: 0.0,
            glow: 0.0,
            shadow: Default::default(),
            border: Default::default(),
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            pixelate: 0.0,
            zoom_blur: 0.0,
            lens: 0.0,
            tilt_band: 0.0,
            tilt_centre: 0.5,
            posterise: 0.0,
            smooth_skin: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform {
                position: bettercut_timeline::Vec2::new(position[0], position[1]),
                scale: bettercut_timeline::Vec2::new(scale, scale),
                anchor: bettercut_timeline::Vec2::new(anchor[0], anchor[1]),
                ..Transform::default()
            },
            opacity: mark.opacity,
            color: bettercut_timeline::ColorAdjust::IDENTITY,
            blur: 0.0,
            chroma_key: None,
            luma_key: None,
            mask: None,
            blend: bettercut_timeline::BlendMode::Normal,
        },
        reveal: None,
        angle: None,
    });
}

/// The visualizer's bars at `position`, over the pictures and titles: one solid
/// layer a bar, standing on the bottom edge or hanging from the top. On the
/// same lane of none as the progress bar, so no adjustment grades them.
fn push_visualizer(requests: &mut Vec<LayerRequest>, sequence: &Sequence, position: TimelineTime) {
    let Some(visualizer) = sequence.visualizer.as_ref() else {
        return;
    };
    let heights = visualizer.bar_heights(position);
    let count = heights.len().max(1) as f32;
    // Each bar a little narrower than its slot, so they read as bars.
    let width = 0.72 / count;
    for (index, height) in heights.into_iter().enumerate() {
        if height <= 0.0 {
            continue;
        }
        let top = visualizer.top;
        requests.push(LayerRequest {
            clip: ClipId::from_u128(0),
            track: TrackId::from_u128(0),
            source: LayerSource::Solid {
                rgb: visualizer.colour,
            },
            source_time: MediaTime::ZERO,
            look: bettercut_timeline::ClipLook {
                corner_pin: Default::default(),
                old_film: 0.0,
                glow: 0.0,
                shadow: Default::default(),
                border: Default::default(),
                sharpen: 0.0,
                lut: None,
                rgb_split: 0.0,
                glitch: 0.0,
                pixelate: 0.0,
                zoom_blur: 0.0,
                lens: 0.0,
                tilt_band: 0.0,
                tilt_centre: 0.5,
                posterise: 0.0,
                smooth_skin: 0.0,
                vignette: 0.0,
                reflection: bettercut_timeline::Reflection::None,
                crop: bettercut_timeline::Crop::NONE,
                transform: Transform {
                    position: bettercut_timeline::Vec2::new(
                        -0.5 + (index as f32 + 0.5) / count,
                        if top { -0.5 } else { 0.5 },
                    ),
                    scale: bettercut_timeline::Vec2::new(width, height),
                    anchor: bettercut_timeline::Vec2::new(0.5, if top { 0.0 } else { 1.0 }),
                    ..Transform::default()
                },
                opacity: 0.9,
                color: bettercut_timeline::ColorAdjust::IDENTITY,
                blur: 0.0,
                chroma_key: None,
                luma_key: None,
                mask: None,
                blend: bettercut_timeline::BlendMode::Normal,
            },
            reveal: None,
            angle: None,
        });
    }
}

/// How much of the frame's width the progress bar fills at `position`, 0–1;
/// `None` for an empty sequence.
pub fn progress_bar_fill(sequence: &Sequence, position: TimelineTime) -> Option<f32> {
    let length = sequence.duration().ticks();
    (length > 0).then(|| (position.ticks() as f64 / length as f64).clamp(0.0, 1.0) as f32)
}

/// Which audio clips are audible in `[position, position + duration)`.
///
/// Over tracks rather than a sequence so the mixer thread can ask it of the
/// snapshot it holds (§54): the thread never sees the project.
/// Put a shadow layer under every picture that has one.
///
/// Last, over the finished picture layers, so the shadow follows the picture
/// wherever everything before decided it goes — keyframes, an entrance, a
/// transition's move, the beat's punch — without each of those having to know
/// shadows exist. The shadow layer keeps the picture's shape (crop, corners)
/// and none of its grade or effects, which a flat dark shape cannot show; the
/// picture above it carries no shadow, which is how the compositor tells the
/// two apart.
pub fn add_shadows(requests: &mut Vec<LayerRequest>, sequence: &Sequence) {
    if !requests.iter().any(|r| r.look.shadow.is_visible()) {
        return;
    }
    let (width, height) = (sequence.resolution.width, sequence.resolution.height);
    let mut with = Vec::with_capacity(requests.len() * 2);
    for mut request in requests.drain(..) {
        let shadow = request.look.shadow;
        if shadow.is_visible() {
            let (dx, dy) = shadow.offset(width, height);
            let mut transform = request.look.transform;
            transform.position.x += dx;
            transform.position.y += dy;
            with.push(LayerRequest {
                look: bettercut_timeline::ClipLook {
                    corner_pin: Default::default(),
                    transform,
                    opacity: request.look.opacity * shadow.opacity,
                    shadow,
                    mask: None,
                    chroma_key: None,
                    luma_key: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                    ..request.look.ungraded()
                },
                reveal: None,
                angle: None,
                ..request
            });
            request.look.shadow = bettercut_timeline::Shadow::NONE;
        }
        with.push(request);
    }
    *requests = with;
}

/// Every adjustment running at `position`, bottom lane first.
///
/// The other half of what is on screen at an instant, beside
/// [`layer_requests`]: those say which pictures are drawn, these say how the
/// pictures beneath are graded. **The only place this rule lives** (§46) — the
/// preview and the export both ask here, for the reason `layer_requests` gives.
///
/// A hidden lane grades nothing, and while any adjustment lane is soloed only
/// those do: the same rule every other kind of lane follows (§20a.4).
pub fn adjustments_at(
    sequence: &Sequence,
    position: TimelineTime,
) -> Vec<bettercut_timeline::AdjustmentLook> {
    let soloed = sequence.adjustment_tracks.iter().any(|track| track.solo);
    let picture_soloed = sequence.picture_clip_soloed();
    sequence
        .adjustment_tracks
        .iter()
        .filter(|track| bettercut_timeline::track_plays(track.enabled, track.solo, soloed))
        .filter_map(|track| track.clip_at(position))
        .filter(|clip| sequence.clip_plays(clip.id, true, picture_soloed))
        .map(|clip| clip.look.clamped())
        // One that changes nothing is not worth a pass.
        .filter(|look| !look.is_identity())
        .collect()
}

/// Read every colour lookup table the project names that `tried` has not
/// seen, and hand each to `load` — the compositor's `load_lut`.
///
/// The preview and the export both load tables through this, so they grade
/// with the same tables (§46). A file that cannot be read is logged and marked
/// tried, never an error: a clip naming it draws ungraded, as a clip whose
/// media is missing still has a place on the timeline (§66).
pub fn load_luts(
    project: &bettercut_project_format::Project,
    tried: &mut std::collections::HashSet<bettercut_foundation::LutId>,
    mut load: impl FnMut(bettercut_foundation::LutId, &bettercut_timeline::CubeLut),
) {
    for asset in &project.luts {
        if !tried.insert(asset.id) {
            continue;
        }
        match bettercut_timeline::load_cube_file(&asset.path) {
            Ok(table) => load(asset.id, &table),
            Err(err) => {
                tracing::warn!(%err, "a LUT could not be loaded; clips using it draw ungraded")
            }
        }
    }

    // Curves, each baked into a table of its own over the clip's file LUT.
    // Keyed by what they are, so an unchanged grade is not rebuilt.
    let mut files: std::collections::HashMap<
        bettercut_foundation::LutId,
        Option<bettercut_timeline::CubeLut>,
    > = std::collections::HashMap::new();
    for sequence in &project.sequences {
        for track in &sequence.video_tracks {
            for clip in track.clips() {
                if clip.curves.is_identity() {
                    continue;
                }
                let Some(generated) = clip.effective_lut() else {
                    continue;
                };
                if !tried.insert(generated.lut) {
                    continue;
                }
                let file = clip.lut.map(bettercut_timeline::ClipLut::clamped);
                let under = file.and_then(|file| {
                    files
                        .entry(file.lut)
                        .or_insert_with(|| {
                            project
                                .luts
                                .iter()
                                .find(|asset| asset.id == file.lut)
                                .and_then(|asset| {
                                    bettercut_timeline::load_cube_file(&asset.path).ok()
                                })
                        })
                        .as_ref()
                        .map(|table| (table, file.strength))
                });
                let table = clip.curves.clamped().build_lut(under);
                load(generated.lut, &table);
            }
        }
    }
}

/// The frame number grain is drawn for at `position`: the preview and the
/// export both ask here, so a frame's grain is the same in both (§46), and it
/// changes from one frame to the next as real grain does.
pub fn grain_seed(sequence: &Sequence, position: TimelineTime) -> u32 {
    (position.ticks().max(0) / sequence.ticks_per_frame().max(1)) as u32
}

/// How many of the layers about to be composited an adjustment grades.
///
/// Every picture, and none of the titles: adjustment lanes sit over the video
/// tracks and under the text. Counted from the layers **actually drawn**, given
/// as the lane each came from, and not from the requests — a frame that is not
/// decoded yet, or media that has gone missing, is dropped between the two, and
/// a count taken earlier would slide a title in under the grade.
///
/// The *leading* pictures, rather than all of them: titles are drawn after
/// every picture (§26), and stopping at the first one means no ordering
/// surprise elsewhere can ever put a title beneath a grade.
pub fn graded_beneath(sequence: &Sequence, tracks: impl IntoIterator<Item = TrackId>) -> usize {
    tracks
        .into_iter()
        // Pictures only: a title, or the progress bar on its lane of none,
        // ends the graded run.
        .take_while(|track| sequence.video_track(*track).is_some())
        .count()
}

pub fn resolve_audio_tracks(
    tracks: &[bettercut_timeline::AudioTrack],
    position: TimelineTime,
    duration: TimelineTime,
) -> Vec<AudibleClip> {
    let end = position + duration;
    let mut audible = Vec::new();

    let soloed = tracks.iter().any(|track| track.solo);
    for (lane, track) in tracks.iter().enumerate() {
        if !bettercut_timeline::track_plays(track.enabled, track.solo, soloed) {
            continue; // muted, or not the one being soloed (§8)
        }
        // Each clip's reach past its edges for crossfades, so a clip whose own
        // span has ended but whose fade-out has not is still heard.
        let halves = bettercut_timeline::crossfade_halves(track.clips());
        for (clip, &(lead, tail)) in track.clips().iter().zip(&halves) {
            // A muted clip keeps its place and is simply not heard.
            if clip.muted {
                continue;
            }
            let span_start = TimelineTime::from_ticks(clip.timeline.start.ticks() - lead);
            let span_end = TimelineTime::from_ticks(clip.timeline.end.ticks() + tail);
            if span_end <= position || span_start >= end {
                continue;
            }
            // Where this clip begins inside the requested block.
            let offset = if span_start > position {
                span_start - position
            } else {
                TimelineTime::ZERO
            };
            let from = position.max(span_start);
            // Scaled by the clip's speed, like every other
            // timeline-to-source mapping: a clip at 2× is already
            // twice as far into its material at the same instant.
            let into_clip = from.ticks() - clip.timeline.start.ticks();
            // Forwards, where the span starts reading. Reversed, where it
            // starts reading *backwards from*: the mixer reads the window
            // before this instant and plays it end first.
            let source_start = if clip.reversed {
                MediaTime::from_ticks(clip.source.end.ticks() - clip.speed.scale(into_clip))
            } else {
                MediaTime::from_ticks(clip.source.start.ticks() + clip.speed.scale(into_clip))
            };

            // In output frames, from the frame the clip first contributes: see
            // `Fades` for why that makes a fade independent of block size.
            let frames =
                |ticks: i64| ticks.div_euclid(bettercut_foundation::TICKS_PER_AUDIO_SAMPLE);
            let (fade_in, fade_out) = clip.fitted_fades();
            // Measured from the reach, not the clip, when it crossfades: the
            // ramp spans the whole overlap. A clip's own fade on an edge that
            // crossfades gives way to the crossfade.
            let fades = bettercut_audio::Fades {
                into_clip: frames(from.ticks() - span_start.ticks()),
                remaining: frames(span_end.ticks() - from.ticks()),
                fade_in: if lead > 0 { 0 } else { frames(fade_in) },
                fade_out: if tail > 0 { 0 } else { frames(fade_out) },
                crossfade_in: frames(2 * lead),
                crossfade_out: frames(2 * tail),
                curve: match clip.fade_shape {
                    bettercut_timeline::FadeShape::Smooth => bettercut_audio::FadeCurve::Smooth,
                    bettercut_timeline::FadeShape::Linear => bettercut_audio::FadeCurve::Linear,
                    bettercut_timeline::FadeShape::Fast => bettercut_audio::FadeCurve::Fast,
                    bettercut_timeline::FadeShape::Slow => bettercut_audio::FadeCurve::Slow,
                },
            };

            // §24: a keyframed volume is evaluated at both ends of the block
            // and mixed as the line between them, so a duck ramps smoothly
            // instead of stepping at every block boundary.
            let automation = clip
                .keyframes
                .is_animated(bettercut_timeline::AnimatedParameter::Gain)
                .then(|| {
                    let until = end.min(clip.timeline.end);
                    bettercut_audio::GainRamp {
                        from: clip.gain_at(from),
                        to: clip.gain_at(until),
                        frames: frames(until.ticks() - from.ticks()),
                    }
                });

            // The lane's own line, read at both ends of the block exactly as
            // the clip's is, so a scene riding down does it smoothly rather
            // than stepping at every block boundary.
            let track_automation = (!track.volume.is_empty()).then(|| {
                let until = end.min(clip.timeline.end);
                bettercut_audio::GainRamp {
                    from: track.volume.gain_at(from).unwrap_or(track.gain),
                    to: track.volume.gain_at(until).unwrap_or(track.gain),
                    frames: frames(until.ticks() - from.ticks()),
                }
            });

            // The clip's pan line, read at both ends exactly as its volume is.
            let pan_automation = clip
                .keyframes
                .is_animated(bettercut_timeline::AnimatedParameter::Pan)
                .then(|| {
                    let until = end.min(clip.timeline.end);
                    bettercut_audio::GainRamp {
                        from: clip.pan_at(from),
                        to: clip.pan_at(until),
                        frames: frames(until.ticks() - from.ticks()),
                    }
                });

            audible.push(AudibleClip {
                clip: clip.id,
                lane,
                media: clip.media_id,
                source_start,
                speed: clip.speed,
                reversed: clip.reversed,
                denoise: clip.denoise,
                gate: clip.gate,
                eq: clip.eq.clamped(),
                space: clip.space.clamped(),
                channels: clip.channels,
                stereo_width: clip.stereo_width,
                pitch: clip.pitch,
                keep_pitch: clip.keep_pitch,
                leveller: clip.leveller,
                de_ess: clip.de_ess,
                robot: clip.robot,
                gain: if automation.is_some() { 1.0 } else { clip.gain },
                offset,
                fades,
                automation,
                track_gain: if track_automation.is_some() {
                    1.0
                } else {
                    track.gain
                },
                track_automation,
                track_pan: track.pan,
                track_eq: track.eq.clamped(),
                pan: if pan_automation.is_some() {
                    0.0
                } else {
                    clip.pan
                },
                pan_automation,
            });
        }
    }

    audible
}

/// How a resolved layer should be positioned on the canvas.
///
/// Media is *fitted*: a 640×360 frame fills a 1920×1080 canvas, which is what
/// anyone expects of footage. A title is not — its size is in sequence pixels
/// (§26), so a bitmap 400 pixels wide must cover 400/1920 of the canvas
/// whatever else it says. Fitted instead, the same words in a longer sentence
/// would come out smaller, which is the opposite of a size control.
///
/// One function, called by both the preview and the export, for the §46 reason
/// that runs through this module: the two must not each decide what a title's
/// size means.
pub fn layer_transform(
    request: &LayerRequest,
    frame: &VideoFrame,
    output: bettercut_timeline::Resolution,
) -> Transform {
    match request.source {
        LayerSource::Media(_) => request.look.transform,
        // Drawn at its own size in the corner it was placed in, like a title.
        LayerSource::BurnIn => bettercut_timeline::natural_size_transform(
            request.look.transform,
            frame.width,
            frame.height,
            output.width,
            output.height,
        ),
        // A generated colour has no shape of its own, so it takes the frame's.
        // The uniform fits every layer to the frame by its aspect, which would
        // letterbox a square of white inside a wide frame; undoing that fit is
        // what makes it cover exactly, whatever size the generated frame is.
        LayerSource::Solid { .. } => {
            let (fit_x, fit_y) = bettercut_timeline::fit_scale(
                frame.width.max(1) as f32 / frame.height.max(1) as f32,
                output.width.max(1) as f32 / output.height.max(1) as f32,
            );
            Transform {
                // Times the look's own scale, so a generated layer can be
                // sized — a progress bar is a strip of a full frame.
                scale: bettercut_timeline::Vec2::new(
                    request.look.transform.scale.x / fit_x,
                    request.look.transform.scale.y / fit_y,
                ),
                ..request.look.transform
            }
        }
        LayerSource::Text(_) => bettercut_timeline::natural_size_transform(
            request.look.transform,
            frame.width,
            frame.height,
            output.width,
            output.height,
        ),
    }
}

/// A transition covering some instant, already resolved to its two clips.
struct Cut<'a> {
    outgoing: &'a VideoClip,
    incoming: &'a VideoClip,
    kind: TransitionKind,
    /// 0 at the window's start, 1 at its end.
    progress: f32,
}

/// The transition covering `position`, if any.
///
/// `index` is the clip containing `position`, which is either side of the cut
/// depending on which half of the window we are in — so both its own outgoing
/// transition and its predecessor's have to be considered.
///
/// A transition needs a clip on the other side of the cut, touching it exactly.
/// A gap is not a cut: there is nothing to fade to, and fading to black across
/// a gap would surprise anyone who put the gap there deliberately.
fn transition_at<'a>(
    clips: &'a [VideoClip],
    index: usize,
    position: TimelineTime,
) -> Option<Cut<'a>> {
    let clip = clips.get(index)?;

    // This clip's own transition, when `position` has reached its window.
    if let Some(transition) = clip.transition_out {
        let cut = clip.timeline().end;
        if position >= transition.window(cut).start
            && let Some(next) = clips.get(index + 1)
            && next.timeline().start == cut
        {
            return Some(Cut {
                outgoing: clip,
                incoming: next,
                kind: transition.kind,
                progress: transition.progress(cut, position),
            });
        }
    }

    // Otherwise the previous clip's, still running into this one.
    let previous = clips.get(index.checked_sub(1)?)?;
    let transition = previous.transition_out?;
    let cut = previous.timeline().end;
    if cut != clip.timeline().start || position >= transition.window(cut).end {
        return None;
    }
    Some(Cut {
        outgoing: previous,
        incoming: clip,
        kind: transition.kind,
        progress: transition.progress(cut, position),
    })
}

/// How many copies a smear is made of, including the one at the true position.
///
/// Three, not thirty. Each is a redraw of the same texture — cheap, but not
/// free — and the difference between three and thirty is far smaller than the
/// difference between one and three: the eye reads any trail behind a fast
/// object as blur, and stops counting almost immediately.
pub const SMEAR_SAMPLES: usize = 3;

/// Where a moving layer is drawn, and how solid each copy is.
///
/// §45 calls motion blur expensive, and it is when it is done properly — a
/// velocity buffer and a directional pass over the frame. This is the cheap
/// version the compositor can already do: the same picture drawn at the places
/// it passed through, each fainter than the last, which is what a camera
/// shutter records anyway.
///
/// `now` and `then` are the same layer's transforms one frame apart. A layer
/// that did not move gets one copy at full strength, so a still shot pays
/// nothing for having the setting on.
pub fn smear(now: Transform, then: Transform, opacity: f32) -> Vec<(Transform, f32)> {
    let still = (now.position.x - then.position.x).abs() < f32::EPSILON
        && (now.position.y - then.position.y).abs() < f32::EPSILON
        && (now.rotation_degrees - then.rotation_degrees).abs() < f32::EPSILON
        && (now.scale.x - then.scale.x).abs() < f32::EPSILON
        && (now.scale.y - then.scale.y).abs() < f32::EPSILON;
    if still {
        return vec![(now, opacity)];
    }

    // Each copy carries its share, so the stack adds up to the opacity the
    // layer would have had alone. Otherwise a smeared clip is brighter than an
    // unsmeared one, and turning the effect on would look like turning the
    // exposure up.
    let share = opacity / SMEAR_SAMPLES as f32;
    (0..SMEAR_SAMPLES)
        .map(|index| {
            // 0 is where it was, the last is where it is: the trail is behind
            // the picture, which is the way round a shutter records it.
            let t = index as f32 / (SMEAR_SAMPLES - 1) as f32;
            let lerp = |a: f32, b: f32| a + (b - a) * t;
            let at = Transform {
                position: bettercut_timeline::Vec2::new(
                    lerp(then.position.x, now.position.x),
                    lerp(then.position.y, now.position.y),
                ),
                scale: bettercut_timeline::Vec2::new(
                    lerp(then.scale.x, now.scale.x),
                    lerp(then.scale.y, now.scale.y),
                ),
                rotation_degrees: lerp(then.rotation_degrees, now.rotation_degrees),
                anchor: now.anchor,
                // Not interpolated, because there is nothing between a picture
                // and its reflection: every copy in the trail is mirrored the
                // same way the clip is, or the smear would show the subject
                // facing both ways at once.
                flip_h: now.flip_h,
                flip_v: now.flip_v,
            };
            (at, share)
        })
        .collect()
}

/// Add one clip's layer, scaled by `alpha`.
///
/// `alpha` multiplies the opacity the clip already has rather than replacing
/// it: a clip set to 50% that also crossfades should end up half of half, not
/// back at full.
/// Whether a caller wants the trail a moving clip leaves.
///
/// Three places take the layer they just pushed and rewrite it into something
/// else — a backdrop, a softened half of a dissolve, a shot placed by a moving
/// transition. Each wants *one* layer of a particular shape, and a trail would
/// leave copies behind that none of them rewrote: the shot drawn again where
/// the effect did not put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trail {
    Allow,
    /// One layer, whatever the clip is doing.
    Single,
}

fn push_layer(
    requests: &mut Vec<LayerRequest>,
    project: &Project,
    track: TrackId,
    clip: &VideoClip,
    position: TimelineTime,
    alpha: f32,
) {
    push_layer_with(
        requests,
        project,
        track,
        clip,
        position,
        alpha,
        Trail::Allow,
    );
}

fn push_layer_with(
    requests: &mut Vec<LayerRequest>,
    project: &Project,
    track: TrackId,
    clip: &VideoClip,
    position: TimelineTime,
    alpha: f32,
    trail: Trail,
) {
    if project.media_asset(clip.media_id).is_none() {
        return; // §66: missing media leaves a gap, not a failure
    }
    let source_time = handle_time(clip, position);
    // §24: resolved in the timeline crate, once, so an animated fade exports
    // as the fade the user watched. Against the clip's *progress* rather than
    // the frame it reads, so a frozen clip still animates: the picture is
    // held, the movement over it is not.
    let mut look = clip.look_at(clip.progress_time_at(position));

    // The entrance and the exit, on top of whatever the keyframes just
    // decided — an animation moves the shot *from* where the user put it, not
    // to the middle of the frame. Here rather than in `look_at` because a
    // motion is timed against the clip's place on the timeline, and `look_at`
    // is asked about a source time.
    if !clip.motion.is_none() {
        let (transform, opacity) =
            clip.motion
                .look(look.transform, look.opacity, clip.timeline, position);
        look.transform = transform;
        look.opacity = opacity;
    }

    look.opacity *= alpha;

    // The smear, last, because it needs the look the rest of this function
    // decided — where the shot actually is this frame, keyframes, entrance and
    // transition alpha included.
    let copies = match trail {
        Trail::Allow => smear_for(clip, position, look.transform, look.opacity),
        Trail::Single => vec![(look.transform, look.opacity)],
    };
    let blend = smooth_motion_blend(clip, project, source_time);
    for (transform, share) in copies {
        let (base, over) = match blend {
            Some((base, next, phase)) => (base, Some((next, phase))),
            None => (source_time, None),
        };
        requests.push(LayerRequest {
            clip: clip.id,
            track,
            angle: clip.angle,
            source: LayerSource::Media(clip.media_id),
            source_time: base,
            look: bettercut_timeline::ClipLook {
                corner_pin: Default::default(),
                transform,
                opacity: share,
                ..look
            },
            reveal: None,
        });
        // The next frame laid over it, as far as the playhead is between
        // the two: a slowed shot glides instead of repeating frames.
        if let Some((next, phase)) = over {
            requests.push(LayerRequest {
                clip: clip.id,
                track,
                angle: clip.angle,
                source: LayerSource::Media(clip.media_id),
                source_time: next,
                look: bettercut_timeline::ClipLook {
                    corner_pin: Default::default(),
                    transform,
                    opacity: share * phase,
                    ..look
                },
                reveal: None,
            });
        }
    }
}

/// For a slowed clip with smooth motion on: the source frame at or before
/// `source_time`, the one after it, and how far between them it is (0–1).
/// `None` when there is nothing to blend — normal speed, a hold, a photo, a
/// file with no frame rate, or exactly on a frame.
pub fn smooth_motion_blend(
    clip: &VideoClip,
    project: &Project,
    source_time: MediaTime,
) -> Option<(MediaTime, MediaTime, f32)> {
    if !clip.smooth_motion || clip.frozen {
        return None;
    }
    // Only slower than normal: a clip at or above full speed has a new frame
    // every output frame anyway.
    if clip.speed.num() >= clip.speed.den() {
        return None;
    }
    let rate = project.media_asset(clip.media_id)?.frame_rate?;
    let frame = bettercut_foundation::ticks_per_frame(rate).filter(|t| *t > 0)?;
    let ticks = source_time.ticks();
    let base = ticks.div_euclid(frame) * frame;
    let phase = (ticks - base) as f32 / frame as f32;
    if phase <= 0.001 {
        return None;
    }
    // Backwards, the next frame shown is the earlier one.
    let (base, next, phase) = if clip.reversed {
        (base + frame, base, 1.0 - phase)
    } else {
        (base, base + frame, phase)
    };
    Some((
        MediaTime::from_ticks(base),
        MediaTime::from_ticks(next),
        phase,
    ))
}

/// The copies a clip is drawn as this frame: one, unless it is smearing.
///
/// The earlier transform comes from asking the same functions about the
/// previous frame, so a shot moving for *any* reason smears — an entrance, a
/// keyframed move, a slow zoom. Nothing here knows which.
fn smear_for(
    clip: &VideoClip,
    position: TimelineTime,
    now: Transform,
    opacity: f32,
) -> Vec<(Transform, f32)> {
    if !clip.motion_blur {
        return vec![(now, opacity)];
    }

    // A frame earlier, clamped into the clip: at its very first frame there is
    // no "before", and reading outside would smear from a position the shot
    // never had.
    let step = TimelineTime::from_ticks(bettercut_foundation::TICKS_PER_SECOND / 60);
    let earlier = TimelineTime::from_ticks(
        (position.ticks() - step.ticks()).max(clip.timeline.start.ticks()),
    );

    let mut then = clip.look_at(clip.progress_time_at(earlier));
    if !clip.motion.is_none() {
        let (transform, _) = clip
            .motion
            .look(then.transform, then.opacity, clip.timeline, earlier);
        then.transform = transform;
    }
    smear(now, then.transform, opacity)
}

/// The blurred copy of a clip that fills the frame behind it (§36).
///
/// Only when the clip does not cover the frame by itself: a backdrop behind a
/// picture that already fills it is a second decode and a second draw of
/// something nobody can see.
///
/// Deliberately *not* drawn during a transition. Both clips are moving then,
/// and a backdrop would be revealed at the edges as they slide — which is a
/// picture of the bug, not of the shot.
fn push_backdrop(
    requests: &mut Vec<LayerRequest>,
    project: &Project,
    sequence: &Sequence,
    track: TrackId,
    clip: &VideoClip,
    position: TimelineTime,
) {
    let picture = match clip.backdrop {
        bettercut_timeline::Backdrop::None => return,
        bettercut_timeline::Backdrop::Blur => None,
        bettercut_timeline::Backdrop::Image(media) => Some(media),
    };
    let Some(asset) = project.media_asset(clip.media_id) else {
        return; // §66: missing media leaves a gap, not a backdrop
    };
    let aspect = |w: u32, h: u32| (w > 0 && h > 0).then(|| w as f32 / h as f32);
    let (Some(source), Some(output)) = (
        aspect(asset.width, asset.height),
        aspect(sequence.resolution.width, sequence.resolution.height),
    ) else {
        return;
    };

    let cover = bettercut_timeline::fill_scale(source, output);
    // Within a whisker of covering already — a 16:9 clip in a 16:9 sequence —
    // so there is nothing to fill.
    if cover <= 1.001 {
        return;
    }
    // A chosen picture covers the frame at its own shape, not the shot's.
    let picture = match picture {
        None => None,
        Some(media) => {
            let Some(image) = project.media_asset(media) else {
                return; // removed or missing: bars, as with no backdrop
            };
            let Some(shape) = aspect(image.width, image.height) else {
                return;
            };
            Some((media, bettercut_timeline::fill_scale(shape, output)))
        }
    };

    let before = requests.len();
    push_layer_with(requests, project, track, clip, position, 1.0, Trail::Single);
    let Some(request) = requests.get_mut(before) else {
        return;
    };
    // The clip's own framing is *not* carried: a shot the user has pushed to
    // one side should still have a full frame behind it, not a backdrop pushed
    // the same way.
    //
    // Its animation is, though, and the distinction is the point: framing is
    // where the user put the shot, an animation is the shot arriving. A slide
    // that left the backdrop behind would look like the picture sliding across
    // a blurred frame that was already there, which is not the effect anyone
    // asked for. Taken from the motion applied to nothing, so what is added
    // here is the movement alone rather than the movement and the framing.
    let (moved, _) = clip.motion.look(
        bettercut_timeline::Transform::default(),
        1.0,
        clip.timeline,
        position,
    );
    let cover = cover * bettercut_timeline::BACKDROP_OVERSCAN;
    request.look.transform = bettercut_timeline::Transform {
        scale: bettercut_timeline::Vec2::new(cover * moved.scale.x, cover * moved.scale.y),
        position: moved.position,
        rotation_degrees: moved.rotation_degrees,
        ..bettercut_timeline::Transform::default()
    };
    if let Some((media, picture_cover)) = picture {
        // The picture as it is: none of the shot's grade, crop or effects,
        // which belong to the shot. Only its opacity and movement carry over,
        // for the reasons given for the blur.
        let neutral = VideoClip::new(media, clip.timeline.start, clip.source)
            .map(|fresh| fresh.look_at(MediaTime::ZERO));
        if let Ok(mut look) = neutral {
            look.opacity = request.look.opacity;
            look.transform = bettercut_timeline::Transform {
                scale: bettercut_timeline::Vec2::new(
                    picture_cover * moved.scale.x,
                    picture_cover * moved.scale.y,
                ),
                ..request.look.transform
            };
            request.look = look;
            request.source = LayerSource::Media(media);
            request.source_time = MediaTime::ZERO;
            request.angle = None;
            return;
        }
    }
    request.look.blur = bettercut_timeline::BACKDROP_BLUR;
    // A fill behind the shot, not a second framed copy of it.
    request.look.border = bettercut_timeline::Border::NONE;
    request.look.shadow = bettercut_timeline::Shadow::NONE;
    // Opacity is left as `push_layer` computed it — the clip's own, keyframes
    // and animation included. This used to be forced to 1, on the grounds that
    // a half-transparent backdrop lets the black through and the frame stops
    // being full. That is the wrong trade: it made a clip faded to *nothing*
    // still cover the whole frame with a blurred copy of itself, hiding every
    // track beneath it, and it made an entrance animation open on a solid
    // background with no shot on top of it.
}

/// Where one clip sits during a moving transition, and how solid it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerMove {
    /// Offset from centre in frame widths: 1.0 is one whole frame to the right.
    pub offset_x: f32,
    /// Multiplied into the clip's own scale.
    pub scale: f32,
    /// Multiplied into the clip's own opacity.
    pub alpha: f32,
    /// Degrees added to the clip's own rotation.
    pub rotation: f32,
}

impl LayerMove {
    const STILL: Self = Self {
        offset_x: 0.0,
        scale: 1.0,
        alpha: 1.0,
        rotation: 0.0,
    };
}

/// How broken up the picture is at `progress` through a glitch transition, on
/// the glitch and RGB split scale (0–100), and how far it jumps sideways, in
/// frame widths.
///
/// Clean at either end and wrecked at the cut. The jump changes a dozen times
/// a second rather than every frame — a shake at the frame rate reads as noise,
/// a few lurches read as a signal dropping out — and it is worked out from the
/// progress alone, so preview and export lurch on the same frames (§46).
pub fn transition_glitch(progress: f32) -> (f32, f32) {
    let t = progress.clamp(0.0, 1.0);
    let strength = (1.0 - (2.0 * t - 1.0).abs()).sqrt();
    let step = (t * 12.0).floor() as u32;
    let hash = step.wrapping_mul(2_654_435_761) >> 16;
    let side = (hash % 1000) as f32 / 1000.0 * 2.0 - 1.0;
    (
        bettercut_timeline::MAX_GLITCH * strength,
        side * 0.03 * strength,
    )
}

/// The outgoing and incoming placements for a moving transition (§25).
///
/// `progress` is 0 at the window's start and 1 at its end. Written as one
/// function so the rule can be read — and tested — without a GPU.
pub fn moving_transition(kind: TransitionKind, progress: f32) -> (LayerMove, LayerMove) {
    let t = progress.clamp(0.0, 1.0);
    match kind {
        // The next shot slides in over a stationary one. The outgoing clip is
        // not moved and not faded: it is being *covered*, which is what makes
        // this read differently from a crossfade.
        TransitionKind::Slide => (
            LayerMove::STILL,
            LayerMove {
                offset_x: 1.0 - t,
                ..LayerMove::STILL
            },
        ),
        // Both move together, as if the two shots were on one strip being
        // pulled across: whatever the incoming clip has not covered, the
        // outgoing one has already vacated.
        TransitionKind::Push => (
            LayerMove {
                offset_x: -t,
                ..LayerMove::STILL
            },
            LayerMove {
                offset_x: 1.0 - t,
                ..LayerMove::STILL
            },
        ),
        // The outgoing shot swells and fades; the incoming one is simply
        // behind it, at rest. Scaling the incoming clip *up* from small would
        // show the frame's edges around it for the first half of the window.
        TransitionKind::Zoom => (
            LayerMove {
                offset_x: 0.0,
                scale: 1.0 + 0.35 * t,
                alpha: 1.0 - t,
                rotation: 0.0,
            },
            LayerMove::STILL,
        ),
        // The outgoing shot turns half a turn as it shrinks into the middle;
        // at the cut the incoming one is exactly there — the same size, the
        // same angle — and unwinds back out to fill the frame. Eased, so the
        // spin is fastest where the two change over.
        TransitionKind::Spin => {
            if t < 0.5 {
                let s = t / 0.5;
                (
                    LayerMove {
                        scale: 1.0 - 0.6 * s,
                        rotation: 180.0 * s * s,
                        ..LayerMove::STILL
                    },
                    LayerMove {
                        alpha: 0.0,
                        ..LayerMove::STILL
                    },
                )
            } else {
                let u = 1.0 - (t - 0.5) / 0.5;
                (
                    LayerMove {
                        alpha: 0.0,
                        ..LayerMove::STILL
                    },
                    LayerMove {
                        scale: 1.0 - 0.6 * u,
                        rotation: -180.0 * u * u,
                        ..LayerMove::STILL
                    },
                )
            }
        }
        // A crossfade is the incoming clip coming up over the outgoing one, and
        // a fade through black never has two layers at once; neither is a
        // moving transition, and both are handled before this is called.
        TransitionKind::Crossfade => (
            LayerMove::STILL,
            LayerMove {
                alpha: t,
                ..LayerMove::STILL
            },
        ),
        // Neither moves anything: a flash is a layer laid over the top, and a
        // blur dissolve is a crossfade with the sharpness taken out of both.
        // A wipe and an iris move nothing either: the incoming shot is cut to
        // a shape that grows (`transition_mask`).
        TransitionKind::FadeThroughBlack
        | TransitionKind::Flash
        | TransitionKind::Blur
        | TransitionKind::Wipe
        | TransitionKind::Iris
        | TransitionKind::Glitch => (LayerMove::STILL, LayerMove::STILL),
    }
}

/// The shape the incoming shot is cut to at `progress` through a wipe or an
/// iris: nothing at the start, the whole frame by the end. `None` for every
/// other kind.
///
/// Built from the clip mask the compositor already draws, so a wipe is a
/// moving mask rather than a shader of its own (§46 for free). The soft edge
/// starts and ends just outside the frame, so neither end shows a sliver.
pub fn transition_mask(kind: TransitionKind, progress: f32) -> Option<bettercut_timeline::Mask> {
    use bettercut_timeline::{Mask, MaskShape};
    const FEATHER: f32 = 0.03;
    let t = progress.clamp(0.0, 1.0);
    match kind {
        // A vertical edge moving left to right, keeping what is left of it.
        // A linear mask keeps the side above its edge; turned a quarter the
        // other way, "above" is "to the left".
        TransitionKind::Wipe => Some(Mask {
            shape: MaskShape::Linear,
            center: [-FEATHER + t * (1.0 + 2.0 * FEATHER), 0.5],
            size: [0.5, 0.5],
            feather: FEATHER,
            rotation_degrees: -90.0,
            invert: false,
        }),
        // A circle from nothing to past the corners, which sit about 0.71
        // from the middle.
        TransitionKind::Iris => {
            let radius = t * (0.72 + FEATHER);
            Some(Mask {
                shape: MaskShape::Ellipse,
                center: [0.5, 0.5],
                size: [radius, radius],
                feather: FEATHER,
                rotation_degrees: 0.0,
                invert: false,
            })
        }
        _ => None,
    }
}

/// How soft both shots are at `progress` through a blur dissolve.
///
/// Sharp at either end and softest at the cut, so the change-over lands where
/// there is least detail for the eye to catch it happening — which is the whole
/// trick of the effect.
pub fn transition_blur(progress: f32) -> f32 {
    MAX_TRANSITION_BLUR * (1.0 - (2.0 * progress.clamp(0.0, 1.0) - 1.0).abs())
}

/// How far a blur dissolve softens, on [`bettercut_timeline::MAX_BLUR`]'s
/// 0–100 scale.
///
/// Well short of the maximum. The point is to lose the detail that would make
/// the cut visible, not to turn the frame into fog — and a dissolve that went
/// all the way would read as a fault rather than as an effect.
pub const MAX_TRANSITION_BLUR: f32 = 55.0;

/// Push one side of a blur dissolve.
fn push_blurred(
    requests: &mut Vec<LayerRequest>,
    project: &Project,
    track: TrackId,
    clip: &VideoClip,
    position: TimelineTime,
    alpha: f32,
    softness: f32,
) {
    let before = requests.len();
    push_layer_with(
        requests,
        project,
        track,
        clip,
        position,
        alpha,
        Trail::Single,
    );
    let Some(request) = requests.get_mut(before) else {
        return; // §66: missing media pushed nothing to soften
    };
    // The greater of the two rather than the sum: a clip the user has already
    // softened is not sharpened by a dissolve, and adding them could ask for
    // more blur than the scale has.
    request.look.blur = request.look.blur.max(softness);
}

/// How solid the white is at `progress` through a flash.
///
/// Nothing at either end and full at the cut, so the shot is untouched until
/// the flash begins and again as soon as it is over — the complement of the
/// fade through black, which is at full picture where this is at full white.
pub fn flash_alpha(progress: f32) -> f32 {
    1.0 - (2.0 * progress.clamp(0.0, 1.0) - 1.0).abs()
}

/// White over the cut, for §25's flash.
///
/// Its own layer rather than a brightness pushed up on the clip: brightness is
/// an exposure, which is a multiply, and multiplying black by anything at all
/// leaves it black. A night shot would flash to grey and the shadows would
/// never leave.
fn push_flash(requests: &mut Vec<LayerRequest>, track: TrackId, clip: ClipId, alpha: f32) {
    requests.push(LayerRequest {
        clip,
        track,
        source: LayerSource::Solid {
            rgb: [255, 255, 255],
        },
        source_time: MediaTime::ZERO,
        // Written out rather than defaulted: `ClipLook` has no `Default`
        // precisely because an opacity of zero is not a sensible one, and the
        // colour of a generated layer is not up for adjustment anyway.
        look: bettercut_timeline::ClipLook {
            corner_pin: Default::default(),
            old_film: 0.0,
            glow: 0.0,
            shadow: Default::default(),
            border: Default::default(),
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            pixelate: 0.0,
            zoom_blur: 0.0,
            lens: 0.0,
            tilt_band: 0.0,
            tilt_centre: 0.5,
            posterise: 0.0,
            smooth_skin: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform::default(),
            opacity: alpha,
            color: bettercut_timeline::ColorAdjust::IDENTITY,
            blur: 0.0,
            chroma_key: None,
            luma_key: None,
            mask: None,
            blend: bettercut_timeline::BlendMode::Normal,
        },
        reveal: None,
        angle: None,
    });
}

/// Where a light leak's glow is, how big, and how strong, `seconds` into a
/// clip with `amount` (0–100): `(centre, half size, opacity)` in frame units.
/// `None` for no leak.
///
/// It drifts and breathes on slow, unrelated cycles, so it never visibly
/// repeats in a short clip — and it is a function of time alone, so the
/// preview and the export draw the same glow on the same frame (§46).
pub fn light_leak_at(amount: f32, seconds: f64) -> Option<([f32; 2], [f32; 2], f32)> {
    if !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    let share = (amount / 100.0).min(1.0);
    let t = seconds as f32;
    let centre = [
        0.5 + 0.35 * (t * 0.43).sin(),
        0.4 + 0.2 * (t * 0.31 + 1.0).sin(),
    ];
    let size = [0.45 + 0.1 * (t * 0.23).sin(), 0.6];
    let breathing = 0.6 + 0.4 * (t * 0.9 + 2.0).sin().abs();
    Some((centre, size, share * breathing))
}

/// How long a beat pulse takes to ease back after its marker.
pub const BEAT_PULSE_SECONDS: f64 = 0.3;

/// The furthest a beat pulse punches in, at 100: a fifth bigger.
pub const MAX_BEAT_PULSE: f32 = 0.2;

/// The scale a beat pulse gives a picture at `position`: largest on a marker,
/// easing back to 1 over [`BEAT_PULSE_SECONDS`] after it. 1 away from every
/// marker or with no amount.
pub fn beat_pulse_at(
    amount: f32,
    markers: &[bettercut_timeline::Marker],
    position: TimelineTime,
) -> f32 {
    if !amount.is_finite() || amount <= 0.0 {
        return 1.0;
    }
    let window = (BEAT_PULSE_SECONDS * bettercut_foundation::TICKS_PER_SECOND as f64) as i64;
    let since = markers
        .iter()
        .map(|m| position.ticks() - m.time.ticks())
        .filter(|since| (0..window).contains(since))
        .min();
    let Some(since) = since else {
        return 1.0;
    };
    let left = 1.0 - since as f32 / window as f32;
    1.0 + (amount / 100.0).min(1.0) * MAX_BEAT_PULSE * left * left
}

/// The warm colour a light leak glows in.
pub const LIGHT_LEAK_RGB: [u8; 3] = [255, 146, 58];

/// A light leak over `clip`: a warm solid, cut to a very soft oval where the
/// glow is, screened over the shot so it only ever brightens.
fn push_light_leak(
    requests: &mut Vec<LayerRequest>,
    track: TrackId,
    clip: &VideoClip,
    position: TimelineTime,
) {
    let seconds = (position.ticks() - clip.timeline.start.ticks()) as f64
        / bettercut_foundation::TICKS_PER_SECOND as f64;
    let Some((centre, size, opacity)) = light_leak_at(clip.light_leak, seconds) else {
        return;
    };
    requests.push(LayerRequest {
        clip: clip.id,
        track,
        source: LayerSource::Solid {
            rgb: LIGHT_LEAK_RGB,
        },
        source_time: MediaTime::ZERO,
        look: bettercut_timeline::ClipLook {
            corner_pin: Default::default(),
            old_film: 0.0,
            glow: 0.0,
            shadow: Default::default(),
            border: Default::default(),
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            pixelate: 0.0,
            zoom_blur: 0.0,
            lens: 0.0,
            tilt_band: 0.0,
            tilt_centre: 0.5,
            posterise: 0.0,
            smooth_skin: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform::default(),
            opacity,
            color: bettercut_timeline::ColorAdjust::IDENTITY,
            blur: 0.0,
            chroma_key: None,
            luma_key: None,
            mask: Some(bettercut_timeline::Mask {
                shape: bettercut_timeline::MaskShape::Ellipse,
                center: centre,
                size,
                feather: 1.0,
                rotation_degrees: 25.0,
                invert: false,
            }),
            blend: bettercut_timeline::BlendMode::Screen,
        },
        reveal: None,
        angle: None,
    });
}

/// One piece of a lens flare: its colour, centre and half size in frame
/// units, strength, and how far it is turned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlarePiece {
    pub rgb: [u8; 3],
    pub centre: [f32; 2],
    pub size: [f32; 2],
    pub opacity: f32,
    pub rotation_degrees: f32,
}

/// A lens flare `seconds` into a clip with `amount` (0–100), as the ovals it
/// is drawn from, back to front. Empty for no flare.
///
/// The source sits near the top left and drifts on slow cycles; the ghosts
/// lie on the line from it through the centre of the frame, as the
/// reflections inside a real lens do, so they swing across as it moves. A
/// function of time alone, so preview and export agree (§46).
pub fn lens_flare_at(amount: f32, seconds: f64) -> Vec<FlarePiece> {
    if !amount.is_finite() || amount <= 0.0 {
        return Vec::new();
    }
    let share = (amount / 100.0).min(1.0);
    let t = seconds as f32;
    let source = [
        0.24 + 0.08 * (t * 0.21).sin(),
        0.2 + 0.05 * (t * 0.17 + 1.3).sin(),
    ];
    // Along the line through the centre: 0 at the source, 1 at its mirror.
    let along = |k: f32| {
        [
            source[0] + (1.0 - 2.0 * source[0]) * k,
            source[1] + (1.0 - 2.0 * source[1]) * k,
        ]
    };
    let twinkle = 0.85 + 0.15 * (t * 2.3).sin().abs();
    let mut pieces = vec![
        // The wide warm halo, then the streak through the source, then the
        // hot core on top.
        FlarePiece {
            rgb: [255, 196, 120],
            centre: source,
            size: [0.22, 0.36],
            opacity: 0.3 * share,
            rotation_degrees: 0.0,
        },
        FlarePiece {
            rgb: [150, 200, 255],
            centre: source,
            size: [0.6, 0.012],
            opacity: 0.7 * share * twinkle,
            rotation_degrees: 0.0,
        },
        FlarePiece {
            rgb: [255, 250, 235],
            centre: source,
            size: [0.045, 0.08],
            opacity: 0.9 * share * twinkle,
            rotation_degrees: 0.0,
        },
    ];
    for (k, radius, rgb, strength) in [
        (0.55, 0.03, [120, 255, 170], 0.22),
        (0.8, 0.06, [140, 170, 255], 0.16),
        (1.15, 0.1, [255, 150, 200], 0.12),
    ] {
        pieces.push(FlarePiece {
            rgb,
            centre: along(k),
            size: [radius, radius * 16.0 / 9.0],
            opacity: strength * share,
            rotation_degrees: 0.0,
        });
    }
    pieces
}

/// A lens flare over `clip`: each piece a warm or tinted solid cut to a soft
/// oval, screened so it only ever brightens.
fn push_lens_flare(
    requests: &mut Vec<LayerRequest>,
    track: TrackId,
    clip: &VideoClip,
    position: TimelineTime,
) {
    let seconds = (position.ticks() - clip.timeline.start.ticks()) as f64
        / bettercut_foundation::TICKS_PER_SECOND as f64;
    for piece in lens_flare_at(clip.lens_flare, seconds) {
        requests.push(LayerRequest {
            clip: clip.id,
            track,
            source: LayerSource::Solid { rgb: piece.rgb },
            source_time: MediaTime::ZERO,
            look: bettercut_timeline::ClipLook {
                corner_pin: Default::default(),
                old_film: 0.0,
                glow: 0.0,
                shadow: Default::default(),
                border: Default::default(),
                sharpen: 0.0,
                lut: None,
                rgb_split: 0.0,
                glitch: 0.0,
                pixelate: 0.0,
                zoom_blur: 0.0,
                lens: 0.0,
                tilt_band: 0.0,
                tilt_centre: 0.5,
                posterise: 0.0,
                smooth_skin: 0.0,
                vignette: 0.0,
                reflection: bettercut_timeline::Reflection::None,
                crop: bettercut_timeline::Crop::NONE,
                transform: Transform::default(),
                opacity: piece.opacity,
                color: bettercut_timeline::ColorAdjust::IDENTITY,
                blur: 0.0,
                chroma_key: None,
                luma_key: None,
                mask: Some(bettercut_timeline::Mask {
                    shape: bettercut_timeline::MaskShape::Ellipse,
                    center: piece.centre,
                    size: piece.size,
                    // Soft all the way in: a flare has no edge. An ellipse's
                    // feather is measured in its own radii, so 1.2 fades from
                    // under half the radius out to past its edge.
                    feather: 1.2,
                    rotation_degrees: piece.rotation_degrees,
                    invert: false,
                }),
                blend: bettercut_timeline::BlendMode::Screen,
            },
            reveal: None,
            angle: None,
        });
    }
}

/// The frame a [`LayerSource::Solid`] draws.
///
/// One pixel: the uniform stretches every layer to the frame, so a larger
/// buffer would be the same picture at a hundred thousand times the cost.
/// Shared by the preview and the export (§46) — a flash that was white in one
/// and grey in the other is exactly the failure that rule exists to stop.
pub fn solid_frame(rgb: [u8; 3]) -> VideoFrame {
    VideoFrame {
        timestamp: MediaTime::ZERO,
        width: 1,
        height: 1,
        color: bettercut_media::ColorMetadata::default(),
        storage: bettercut_media::FrameStorage::System {
            data: vec![rgb[0], rgb[1], rgb[2], 255],
            stride: 4,
        },
    }
}

/// Push one layer of a moving transition, placed by `movement`.
fn push_moving(
    requests: &mut Vec<LayerRequest>,
    project: &Project,
    track: TrackId,
    clip: &VideoClip,
    position: TimelineTime,
    movement: LayerMove,
) {
    let before = requests.len();
    push_layer_with(
        requests,
        project,
        track,
        clip,
        position,
        movement.alpha,
        Trail::Single,
    );
    // §66: a missing file pushes nothing, and there is then nothing to place.
    let Some(request) = requests.get_mut(before) else {
        return;
    };
    // Applied *over* whatever the clip already has, so a transition on a clip
    // the user has moved or scaled shifts it from where they put it.
    request.look.transform.position.x += movement.offset_x;
    request.look.transform.scale.x *= movement.scale;
    request.look.transform.scale.y *= movement.scale;
    request.look.transform.rotation_degrees += movement.rotation;
}

/// Where in the source a clip is reading at `position`, *including* outside its
/// own range.
///
/// [`source_time_of`] clamps a position before the clip to its in-point, which
/// is right for the ordinary case and exactly wrong during a crossfade: reading
/// the handle is the whole point. Only the start of the file clamps here,
/// because there is genuinely nothing before it.
fn handle_time(clip: &VideoClip, position: TimelineTime) -> MediaTime {
    // A freeze holds one instant, inside its own range and out: reading a
    // handle would show the frames either side of it during a transition,
    // which is not a freeze.
    if clip.frozen {
        return clip.source().start;
    }
    let into_clip = position.ticks() - clip.timeline().start.ticks();
    // Scaled by the clip's speed, like every other timeline-to-source
    // mapping: during a crossfade a 2× clip reads its handle twice as fast as
    // the window advances. Backwards on a reversed clip, through the one
    // mapping every lookup shares — a handle past a reversed clip's end is
    // the material *before* its source range, which is what continues the
    // reversed motion.
    let at = bettercut_timeline::source_time(clip.source(), clip.speed, clip.reversed, into_clip);
    MediaTime::from_ticks(at.ticks().max(0))
}

/// One audio clip audible at a given instant.
#[derive(Debug, Clone, Copy)]
pub struct AudibleClip {
    pub clip: ClipId,
    /// Which sound lane it came from, counted from the top as the timeline
    /// draws them. For the per-lane meters: everything else about mixing is
    /// the clip's own business, but a meter is a *lane's* reading.
    pub lane: usize,
    pub media: MediaId,
    /// Where in the source the audible span starts.
    pub source_start: MediaTime,
    /// the clip's playback rate. The mixer reads this many source frames per output
    /// frame and resamples them down.
    pub speed: bettercut_foundation::Rational,
    /// Played backwards: [`Self::source_start`] is then where the span ends in
    /// the source, and the mixer reads the window before it, end first.
    pub reversed: bool,
    /// Voice clean-up, 0–100. Carried from block to block by the mixer, so a
    /// clip's clean-up is one continuous process however the audio is cut up.
    pub denoise: f32,
    /// The noise gate, 0–100. Carried from block to block by the mixer.
    pub gate: f32,
    /// The clip's equaliser, already held to its ranges. Carried from block to
    /// block by the mixer, as the clean-up is.
    pub eq: bettercut_timeline::ClipEq,
    /// The clip's echo or reverb, already held to its range. Carried from
    /// block to block by the mixer, as the equaliser is.
    pub space: bettercut_timeline::ClipSpace,
    /// Which channels play where, applied as soon as the sound is read.
    pub channels: bettercut_timeline::ChannelMode,
    /// How wide the stereo image is, 0–2, applied right after the channels.
    pub stereo_width: f32,
    /// The voice changer's shift in semitones; zero is none. Carried from
    /// block to block by the mixer.
    pub pitch: f32,
    /// Hold the pitch where it was through a speed change: the mixer shifts it
    /// back by as much as the resampling moved it.
    pub keep_pitch: bool,
    /// The leveller, 0–100. Carried from block to block by the mixer.
    pub leveller: f32,
    /// The de-esser, 0–100. Carried from block to block by the mixer.
    pub de_ess: f32,
    /// The robot voice, 0–100. Needs no state: its phase is the timeline's.
    pub robot: f32,
    pub gain: f32,
    /// Offset from the start of the requested block, in timeline ticks.
    pub offset: TimelineTime,
    /// The clip's fades, measured from the first frame it contributes to the
    /// block.
    pub fades: bettercut_audio::Fades,
    /// §24's volume envelope across this block, where the clip has one. When
    /// it is here, [`Self::gain`] is 1.0: a keyframed value replaces the static
    /// one rather than scaling it.
    pub automation: Option<bettercut_audio::GainRamp>,
    /// §20a.4's track stage, from the track the clip is on.
    ///
    /// 1.0 when [`Self::track_automation`] is there, for the same reason
    /// [`Self::gain`] is: a line replaces the static level rather than scaling
    /// it.
    pub track_gain: f32,
    /// The track's volume line across this block, where the lane has one.
    pub track_automation: Option<bettercut_audio::GainRamp>,
    pub track_pan: f32,
    /// The lane's equaliser, already held to its ranges, run after the clip's
    /// own. Carried from block to block by the mixer, per clip.
    pub track_eq: bettercut_timeline::ClipEq,
    /// The clip's own static pan, -1 to +1; `0.0` while a pan line is riding.
    pub pan: f32,
    /// The clip's keyframed pan across this block, where it has one, read at
    /// both ends as the volume is.
    pub pan_automation: Option<bettercut_audio::GainRamp>,
}

/// Where preview frames are read from (§14).
///
/// > Editing uses proxy media. Export uses original media.
///
/// Held as runtime state rather than in the project, because whether a proxy
/// exists is a fact about *this machine's* cache. Storing it in the `.vproj`
/// would mean a project opened on another computer claims proxies that are not
/// there — and §66's missing-media handling would have to cover cache files
/// too, for no benefit.
#[derive(Clone)]
pub struct ProxySource {
    pub cache: std::sync::Arc<bettercut_cache::CacheStore>,
    pub height: u32,
}

/// Two sources are the same when they name the same cache and height.
///
/// Compared by pointer rather than by contents: a `CacheStore` has no
/// meaningful value equality, and the question being asked is "is this the same
/// source I already configured?", which pointer identity answers exactly.
impl PartialEq for ProxySource {
    fn eq(&self, other: &Self) -> bool {
        self.height == other.height && std::sync::Arc::ptr_eq(&self.cache, &other.cache)
    }
}

impl ProxySource {
    /// The proxy for `media`, if the cache actually holds one.
    pub(crate) fn path_for(&self, media: MediaId) -> Option<std::path::PathBuf> {
        let path = self.cache.layout().proxy_file(media, self.height);
        path.exists().then_some(path)
    }
}

pub struct PlaybackEngine {
    /// The decoder pool. The *same* type the decode-ahead thread runs, which
    /// is the point: this used to be a second, hand-rolled copy of it here,
    /// and the copy is what let §47a.2's sequential-read rule be implemented
    /// in one of the two places and missed in the other.
    frames: FrameSource,
    /// Kept only to size the decode-ahead budget; the pool owns the rest.
    proxy: Option<ProxySource>,
    cache: FrameCache,
    /// FFmpeg threads per decoder (§15.1), from `HardwareProfile`.
    decoder_threads: u32,

    /// The decode-ahead thread (§47a.3). `None` until playback starts, because
    /// a paused editor has nothing to decode ahead of.
    prefetcher: Option<crate::prefetcher::Prefetcher>,
    /// Frames served from the ring rather than decoded inline — the number
    /// that says whether decode-ahead is doing anything.
    prefetch_hits: u64,

    /// §26.1's rasterizer. The export builds its own; what is shared is the
    /// *implementation*, which is what makes the title on screen the title in
    /// the file.
    text: crate::text_frames::TextFrames,
}

impl PlaybackEngine {
    pub fn new(cache_bytes: usize, decoder_threads: u32) -> Self {
        Self {
            frames: FrameSource::new(decoder_threads),
            proxy: None,
            cache: FrameCache::new(cache_bytes),
            decoder_threads: decoder_threads.max(1),
            prefetcher: None,
            prefetch_hits: 0,
            text: crate::text_frames::TextFrames::new(),
        }
    }

    /// Start decoding ahead of the playhead (§47a.3).
    ///
    /// Called when playback starts. Idempotent — starting twice would run two
    /// decode threads over the same files, which §74 forbids and which would be
    /// slower than one.
    pub fn start_prefetch(&mut self, budget_bytes: usize) {
        if self.prefetcher.is_some() {
            return;
        }
        self.prefetcher = Some(crate::prefetcher::Prefetcher::start(
            budget_bytes,
            self.decoder_threads,
        ));
    }

    /// Stop decoding ahead. Called when playback stops: a paused editor should
    /// not hold a decode thread or its share of the frame budget (§81).
    pub fn stop_prefetch(&mut self) {
        self.prefetcher = None;
    }

    pub fn prefetch_hits(&self) -> u64 {
        self.prefetch_hits
    }

    /// Frames currently waiting in the ring.
    pub fn prefetched_frames(&self) -> usize {
        self.prefetcher.as_ref().map_or(0, |p| p.buffer().len())
    }

    /// Abandon decode-ahead work and start a new generation (§47a.5).
    ///
    /// Called on any seek: the frames queued for the old position are worthless
    /// and the bytes they hold are needed for the new one.
    pub fn reset_prefetch(&mut self) {
        if let Some(prefetcher) = self.prefetcher.as_ref() {
            prefetcher.reset();
        }
    }

    /// Queue the next `span` of frames for the decode thread (§47a.3).
    ///
    /// Cheap: it walks the visible tracks at frame intervals and resolves which
    /// media each instant needs. No decoding happens here — that is the whole
    /// point — so this is safe to call every UI frame while playing.
    pub fn prefetch_ahead(
        &mut self,
        project: &Project,
        sequence: &Sequence,
        from: TimelineTime,
        span: TimelineTime,
    ) {
        let Some(prefetcher) = self.prefetcher.as_ref() else {
            return;
        };

        let interval = sequence.ticks_per_frame().max(1);
        let generation = prefetcher.generation();
        let mut items = Vec::new();

        // The same rule the composite uses, so the ring reads ahead on the
        // tracks that are actually going to be drawn.
        let soloed = sequence.video_tracks.iter().any(|track| track.solo);
        let picture_soloed = sequence.picture_clip_soloed();
        let mut tick = from.ticks();
        let end = from.ticks().saturating_add(span.ticks());
        while tick < end {
            let at = TimelineTime::from_ticks(tick);
            for track in &sequence.video_tracks {
                if !bettercut_timeline::track_plays(track.enabled, track.solo, soloed) {
                    continue;
                }
                let Some(clip) = track.clip_at(at) else {
                    continue;
                };
                if !sequence.clip_plays(clip.id, true, picture_soloed) {
                    continue;
                }
                let Some(asset) = project.media_asset(clip.media_id) else {
                    continue;
                };
                // Nothing to decode ahead for a made picture.
                if asset.generated.is_some() {
                    continue;
                }
                let source = asset.frame_time(if clip.frozen {
                    clip.source().start
                } else {
                    source_time_of(clip.timeline().start, clip.source().start, at)
                });
                let key = FrameKey {
                    media: asset.id,
                    timestamp: source,
                };
                // Skip what is already decoded: re-planning every frame would
                // otherwise ask for the same second of work sixty times a
                // second.
                if self.cache.contains(&key) || prefetcher.buffer().contains(&key) {
                    continue;
                }
                items.push((asset.clone(), source));
            }
            tick = tick.saturating_add(interval);
        }

        if items.is_empty() {
            return;
        }

        prefetcher.submit(crate::prefetcher::Plan {
            generation,
            proxy: self.proxy.clone(),
            items,
        });
    }

    pub fn cache(&self) -> &FrameCache {
        &self.cache
    }

    pub fn last_seek_error(&self) -> TimelineTime {
        TimelineTime::from_ticks(self.frames.last_seek_error().ticks())
    }

    /// Drop cached frames and decoders for one asset.
    /// Read preview frames from proxies where they exist (§14).
    pub fn set_proxy_source(&mut self, proxy: Option<ProxySource>) {
        self.proxy = proxy.clone();
        // Every open decoder points at whichever copy was current when it was
        // opened, and every cached frame came from one of them.
        self.frames.set_proxy_source(proxy);
        self.cache.clear();
        // Frames already decoded ahead came from the old copy too.
        self.reset_prefetch();
    }

    /// Forget everything cached or open for one asset.
    ///
    /// Called when a proxy finishes encoding (§13): the frames already cached
    /// are from the original and still correct, but the point is to start using
    /// the proxy, and keeping both wastes the budget.
    pub fn invalidate(&mut self, media: MediaId) {
        self.cache.invalidate_media(media);
        self.frames.invalidate(media);
        // Audio decoders live on the mixer thread now (§20a.2), which notices a
        // changed asset in the next plan it is sent — see `AudioMixer`.
    }

    pub fn set_cache_bytes(&mut self, bytes: usize) {
        self.cache.set_max_bytes(bytes);
    }

    /// Everything visible at `position`, bottom track first (§22).
    ///
    /// A track that fails to decode is **skipped, not fatal**: §50 requires a
    /// media failure to mark the clip unavailable and let the session continue.
    /// One corrupt file must not black out the other tracks.
    pub fn resolve_video(
        &mut self,
        project: &Project,
        sequence: &Sequence,
        position: TimelineTime,
    ) -> Vec<ResolvedLayer> {
        let mut layers = Vec::new();

        for request in layer_requests(project, sequence, position) {
            let frame = match request.source {
                LayerSource::Media(media) => {
                    let Some(asset) = project.media_asset(media) else {
                        continue;
                    };
                    match self.frame_at(asset, request.source_time) {
                        Ok(frame) => frame,
                        Err(err) => {
                            tracing::warn!(
                                file = %asset.file_name,
                                %err,
                                "could not decode a frame; skipping this track"
                            );
                            continue;
                        }
                    }
                }
                // §26.1: the same rasterizer the export uses, so the title on
                // screen is the title in the file.
                LayerSource::Solid { rgb } => Arc::new(solid_frame(rgb)),
                LayerSource::BurnIn => {
                    let Some(clip) = burn_in_clip(
                        project,
                        sequence,
                        bettercut_foundation::TimelineTime::from_ticks(request.source_time.ticks()),
                    ) else {
                        continue;
                    };
                    match self.text.frame_for(&clip, None) {
                        Some(frame) => frame,
                        None => continue,
                    }
                }
                LayerSource::Text(clip) => {
                    let Some(text) = sequence.text_clip(clip) else {
                        continue;
                    };
                    let into =
                        bettercut_foundation::TimelineTime::from_ticks(request.source_time.ticks());
                    match self.text.frame_at(text, request.reveal, into) {
                        Some(frame) => frame,
                        None => continue,
                    }
                }
            };

            let transform = layer_transform(&request, &frame, sequence.resolution);

            layers.push(ResolvedLayer {
                clip: request.clip,
                track: request.track,
                frame,
                look: bettercut_timeline::ClipLook {
                    corner_pin: Default::default(),
                    transform,
                    ..request.look
                },
            });
        }

        layers
    }

    /// The font families available for §26's text overlays.
    /// Make an imported font available to the preview's titles.
    pub fn add_font_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<Vec<String>, bettercut_text::TextError> {
        self.text.add_font_file(path)
    }

    pub fn font_families(&self) -> Vec<String> {
        self.text.families()
    }

    /// Which audio clips are audible in `[position, position + duration)`.
    ///
    /// Returns descriptions rather than samples: the mixer thread owns the
    /// decoding, and this runs wherever the project happens to be borrowed.
    pub fn resolve_audio(
        sequence: &Sequence,
        position: TimelineTime,
        duration: TimelineTime,
    ) -> Vec<AudibleClip> {
        resolve_audio_tracks(&sequence.audio_tracks, position, duration)
    }

    /// Decode (or recall) the frame of `asset` at `source_time`.
    fn frame_at(
        &mut self,
        asset: &MediaAsset,
        source_time: MediaTime,
    ) -> Result<Arc<VideoFrame>, PlaybackError> {
        // A colour clip is drawn each time rather than cached, so changing
        // its colour needs no invalidation; it is a few hundred small rows.
        if let Some(frame) = asset.generated_frame() {
            return Ok(Arc::new(frame));
        }
        // Every instant of a still is the same picture, and keyed as one it
        // is decoded once rather than once per frame.
        let source_time = asset.frame_time(source_time);
        let key = FrameKey {
            media: asset.id,
            timestamp: source_time,
        };
        if let Some(frame) = self.cache.get(&key) {
            return Ok(frame);
        }

        // §47a.3: the decode-ahead thread may already have this. Taking it
        // frees its bytes back to the ring, which is what releases the decode
        // thread to work further ahead.
        if let Some(prefetcher) = self.prefetcher.as_ref()
            && let Some(frame) = prefetcher.buffer().take(&key)
        {
            self.prefetch_hits += 1;
            self.cache.insert(key, Arc::clone(&frame));
            return Ok(frame);
        }

        // Nothing had it, so decode here and now — on the UI thread, which is
        // why keeping this cheap matters. `FrameSource` reads forward without
        // seeking when the frame wanted is simply the next one (§47a.2), and
        // falls back to a `Precise` seek when the playhead has actually jumped.
        //
        // Scrubbing will want `Scrub` here once §17's quality ladder is wired
        // up; `Precise` is the right fallback for stepping and for stopping.
        let frame = self
            .frames
            .decode(asset, source_time, SeekMode::Precise, &NeverCancelled)?;

        // Key the cache by the frame's *actual* timestamp as well, so a later
        // request landing on the same frame is a hit even if it asks for a
        // slightly different instant.
        self.cache.insert(key, Arc::clone(&frame));
        Ok(frame)
    }
}

/// Map a timeline instant onto a position inside a clip's source media.
///
/// No speed change in the MVP (§59), so a tick on the timeline is a tick in the
/// source and this is a straight offset.
pub fn source_time_of(
    timeline_start: TimelineTime,
    source_start: MediaTime,
    position: TimelineTime,
) -> MediaTime {
    let into_clip = position.ticks() - timeline_start.ticks();
    MediaTime::from_ticks(source_start.ticks() + into_clip.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_at_the_clip_start_maps_to_the_source_in_point() {
        let mapped = source_time_of(
            TimelineTime::from_seconds(10),
            MediaTime::from_seconds(3),
            TimelineTime::from_seconds(10),
        );
        assert_eq!(mapped, MediaTime::from_seconds(3));
    }

    #[test]
    fn moving_along_the_timeline_moves_equally_far_into_the_source() {
        let mapped = source_time_of(
            TimelineTime::from_seconds(10),
            MediaTime::from_seconds(3),
            TimelineTime::from_seconds(14),
        );
        assert_eq!(
            mapped,
            MediaTime::from_seconds(7),
            "timeline and source advanced by different amounts"
        );
    }

    /// A trimmed clip shows later material, and the mapping has to follow the
    /// in-point rather than assuming the clip starts at the media's start.
    #[test]
    fn a_trimmed_clip_maps_from_its_in_point() {
        let mapped = source_time_of(
            TimelineTime::ZERO,
            MediaTime::from_seconds(60),
            TimelineTime::from_seconds(5),
        );
        assert_eq!(mapped, MediaTime::from_seconds(65));
    }

    #[test]
    fn a_position_before_the_clip_clamps_to_its_in_point() {
        let mapped = source_time_of(
            TimelineTime::from_seconds(10),
            MediaTime::from_seconds(3),
            TimelineTime::ZERO,
        );
        assert_eq!(
            mapped,
            MediaTime::from_seconds(3),
            "mapped to before the start of the media"
        );
    }
}
