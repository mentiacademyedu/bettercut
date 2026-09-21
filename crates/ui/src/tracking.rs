//! "Make this follow that": the motion track, from the two clips picked out
//! to the keyframes it leaves behind.
//!
//! The user puts the sticker, title card or blurred patch over the thing it
//! should follow, selects it and the footage, and asks for a track. The box
//! the clip already occupies *is* the patch to follow, so there is no second
//! step where a rectangle is dragged out over the picture — what you see is
//! what is tracked.
//!
//! Decoding happens here and now rather than in a background job: a track is
//! capped at [`Editor::MAX_TRACK_SECONDS`], which is a few hundred frames, and
//! a spinner for two seconds is a better trade than a job queue nobody can
//! follow.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, TICKS_PER_SECOND, TimelineTime};
use bettercut_editor_core::media::{NeverCancelled, SeekMode};
use bettercut_editor_core::track_motion::TrackedPoint;
use bettercut_playback::FrameSource;
use bettercut_playback::tracker::{Grey, Patch, follow};

use crate::state::UiState;

/// Measure how `clip` moves and take the shake out of it.
///
/// The patch followed is the middle of the frame — the part of a hand-held
/// shot that is the subject often enough, and the part least likely to leave
/// the frame while the camera wobbles.
pub fn steady(editor: &mut Editor, state: &mut UiState, clip: ClipId) {
    let Some(path) = measure(editor, state, clip, clip) else {
        return;
    };
    match editor.stabilise(clip, &path) {
        Ok(done) => {
            state.info(format!(
                "Steadied: {} keyframes, cropped in {:.0}%",
                done.keys,
                (done.zoom - 1.0) * 100.0
            ));
            state.needs_repaint = true;
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Follow what is under `follower` through `footage`, and key the follower to
/// it. Says what happened in the status bar either way.
pub fn track_and_attach(
    editor: &mut Editor,
    state: &mut UiState,
    follower: ClipId,
    footage: ClipId,
) {
    let Some(path) = measure(editor, state, follower, footage) else {
        return;
    };
    match editor.attach_to_path(follower, &path) {
        Ok(written) => {
            state.info(format!("Followed with {written} keyframes"));
            state.needs_repaint = true;
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Follow a patch through `footage` and hand back where it went.
///
/// The patch is where `follower` sits — its own box when something is being
/// attached to a moving thing, and the middle of the frame when a clip is
/// being steadied against itself. `None` when there was nothing to measure,
/// with the reason already in the status bar.
fn measure(
    editor: &mut Editor,
    state: &mut UiState,
    follower: ClipId,
    footage: ClipId,
) -> Option<Vec<TrackedPoint>> {
    let Some((start, end)) = editor.track_span(follower, footage) else {
        state.error("Put the playhead where both clips are playing first");
        return None;
    };
    // Steadying a clip against itself: the middle of the frame, big enough to
    // hold something to recognise.
    let box_of = if follower == footage {
        Some(([0.5, 0.5], [0.2, 0.2]))
    } else {
        editor.clip_box(follower)
    };
    let Some((center, half)) = box_of else {
        state.error("Only a picture clip can follow something");
        return None;
    };
    let Some(media) = editor.media_of_clip(footage) else {
        state.error("That clip has no footage to follow");
        return None;
    };
    let Some(asset) = editor.project().media_asset(media).cloned() else {
        state.error("That clip's file is missing");
        return None;
    };

    // One sample every other frame: enough for anything a hand-held camera
    // does, and half the decoding.
    let step = editor
        .active_sequence()
        .map_or(TICKS_PER_SECOND / 15, |sequence| {
            sequence.ticks_per_frame().max(1) * 2
        });

    let mut frames = FrameSource::new(1);
    let mut greys: Vec<(TimelineTime, Grey)> = Vec::new();
    let mut at = start;
    while at <= end {
        let Some(source_time) = editor.source_time_of(footage, at) else {
            break;
        };
        let Ok(frame) = frames.decode(&asset, source_time, SeekMode::Playback, &NeverCancelled)
        else {
            break;
        };
        let Some(grey) = Grey::of(&frame) else {
            break;
        };
        greys.push((at, grey));
        at = TimelineTime::from_ticks(at.ticks() + step);
    }
    if greys.len() < 2 {
        state.error("Could not read enough of that clip to follow anything");
        return None;
    }

    let path = follow(
        greys.iter().map(|(at, grey)| {
            (
                bettercut_editor_core::foundation::MediaTime::from_ticks(at.ticks()),
                grey,
            )
        }),
        Patch { center, half },
    );
    Some(
        path.into_iter()
            .map(|point| TrackedPoint {
                at: TimelineTime::from_ticks(point.at.ticks()),
                center: point.center,
            })
            .collect(),
    )
}
