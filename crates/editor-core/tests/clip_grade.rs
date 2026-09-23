//! Setting a picture clip's whole grade at once, as colour match does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::ColorAdjust;
use bettercut_editor_core::{Editor, EditorError};

#[test]
fn a_whole_grade_is_one_undo_step() {
    let (mut editor, _events) = Editor::new_project("Grade");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(3),
    ));
    editor.place_media(media).unwrap();
    let clip = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    let grade = ColorAdjust {
        brightness: 1.3,
        contrast: 0.9,
        saturation: 1.2,
        temperature: 0.25,
        tint: -0.1,
        vibrance: 0.0,
        wheels: Default::default(),
        secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
    };
    let depth = editor.undo_depth();
    editor.set_clip_grade(clip, grade, "Match Colour").unwrap();
    assert_eq!(editor.video_clip(clip).unwrap().color, grade);
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(
        editor.video_clip(clip).unwrap().color,
        ColorAdjust::default()
    );
}

#[test]
fn only_a_picture_clip_takes_a_grade() {
    let (mut editor, _events) = Editor::new_project("Grade");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(3),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    let clip = editor.active_sequence().unwrap().audio_tracks[0].clips()[0].id;
    assert!(matches!(
        editor.set_clip_grade(clip, ColorAdjust::default(), "Match Colour"),
        Err(EditorError::ClipKindMismatch)
    ));
}
