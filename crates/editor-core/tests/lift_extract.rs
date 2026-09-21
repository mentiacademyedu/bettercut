//! Lift and extract the marked range (`Editor::lift_marked`, `extract_marked`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A 10 s shot with sound from 0, marks at 4 s and 6 s, and markers at 5 s
/// and 8 s.
fn marked() -> Editor {
    let (mut editor, _events) = Editor::new_project("Marks");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/talk.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    editor.set_mark_in(seconds(4)).unwrap();
    editor.set_mark_out(seconds(6)).unwrap();
    editor.add_markers(&[seconds(5), seconds(8)]).unwrap();
    editor
}

fn spans(editor: &Editor) -> Vec<(TimelineTime, TimelineTime)> {
    let mut spans: Vec<_> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| (c.timeline.start, c.timeline.end))
        .collect();
    spans.sort();
    spans
}

#[test]
fn a_lift_leaves_the_gap() {
    let mut editor = marked();
    let depth = editor.undo_depth();
    let removed = editor.lift_marked().unwrap();
    assert_eq!(removed, 2, "the picture and the sound inside the marks");
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(
        spans(&editor),
        vec![(seconds(0), seconds(4)), (seconds(6), seconds(10))]
    );
    let times: Vec<_> = editor.markers().iter().map(|m| m.time).collect();
    assert_eq!(times, vec![seconds(8)], "the marker inside was not removed");

    editor.undo().unwrap();
    assert_eq!(spans(&editor), vec![(seconds(0), seconds(10))]);
}

#[test]
fn an_extract_closes_the_gap_on_every_lane() {
    let mut editor = marked();
    editor.extract_marked().unwrap();
    assert_eq!(
        spans(&editor),
        vec![(seconds(0), seconds(4)), (seconds(4), seconds(8))]
    );
    let sound: Vec<_> = editor.active_sequence().unwrap().audio_tracks[0]
        .clips()
        .iter()
        .map(|c| (c.timeline.start, c.timeline.end))
        .collect();
    assert_eq!(
        sound,
        vec![(seconds(0), seconds(4)), (seconds(4), seconds(8))],
        "the sound fell out of step"
    );
    let times: Vec<_> = editor.markers().iter().map(|m| m.time).collect();
    assert_eq!(times, vec![seconds(6)], "the later marker did not follow");
    assert!(editor.active_sequence().unwrap().marked_range().is_none());
}

#[test]
fn no_marks_is_refused() {
    let (mut editor, _events) = Editor::new_project("Marks");
    assert!(matches!(
        editor.lift_marked(),
        Err(EditorError::NoMarkedRange)
    ));
    assert!(matches!(
        editor.extract_marked(),
        Err(EditorError::NoMarkedRange)
    ));
}
