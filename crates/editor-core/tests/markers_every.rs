//! A marker grid (`Editor::add_markers_every`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

#[test]
fn a_marker_at_every_interval_inside_the_edit_or_the_range() {
    let (mut editor, _events) = Editor::new_project("Grid");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/talk.mp4",
        MediaTime::from_seconds(10),
    ));
    editor.place_media(media).unwrap();
    let depth = editor.undo_depth();

    assert_eq!(
        editor
            .add_markers_every(TimelineTime::from_seconds(3))
            .unwrap(),
        3
    );
    let at: Vec<_> = editor.markers().iter().map(|m| m.time).collect();
    assert_eq!(at, [3, 6, 9].map(TimelineTime::from_seconds));
    assert_eq!(editor.undo_depth(), depth + 1, "one step");

    // Again: nothing new.
    assert_eq!(
        editor
            .add_markers_every(TimelineTime::from_seconds(3))
            .unwrap(),
        0
    );
    assert!(matches!(
        editor.add_markers_every(TimelineTime::from_ticks(10)),
        Err(EditorError::TooManyMarkers)
    ));
}
