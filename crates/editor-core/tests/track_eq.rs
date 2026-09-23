//! A sound lane's equaliser as an edit (`Command::SetTrackEq`): a sound
//! lane's and never a picture lane's, held to the clip EQ's ranges, one step
//! for a drag, flat by default.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::TrackId;
use bettercut_editor_core::timeline::ClipEq;
use bettercut_editor_core::{Editor, EditorError};

fn editor() -> (Editor, TrackId, TrackId) {
    let (editor, _events) = Editor::new_project("Lane EQ");
    let (lane, picture) = {
        let sequence = editor.active_sequence().unwrap();
        (sequence.audio_tracks[0].id, sequence.video_tracks[0].id)
    };
    (editor, lane, picture)
}

fn warmer() -> ClipEq {
    ClipEq {
        low_cut: 90.0,
        high_cut: 9_000.0,
        presence: -2.0,
        hum: 0.0,
    }
}

#[test]
fn a_lane_starts_flat_and_takes_an_equaliser_as_one_step() {
    let (mut editor, lane, _) = editor();
    assert!(editor.track_eq(lane).is_flat());
    let depth = editor.undo_depth();

    editor.set_track_eq(lane, warmer(), false).unwrap();
    assert_eq!(editor.track_eq(lane), warmer());
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert!(editor.track_eq(lane).is_flat());
}

/// Held to the clip equaliser's own ranges, so the lane can never ask the
/// mixer for a filter it would refuse.
#[test]
fn the_lane_equaliser_is_held_to_its_ranges() {
    let (mut editor, lane, _) = editor();
    let wild = ClipEq {
        low_cut: 5_000.0,
        high_cut: 5.0,
        presence: 40.0,
        hum: 33.0,
    };
    editor.set_track_eq(lane, wild, false).unwrap();
    assert_eq!(editor.track_eq(lane), wild.clamped());
    assert_ne!(editor.track_eq(lane), wild);
}

#[test]
fn a_drag_is_one_step() {
    let (mut editor, lane, _) = editor();
    let depth = editor.undo_depth();
    for step in 1..=10 {
        let eq = ClipEq {
            presence: step as f32 * 0.5,
            ..ClipEq::default()
        };
        editor.set_track_eq(lane, eq, step > 1).unwrap();
    }
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(editor.track_eq(lane).presence, 5.0);
}

/// A picture lane has nothing to equalise.
#[test]
fn a_picture_lane_is_refused() {
    let (mut editor, _, picture) = editor();
    assert!(matches!(
        editor.set_track_eq(picture, warmer(), false),
        Err(EditorError::ClipKindMismatch)
    ));
    assert!(editor.track_eq(picture).is_flat());
}
