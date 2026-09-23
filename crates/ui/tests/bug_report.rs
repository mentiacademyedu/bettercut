//! Details for a bug report.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn the_report_says_the_shape_of_the_project_and_nothing_private() {
    let (mut editor, _events) = Editor::new_project("Secret project");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/private/holiday.mp4",
        MediaTime::from_seconds(2),
    ));
    editor.place_media(media).unwrap();
    let text = bettercut_ui::bug_report::details(&editor);
    assert!(text.starts_with("bettercut "), "{text}");
    assert!(text.contains("1 media file"), "{text}");
    assert!(text.contains("clip(s)"), "{text}");
    assert!(text.contains("What happened:"));
    assert!(!text.contains("holiday"), "a file name leaked: {text}");
    assert!(!text.contains("private"), "a path leaked: {text}");
    assert!(!text.contains("Secret"), "the project name leaked: {text}");
}
