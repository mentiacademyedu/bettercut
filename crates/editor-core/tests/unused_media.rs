//! Taking unused media out of the library.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};

/// Only files no clip uses go, all in one undo step, and undo brings every one
/// back.
#[test]
fn unused_files_go_in_one_step_and_come_back() {
    let (mut editor, _events) = Editor::new_project("Library");
    let used = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/used.mp4",
        MediaTime::from_seconds(4),
    ));
    editor.place_media(used).unwrap();
    let spare = editor.import_media(MediaAsset::new(
        MediaKind::Audio,
        "C:/media/spare.mp3",
        MediaTime::from_seconds(30),
    ));
    let photo = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/logo.png",
        MediaTime::ZERO,
    ));

    assert_eq!(editor.unused_media(), vec![spare, photo]);
    assert_eq!(editor.remove_unused_media().unwrap(), 2);
    let library: Vec<_> = editor.project().media.iter().map(|m| m.id).collect();
    assert_eq!(
        library,
        vec![used],
        "the used file went, or an unused one stayed"
    );
    assert_eq!(
        editor.undo_label().as_deref(),
        Some("Remove 2 Unused Files")
    );

    let depth = editor.undo_depth();
    assert_eq!(editor.remove_unused_media().unwrap(), 0);
    assert_eq!(editor.undo_depth(), depth, "nothing to remove made a step");

    editor.undo().unwrap();
    let mut back: Vec<_> = editor.project().media.iter().map(|m| m.id).collect();
    back.sort();
    let mut all = vec![used, spare, photo];
    all.sort();
    assert_eq!(back, all);
}
