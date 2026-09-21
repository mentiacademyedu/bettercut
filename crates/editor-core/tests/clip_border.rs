//! Rounded corners and a border on a picture clip (`ClipProperty::Border`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::Border;
use bettercut_editor_core::{ClipProperty, Editor};

fn two_shots() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Border");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(10),
    ));
    let first = editor.place_media(media).unwrap()[0];
    let second = editor.place_media(media).unwrap()[0];
    (editor, first, second)
}

const FRAMED: Border = Border {
    radius: 0.4,
    width: 0.05,
    colour: [20, 200, 90],
};

#[test]
fn a_border_is_set_undone_and_kept_in_range() {
    let (mut editor, clip, _) = two_shots();
    editor
        .set_clip_property(clip, ClipProperty::Border(FRAMED), false)
        .unwrap();
    assert_eq!(editor.video_clip(clip).unwrap().border, FRAMED);
    assert!(!ClipProperty::Border(FRAMED).is_default());
    assert!(ClipProperty::Border(Border::NONE).is_default());

    editor.undo().unwrap();
    assert_eq!(editor.video_clip(clip).unwrap().border, Border::NONE);

    editor
        .set_clip_property(
            clip,
            ClipProperty::Border(Border {
                radius: 7.0,
                width: f32::NAN,
                colour: [1, 2, 3],
            }),
            false,
        )
        .unwrap();
    let border = editor.video_clip(clip).unwrap().border;
    assert_eq!(border.radius, 1.0);
    assert_eq!(border.width, 0.0);
}

/// Part of a copied look, so a framed picture-in-picture style pastes onto
/// the next shot.
#[test]
fn a_border_travels_with_a_copied_look() {
    let (mut editor, from, _) = two_shots();
    editor
        .set_clip_property(from, ClipProperty::Border(FRAMED), false)
        .unwrap();
    let look = editor.clip_look(from).unwrap();
    assert!(look.contains(&ClipProperty::Border(FRAMED)));
    assert!(Editor::plain_look().contains(&ClipProperty::Border(Border::NONE)));
}

/// A drop shadow is set and undone the same way, kept in range, and part of
/// a copied look.
#[test]
fn a_shadow_is_set_clamped_and_copied() {
    use bettercut_editor_core::timeline::{MAX_SHADOW_DISTANCE, Shadow};

    let (mut editor, clip, _) = two_shots();
    let shadow = Shadow {
        opacity: 0.7,
        distance: 5.0,
        angle_degrees: -90.0,
        ..Shadow::NONE
    };
    editor
        .set_clip_property(clip, ClipProperty::Shadow(shadow), false)
        .unwrap();
    let kept = editor.video_clip(clip).unwrap().shadow;
    assert_eq!(kept.opacity, 0.7);
    assert_eq!(kept.distance, MAX_SHADOW_DISTANCE);
    assert_eq!(kept.angle_degrees, 270.0);
    assert!(
        editor
            .clip_look(clip)
            .unwrap()
            .contains(&ClipProperty::Shadow(kept))
    );
    assert!(ClipProperty::Shadow(Shadow::NONE).is_default());

    editor.undo().unwrap();
    assert!(!editor.video_clip(clip).unwrap().shadow.is_visible());
}
