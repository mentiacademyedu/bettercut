//! Playing a compound clip: the sequence folded inside it, laid out in the
//! place the compound occupies (`bettercut_editor_core::compound`).
//!
//! Nothing is rendered to a file and nothing is decoded: the compound's layers
//! *are* the inner sequence's layers, each carrying the compound's own framing,
//! fade and grade on top of its own. One place the contents live, so opening
//! the compound and changing something changes every frame it appears in.
//!
//! What the compound carries down: where it sits and how big it is, its
//! opacity, its colour grade and its blur. What stays inside: each inner clip's
//! crop, mask, key and blend, which are about that layer against its own
//! source rather than about the compound against the frame.

use bettercut_foundation::TimelineTime;
use bettercut_project_format::Project;
use bettercut_timeline::{ClipLook, ColorAdjust, Transform, Vec2};

use crate::engine::{LayerRequest, LayerSource};

/// How deep compounds may be nested before the rest are left undrawn.
///
/// A limit rather than a cycle check: a compound that somehow contained itself
/// would otherwise plan frames forever, and nobody nests five deep on purpose.
pub const MAX_DEPTH: u8 = 4;

/// The sequence a layer's media stands for, when that media is a compound.
pub fn compound_of(
    project: &Project,
    source: LayerSource,
) -> Option<bettercut_foundation::SequenceId> {
    let media = source.media()?;
    project.media_asset(media)?.generated?.compound()
}

/// `inner` as it looks inside `outer`: the compound's framing, fade, grade and
/// blur applied over the layer's own.
pub fn compose(outer: ClipLook, inner: ClipLook) -> ClipLook {
    ClipLook {
        corner_pin: Default::default(),
        transform: compose_transform(outer.transform, inner.transform),
        opacity: (outer.opacity * inner.opacity).clamp(0.0, 1.0),
        color: compose_colour(outer.color, inner.color),
        blur: (outer.blur + inner.blur).clamp(0.0, 100.0),
        ..inner
    }
}

/// The inner transform carried out to the frame by the outer one: scaled by
/// it, turned by it, and moved with it.
fn compose_transform(outer: Transform, inner: Transform) -> Transform {
    let radians = outer.rotation_degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    // The inner offset is in the compound's own frame, so it is scaled and
    // turned before it is added to where the compound sits.
    let x = inner.position.x * outer.scale.x;
    let y = inner.position.y * outer.scale.y;
    Transform {
        position: Vec2::new(
            outer.position.x + x * cos - y * sin,
            outer.position.y + x * sin + y * cos,
        ),
        scale: Vec2::new(inner.scale.x * outer.scale.x, inner.scale.y * outer.scale.y),
        rotation_degrees: inner.rotation_degrees + outer.rotation_degrees,
        anchor: inner.anchor,
        // Two mirrors are no mirror, which is what `^` says.
        flip_h: inner.flip_h ^ outer.flip_h,
        flip_v: inner.flip_v ^ outer.flip_v,
    }
}

/// Both grades, one after the other: the multipliers multiply and the offsets
/// add, which is what applying one grade to an already-graded picture does.
fn compose_colour(outer: ColorAdjust, inner: ColorAdjust) -> ColorAdjust {
    ColorAdjust {
        brightness: (outer.brightness * inner.brightness).clamp(0.0, 4.0),
        contrast: (outer.contrast * inner.contrast).clamp(0.0, 4.0),
        saturation: (outer.saturation * inner.saturation).clamp(0.0, 4.0),
        temperature: (outer.temperature + inner.temperature).clamp(-1.0, 1.0),
        tint: (outer.tint + inner.tint).clamp(-1.0, 1.0),
        vibrance: (outer.vibrance + inner.vibrance).clamp(-1.0, 1.0),
        wheels: outer.wheels.combined(inner.wheels),
        secondary: outer.secondary.combined(inner.secondary),
    }
}

