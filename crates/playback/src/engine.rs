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
            Self::Text(_) | Self::Solid { .. } => None,
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
    let mut requests = Vec::new();

    // §20a.4: while any picture track is soloed, only those are on screen.
    // Asked once rather than per track — it is a fact about the lane.
    let soloed = sequence.video_tracks.iter().any(|track| track.solo);

    for track in &sequence.video_tracks {
        if !bettercut_timeline::track_plays(track.enabled, track.solo, soloed) {
            continue; // hidden, or not the one being soloed (§8)
        }
        let clips = track.clips();
        let index = clips.partition_point(|c| c.timeline().end <= position);
        let Some(clip) = clips.get(index).filter(|c| c.timeline().contains(position)) else {
            continue;
        };

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
                push_moving(
                    &mut requests,
                    project,
                    track.id,
                    incoming,
                    position,
                    in_move,
                );
            }
            None => {
                // §36: something behind a clip that does not fill the frame,
                // drawn first so the clip itself lands on top of it.
                push_backdrop(&mut requests, project, sequence, track.id, clip, position);
                push_layer(&mut requests, project, track.id, clip, position, 1.0);
            }
        }
    }

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
        if clip.is_blank() || !clip.enabled {
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
                    sharpen: 0.0,
                    // A title's colours are chosen, not filmed: nothing to grade.
                    lut: None,
                    rgb_split: 0.0,
                    glitch: 0.0,
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
                    mask: None,
                    // §26: a title covers what is beneath it.
                    blend: bettercut_timeline::BlendMode::Normal,
                },
                reveal: look.reveal,
            });
        }
    }

    requests
}

/// Which audio clips are audible in `[position, position + duration)`.
///
/// Over tracks rather than a sequence so the mixer thread can ask it of the
/// snapshot it holds (§54): the thread never sees the project.
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
    sequence
        .adjustment_tracks
        .iter()
        .filter(|track| bettercut_timeline::track_plays(track.enabled, track.solo, soloed))
        .filter_map(|track| track.clip_at(position))
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
        .take_while(|track| sequence.text_track(*track).is_none())
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
    for track in tracks {
        if !bettercut_timeline::track_plays(track.enabled, track.solo, soloed) {
            continue; // muted, or not the one being soloed (§8)
        }
        let range = bettercut_timeline::TimelineRange {
            start: position,
            end,
        };
        for clip in track.clips_in_range(range) {
            // Where this clip begins inside the requested block.
            let offset = if clip.timeline.start > position {
                clip.timeline.start - position
            } else {
                TimelineTime::ZERO
            };
            let from = position.max(clip.timeline.start);
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
            let fades = bettercut_audio::Fades {
                into_clip: frames(into_clip),
                remaining: frames(clip.timeline.end.ticks() - from.ticks()),
                fade_in: frames(fade_in),
                fade_out: frames(fade_out),
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

            audible.push(AudibleClip {
                clip: clip.id,
                media: clip.media_id,
                source_start,
                speed: clip.speed,
                reversed: clip.reversed,
                denoise: clip.denoise,
                gain: if automation.is_some() { 1.0 } else { clip.gain },
                offset,
                fades,
                automation,
                track_gain: track.gain,
                track_pan: track.pan,
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
                scale: bettercut_timeline::Vec2::new(1.0 / fit_x, 1.0 / fit_y),
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
    for (transform, share) in copies {
        requests.push(LayerRequest {
            clip: clip.id,
            track,
            source: LayerSource::Media(clip.media_id),
            source_time,
            look: bettercut_timeline::ClipLook {
                transform,
                opacity: share,
                ..look
            },
            reveal: None,
        });
    }
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
    if clip.backdrop != bettercut_timeline::Backdrop::Blur {
        return;
    }
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
    request.look.blur = bettercut_timeline::BACKDROP_BLUR;
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
}

impl LayerMove {
    const STILL: Self = Self {
        offset_x: 0.0,
        scale: 1.0,
        alpha: 1.0,
    };
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
            },
            LayerMove::STILL,
        ),
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
        TransitionKind::FadeThroughBlack | TransitionKind::Flash | TransitionKind::Blur => {
            (LayerMove::STILL, LayerMove::STILL)
        }
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
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform::default(),
            opacity: alpha,
            color: bettercut_timeline::ColorAdjust::IDENTITY,
            blur: 0.0,
            chroma_key: None,
            mask: None,
            blend: bettercut_timeline::BlendMode::Normal,
        },
        reveal: None,
    });
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
    pub track_gain: f32,
    pub track_pan: f32,
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
                let Some(asset) = project.media_asset(clip.media_id) else {
                    continue;
                };
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
                    transform,
                    ..request.look
                },
            });
        }

        layers
    }

    /// The font families available for §26's text overlays.
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
