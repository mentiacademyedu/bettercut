//! The deinterlace flag (`Editor::set_media_deinterlace`): one step, no
//! step when unchanged, undone, refused for a file not in the project.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaId, MediaTime};
use bettercut_media::{MediaAsset, MediaKind};

#[test]
fn the_flag_is_one_step_and_undoes() {
    let (mut editor, _events) = Editor::new_project("Tape");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/tape.avi",
        MediaTime::from_seconds(10),
    ));
    let depth = editor.undo_depth();
    assert!(editor.set_media_deinterlace(media, true).unwrap());
    assert!(editor.project().media_asset(media).unwrap().deinterlace);
    assert_eq!(editor.undo_depth(), depth + 1);
    assert!(!editor.set_media_deinterlace(media, true).unwrap());
    assert_eq!(editor.undo_depth(), depth + 1);
    editor.undo().unwrap();
    assert!(!editor.project().media_asset(media).unwrap().deinterlace);
    assert!(editor.set_media_deinterlace(MediaId::new(), true).is_err());
}
