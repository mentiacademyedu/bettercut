//! Cropping a clip to a shape in one click (`Editor::crop_to_shape`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, FrameRate, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn wide_shot() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Crops");
    let media = editor.import_media(
        MediaAsset::new(
            MediaKind::Video,
            "C:/media/wide.mp4",
            MediaTime::from_seconds(5),
        )
        .with_video(1920, 1080, FrameRate::new(30, 1).unwrap()),
    );
    let clip = editor.place_media(media).unwrap()[0];
    (editor, clip)
}

#[test]
fn a_square_takes_equal_strips_off_the_sides() {
    let (mut editor, clip) = wide_shot();
    editor.crop_to_shape(clip, Some((1, 1))).unwrap();
    let crop = editor.video_clip(clip).unwrap().crop;
    // 1080 of 1920 kept across: 420 off each side.
    assert!((crop.left - 420.0 / 1920.0).abs() < 1e-4, "{crop:?}");
    assert!((crop.right - crop.left).abs() < 1e-6);
    assert_eq!((crop.top, crop.bottom), (0.0, 0.0));
    assert_eq!(editor.crop_shape_of(clip), Some((1, 1)));

    editor.crop_to_shape(clip, Some((9, 16))).unwrap();
    assert_eq!(editor.crop_shape_of(clip), Some((9, 16)));

    // A wide shot cropped to its own shape keeps everything.
    editor.crop_to_shape(clip, Some((16, 9))).unwrap();
    assert!(editor.video_clip(clip).unwrap().crop.is_none());

    editor.crop_to_shape(clip, Some((4, 5))).unwrap();
    editor.crop_to_shape(clip, None).unwrap();
    assert!(editor.video_clip(clip).unwrap().crop.is_none());
    assert_eq!(editor.crop_shape_of(clip), None);

    editor.undo().unwrap();
    assert_eq!(editor.crop_shape_of(clip), Some((4, 5)));
}

#[test]
fn a_picture_of_unknown_size_is_refused() {
    let (mut editor, _events) = Editor::new_project("Crops");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/odd.mp4",
        MediaTime::from_seconds(5),
    ));
    let clip = editor.place_media(media).unwrap()[0];
    assert!(matches!(
        editor.crop_to_shape(clip, Some((1, 1))),
        Err(EditorError::UnknownPictureSize)
    ));
}
