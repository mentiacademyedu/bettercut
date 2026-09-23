//! A crossfade on every sound cut of a lane
//! (`Editor::crossfade_every_cut`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::timeline::{AudioClip, SourceRange};

/// Three sounds in a row, each four seconds of a twenty-second file, so
/// every join has handles to fade through.
fn lane() -> (
    Editor,
    TrackId,
    Vec<bettercut_editor_core::foundation::ClipId>,
) {
    let (mut editor, _events) = Editor::new_project("Crossfades");
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    let media = MediaId::new();
    let mut ids = Vec::new();
    for index in 0..3 {
        let source =
            SourceRange::new(MediaTime::from_seconds(4), MediaTime::from_seconds(8)).unwrap();
        let clip = AudioClip::new(media, TimelineTime::from_seconds(index * 4), source).unwrap();
        ids.push(clip.id);
        editor
            .add_clip(track, ClipPayload::Audio(Box::new(clip)))
            .unwrap();
    }
    (editor, track, ids)
}

#[test]
fn every_join_takes_a_crossfade_as_one_step() {
    let (mut editor, track, clips) = lane();
    let depth = editor.undo_depth();

    let (done, skipped) = editor
        .crossfade_every_cut(track, TimelineTime::from_millis(250))
        .unwrap();
    assert_eq!((done, skipped), (2, 0), "two joins between three clips");
    for clip in &clips[..2] {
        let fade = editor.audio_clip(*clip).unwrap().crossfade_out;
        assert_eq!(fade, TimelineTime::from_millis(250), "clip {clip:?}");
    }
    assert_eq!(
        editor.audio_clip(clips[2]).unwrap().crossfade_out,
        TimelineTime::ZERO,
        "the last clip has no join after it"
    );

    // One step each, as the crossfades are dispatched one at a time — undo
    // takes them off in order.
    assert!(editor.undo_depth() > depth);
    while editor.undo_depth() > depth {
        editor.undo().unwrap();
    }
    assert_eq!(
        editor.audio_clip(clips[0]).unwrap().crossfade_out,
        TimelineTime::ZERO
    );
}

#[test]
fn a_lane_with_nothing_to_join_does_nothing() {
    let (mut editor, _events) = Editor::new_project("Empty");
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    // An empty lane is a lane that is there: nothing to do, no error.
    assert!(
        editor
            .crossfade_every_cut(track, TimelineTime::from_millis(250))
            .is_err(),
        "an empty lane has no clips at all"
    );

    // A lane with one clip has no join, and says so quietly.
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();
    let clip = AudioClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
    editor
        .add_clip(track, ClipPayload::Audio(Box::new(clip)))
        .unwrap();
    assert_eq!(
        editor
            .crossfade_every_cut(track, TimelineTime::from_millis(250))
            .unwrap(),
        (0, 0)
    );
}
