//! Markers follow a ripple delete and a closed gap, as they do an inserted
//! gap: a mark inside the closed stretch goes, one after it moves back, and
//! undo puts them all where they were.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_media::{MediaAsset, MediaKind};

fn secs(s: i64) -> TimelineTime {
    TimelineTime::from_seconds(s)
}

/// Two silent shots, 0–4 s and 4–10 s, with marks at 2, 6 and 9 s.
fn edit() -> (Editor, TrackId, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Ripple");
    let mut clips = Vec::new();
    for (name, seconds) in [("a", 4), ("b", 6)] {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(seconds),
        ));
        clips.push(editor.place_media(media).unwrap()[0]);
    }
    editor.add_markers(&[secs(2), secs(6), secs(9)]).unwrap();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    (editor, track, clips[0], clips[1])
}

fn times(editor: &Editor) -> Vec<TimelineTime> {
    editor.markers().iter().map(|m| m.time).collect()
}

#[test]
fn a_ripple_delete_takes_its_marks_and_pulls_the_rest_back() {
    let (mut editor, track, first, _) = edit();
    let depth = editor.undo_depth();
    editor.ripple_delete(track, first).unwrap();
    assert_eq!(
        times(&editor),
        [secs(2), secs(5)],
        "6 and 9 moved back by 4; 2 went"
    );
    assert_eq!(editor.undo_depth(), depth + 1, "one step with the delete");
    editor.undo().unwrap();
    assert_eq!(times(&editor), [secs(2), secs(6), secs(9)]);
}

#[test]
fn closing_a_gap_moves_the_marks_after_it() {
    let (mut editor, track, first, _) = edit();
    // Plain delete leaves a 0–4 s gap; the marks stay. Closing it is what
    // moves them.
    editor.remove_clip(track, first).unwrap();
    assert_eq!(times(&editor), [secs(2), secs(6), secs(9)]);
    editor.close_gap(track, secs(1)).unwrap();
    assert_eq!(times(&editor), [secs(2), secs(5)]);
    editor.undo().unwrap();
    assert_eq!(times(&editor), [secs(2), secs(6), secs(9)]);
}
