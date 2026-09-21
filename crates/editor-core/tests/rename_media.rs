//! Naming a file in the project (`Editor::rename_media`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn with_file() -> (Editor, bettercut_editor_core::foundation::MediaId) {
    let (mut editor, _events) = Editor::new_project("Names");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/C0042.MP4",
        MediaTime::from_seconds(4),
    ));
    (editor, media)
}

fn shown(editor: &Editor, media: bettercut_editor_core::foundation::MediaId) -> String {
    editor
        .project()
        .media_asset(media)
        .unwrap()
        .display_name()
        .to_owned()
}

#[test]
fn a_file_takes_a_name_and_undo_takes_it_away() {
    let (mut editor, media) = with_file();
    assert_eq!(shown(&editor, media), "C0042.MP4");

    assert!(editor.rename_media(media, "  Interview, wide ").unwrap());
    assert_eq!(shown(&editor, media), "Interview, wide");
    let asset = editor.project().media_asset(media).unwrap();
    assert_eq!(asset.file_name, "C0042.MP4", "the file name itself changed");
    assert!(asset.path.ends_with("C0042.MP4"), "the path changed");

    assert!(
        !editor.rename_media(media, "Interview, wide").unwrap(),
        "the same name again was a step"
    );
    editor.undo().unwrap();
    assert_eq!(shown(&editor, media), "C0042.MP4");
}

/// Clearing the name, or typing the file name, shows the file name again and
/// stores nothing.
#[test]
fn an_empty_name_or_the_file_name_goes_back_to_the_file() {
    let (mut editor, media) = with_file();
    editor.rename_media(media, "B-roll").unwrap();
    assert!(editor.rename_media(media, "   ").unwrap());
    assert_eq!(editor.project().media_asset(media).unwrap().label, None);

    editor.rename_media(media, "B-roll").unwrap();
    editor.rename_media(media, "C0042.MP4").unwrap();
    assert_eq!(editor.project().media_asset(media).unwrap().label, None);

    let long = "x".repeat(500);
    editor.rename_media(media, &long).unwrap();
    assert_eq!(
        shown(&editor, media).chars().count(),
        Editor::MAX_MEDIA_NAME
    );
}

/// The name is saved with the project, and a project from before it existed
/// opens with none.
#[test]
fn a_name_survives_a_save_and_old_projects_have_none() {
    let (mut editor, media) = with_file();
    editor.rename_media(media, "Drone shot").unwrap();
    let json = serde_json::to_value(editor.project()).unwrap();
    let back: bettercut_editor_core::project_format::Project =
        serde_json::from_value(json.clone()).unwrap();
    assert_eq!(
        back.media_asset(media).unwrap().display_name(),
        "Drone shot"
    );

    let mut old = json;
    let asset = old["media"][0].as_object_mut().unwrap();
    assert!(
        asset.remove("label").is_some(),
        "the field is not where this test looks"
    );
    let back: bettercut_editor_core::project_format::Project = serde_json::from_value(old).unwrap();
    assert_eq!(back.media_asset(media).unwrap().label, None);
}
