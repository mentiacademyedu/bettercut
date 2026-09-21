//! §45's corner pin on a clip: set, undone, copied with a look.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{CornerPin, MAX_CORNER_REACH};
use bettercut_editor_core::{ClipProperty, Editor, EditorError};

fn editor_with_clip() -> (Editor, bettercut_editor_core::foundation::ClipId) {
    let (mut editor, _events) = Editor::new_project("Pin");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/screen.mp4",
        MediaTime::from_seconds(5),
    ));
    let clip = editor.place_media(media).unwrap()[0];
    (editor, clip)
}

#[test]
fn a_corner_is_pinned_and_undone() {
    let (mut editor, clip) = editor_with_clip();
    assert!(editor.video_clip(clip).unwrap().corner_pin.is_none());
    let depth = editor.undo_depth();

    let pin = CornerPin::NONE.with_corner(0, [-0.1, -0.05]);
    editor
        .set_clip_property(clip, ClipProperty::CornerPin(pin), false)
        .unwrap();

    let now = editor.video_clip(clip).unwrap().corner_pin;
    assert_eq!(now.offsets[0], [-0.1, -0.05]);
    assert!(!now.is_none());
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert!(editor.video_clip(clip).unwrap().corner_pin.is_none());
}

#[test]
fn a_corner_dragged_absurdly_far_is_held() {
    let (mut editor, clip) = editor_with_clip();
    editor
        .set_clip_property(
            clip,
            ClipProperty::CornerPin(CornerPin {
                offsets: [[50.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]],
            }),
            false,
        )
        .unwrap();
    assert_eq!(
        editor.video_clip(clip).unwrap().corner_pin.offsets[0][0],
        MAX_CORNER_REACH
    );
}

/// The pin travels with a copied look, like the border and the shadow.
#[test]
fn a_pin_travels_with_a_copied_look() {
    let (mut editor, clip) = editor_with_clip();
    let pin = CornerPin::NONE.with_corner(2, [0.2, 0.1]);
    editor
        .set_clip_property(clip, ClipProperty::CornerPin(pin), false)
        .unwrap();

    let look = editor.clip_look(clip).unwrap();
    assert!(look.contains(&ClipProperty::CornerPin(pin)));
    // And a plain look puts it back.
    assert!(
        Editor::plain_look().contains(&ClipProperty::CornerPin(CornerPin::NONE)),
        "a plain look leaves a pinned corner where it was"
    );
}

/// A pin is about one picture inside another; the whole video has no corners
/// of its own to move.
#[test]
fn the_whole_video_cannot_be_pinned() {
    let (mut editor, _clip) = editor_with_clip();
    assert!(matches!(
        editor.set_sequence_value(
            ClipProperty::CornerPin(CornerPin::NONE.with_corner(0, [0.1, 0.1])),
            false
        ),
        Err(EditorError::ClipKindMismatch)
    ));
}
