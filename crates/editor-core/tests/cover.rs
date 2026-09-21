//! The cover frame (`Editor::set_cover_frame`, `cover::cover_path_for`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::cover::cover_path_for;
use bettercut_editor_core::foundation::TimelineTime;

#[test]
fn a_cover_is_chosen_cleared_undone_and_saved() {
    let (mut editor, _events) = Editor::new_project("Cover");
    let depth = editor.undo_depth();
    editor
        .set_cover_frame(Some(TimelineTime::from_seconds(4)))
        .unwrap();
    assert_eq!(editor.cover_frame(), Some(TimelineTime::from_seconds(4)));
    // The same frame again is not a step.
    editor
        .set_cover_frame(Some(TimelineTime::from_seconds(4)))
        .unwrap();
    assert_eq!(editor.undo_depth(), depth + 1);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cover.vproj");
    editor.save_as(&path).unwrap();
    let (opened, _) = Editor::open(&path).unwrap();
    assert_eq!(opened.cover_frame(), Some(TimelineTime::from_seconds(4)));

    editor.set_cover_frame(None).unwrap();
    assert_eq!(editor.cover_frame(), None);
    editor.undo().unwrap();
    assert_eq!(editor.cover_frame(), Some(TimelineTime::from_seconds(4)));
}

#[test]
fn the_cover_is_named_after_the_export_beside_it() {
    let export = std::path::Path::new("D:/exports/Holiday.mp4");
    assert_eq!(
        cover_path_for(export),
        std::path::Path::new("D:/exports/Holiday cover.png")
    );
}
