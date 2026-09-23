//! A clip's own name (`Editor::set_clip_name`): trimmed, capped, one step,
//! `None` or empty for the file's name back, on any kind of lane.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::timeline::MAX_CLIP_NAME;
use bettercut_media::{MediaAsset, MediaKind};

fn shot() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Names");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/C0042.MP4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0], placed[1])
}

#[test]
fn a_name_is_kept_trimmed_and_undone() {
    let (mut editor, picture, sound) = shot();
    assert_eq!(editor.clip_name(picture), None);
    let depth = editor.undo_depth();

    assert!(
        editor
            .set_clip_name(picture, Some("  interview wide  "))
            .unwrap()
    );
    assert_eq!(editor.clip_name(picture).as_deref(), Some("interview wide"));
    assert_eq!(
        editor.clip_name(sound),
        None,
        "the sound keeps the file's name"
    );
    assert_eq!(editor.undo_depth(), depth + 1);
    assert!(
        !editor
            .set_clip_name(picture, Some("interview wide"))
            .unwrap()
    );
    assert_eq!(editor.undo_depth(), depth + 1);

    assert!(editor.set_clip_name(sound, Some("room tone")).unwrap());
    assert_eq!(editor.clip_name(sound).as_deref(), Some("room tone"));

    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(editor.clip_name(picture), None);

    let long = "x".repeat(MAX_CLIP_NAME + 20);
    editor.set_clip_name(picture, Some(&long)).unwrap();
    assert_eq!(
        editor.clip_name(picture).unwrap().chars().count(),
        MAX_CLIP_NAME
    );
    assert!(editor.set_clip_name(picture, Some("   ")).unwrap());
    assert_eq!(
        editor.clip_name(picture),
        None,
        "an empty name is the file's"
    );
    assert!(editor.set_clip_name(ClipId::new(), Some("ghost")).is_err());
}
