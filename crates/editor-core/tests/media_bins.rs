//! Filing media in bins (`Editor::set_media_bin`, `bins`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn files_are_filed_listed_and_taken_out_again() {
    let (mut editor, _events) = Editor::new_project("Bins");
    let add = |editor: &mut Editor, name: &str| {
        editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(3),
        ))
    };
    let (a, b, c) = (
        add(&mut editor, "a"),
        add(&mut editor, "b"),
        add(&mut editor, "c"),
    );
    assert!(editor.bins().is_empty());

    assert!(editor.set_media_bin(a, "  Interviews ").unwrap());
    editor.set_media_bin(b, "B-roll").unwrap();
    editor.set_media_bin(c, "B-roll").unwrap();
    assert_eq!(
        editor.bins(),
        vec!["B-roll".to_owned(), "Interviews".to_owned()]
    );
    assert_eq!(
        editor.project().media_asset(a).unwrap().bin.as_deref(),
        Some("Interviews")
    );
    assert!(
        !editor.set_media_bin(b, "B-roll").unwrap(),
        "the same bin again was a step"
    );

    editor.set_media_bin(a, "").unwrap();
    assert_eq!(editor.project().media_asset(a).unwrap().bin, None);
    assert_eq!(editor.bins(), vec!["B-roll".to_owned()]);

    editor.undo().unwrap();
    assert_eq!(
        editor.project().media_asset(a).unwrap().bin.as_deref(),
        Some("Interviews")
    );
}