/// Replace every compound layer in `requests` with the layers of the sequence
/// inside it, read at the instant the compound is playing.
///
/// `plan` is how the inner sequence is laid out — the engine's own planner,
/// passed in so this module does not have to know about transitions, titles or
/// anything else it plans.
pub fn expand(
    project: &Project,
    requests: Vec<LayerRequest>,
    depth: u8,
    plan: impl Fn(&Project, &bettercut_timeline::Sequence, TimelineTime, u8) -> Vec<LayerRequest>,
) -> Vec<LayerRequest> {
    if depth >= MAX_DEPTH
        || !requests
            .iter()
            .any(|r| compound_of(project, r.source).is_some())
    {
        return requests;
    }
    let mut out = Vec::with_capacity(requests.len());
    for request in requests {
        let inner = compound_of(project, request.source).and_then(|id| project.sequence(id));
        let Some(inner) = inner else {
            out.push(request);
            continue;
        };
        // The compound's source time *is* the instant inside it: the inner
        // sequence starts at zero, so a compound trimmed or moved reads from
        // further in without anything else having to know.
        let at = TimelineTime::from_ticks(request.source_time.ticks());
        // A multicam clip shows one camera: the lane its angle names, and
        // nothing else (`editor_core::multicam`). An angle past the end of the
        // lanes shows nothing rather than the wrong camera.
        let only = request.angle.and_then(|angle| {
            inner
                .video_tracks
                .get(angle)
                .map(|track| track.id)
                .or(Some(bettercut_foundation::TrackId::from_u128(0)))
        });
        for mut layer in plan(project, inner, at, depth + 1) {
            if let Some(only) = only
                && layer.track != only
            {
                continue;
            }
            layer.look = compose(request.look, layer.look);
            // On the compound's own lane, so what is graded beneath it, and
            // what draws over it, is decided by where the compound sits rather
            // than by lanes inside it that the parent knows nothing about.
            layer.track = request.track;
            out.push(layer);
        }
    }
    out
}

/// The audio clips inside `sequence`'s compounds, mapped onto the parent's
/// timeline: each inner clip shifted to where the compound plays and cut to
/// what the compound actually shows.
///
/// Returned as extra tracks for [`crate::mixer::AudioPlan`], so a compound's
/// sound plays without the mixer knowing what a compound is.
pub fn audio_tracks(
    project: &Project,
    sequence: &bettercut_timeline::Sequence,
    depth: u8,
) -> Vec<bettercut_timeline::AudioTrack> {
    if depth >= MAX_DEPTH {
        return Vec::new();
    }
    let mut tracks = Vec::new();
    for track in &sequence.video_tracks {
        for clip in track.clips() {
            let Some(inner) = project
                .media_asset(clip.media_id)
                .and_then(|asset| asset.generated)
                .and_then(|generated| generated.compound())
                .and_then(|id| project.sequence(id))
            else {
                continue;
            };
            // Where the compound's window sits inside the inner sequence, and
            // how far that is from where it plays on this timeline.
            let window = clip.source;
            let shift = clip.timeline.start.ticks() - window.start.ticks();
            let mut inner_tracks: Vec<bettercut_timeline::AudioTrack> = inner.audio_tracks.clone();
            inner_tracks.extend(audio_tracks(project, inner, depth + 1));

            for inner_track in inner_tracks {
                let mut moved = bettercut_timeline::AudioTrack::new(inner_track.name.clone());
                // The compound's own lane decides whether its sound plays: a
                // hidden compound is not heard either.
                moved.enabled = track.enabled && inner_track.enabled;
                for inner_clip in inner_track.clips() {
                    let start = inner_clip.timeline.start.ticks().max(window.start.ticks());
                    let end = inner_clip.timeline.end.ticks().min(window.end.ticks());
                    if end <= start {
                        continue; // outside what the compound shows
                    }
                    let mut copy = inner_clip.clone();
                    // Trimmed to the window first, in the clip's own time, so
                    // the source range follows the visible part.
                    let head = start - inner_clip.timeline.start.ticks();
                    let tail = inner_clip.timeline.end.ticks() - end;
                    let source = bettercut_timeline::SourceRange {
                        start: bettercut_foundation::MediaTime::from_ticks(
                            inner_clip.source.start.ticks() + copy.speed.scale(head),
                        ),
                        end: bettercut_foundation::MediaTime::from_ticks(
                            inner_clip.source.end.ticks() - copy.speed.scale(tail),
                        ),
                    };
                    if source.end <= source.start {
                        continue;
                    }
                    copy.source = source;
                    copy.timeline = bettercut_timeline::TimelineRange {
                        start: TimelineTime::from_ticks(start + shift),
                        end: TimelineTime::from_ticks(end + shift),
                    };
                    // A fresh identity: the same clip can play in two places
                    // through two compounds, and the mixer keeps per-clip
                    // state (an echo's tail, a compressor's memory) by id.
                    copy.id = bettercut_foundation::ClipId::new();
                    copy.link = None;
                    let _ = moved.insert(copy);
                }
                if !moved.clips().is_empty() {
                    tracks.push(moved);
                }
            }
        }
    }
    tracks
}
