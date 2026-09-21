//! Quarter turns (`Editor::rotate_quarter`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{ClipProperty, Editor};

fn shots(count: usize) -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Turns");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/phone.mp4",
        MediaTime::from_seconds(4),
    ));
    let clips = (0..count)
        .map(|_| editor.place_media(media).unwrap()[0])
        .collect();
    (editor, clips)
}

fn rotation(editor: &Editor, clip: ClipId) -> f32 {
    editor.video_clip(clip).unwrap().transform.rotation_degrees
}

#[test]
fn a_selection_turns_together_in_one_step() {
    let (mut editor, clips) = shots(2);
    let depth = editor.undo_depth();
    assert_eq!(editor.rotate_quarter(&clips, 1).unwrap(), 2);
    assert_eq!(rotation(&editor, clips[0]), 90.0);
    assert_eq!(rotation(&editor, clips[1]), 90.0);
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(rotation(&editor, clips[0]), 0.0);
}

#[test]
fn turns_add_up_and_wrap() {
    let (mut editor, clips) = shots(1);
    let clip = clips[0];
    editor.rotate_quarter(&clips, -1).unwrap();
    assert_eq!(rotation(&editor, clip), -90.0);
    editor.rotate_quarter(&clips, -1).unwrap();
    assert_eq!(rotation(&editor, clip), 180.0);
    for _ in 0..2 {
        editor.rotate_quarter(&clips, 1).unwrap();
    }
    assert_eq!(rotation(&editor, clip), 0.0);

    // From a hand-set angle, a quarter turn is added to it.
    editor
        .set_clip_property(clip, ClipProperty::Rotation(10.0), false)
        .unwrap();
    editor.rotate_quarter(&clips, 1).unwrap();
    assert_eq!(rotation(&editor, clip), 100.0);
}

#[test]
fn nothing_to_turn_is_no_step() {
    let (mut editor, _) = shots(1);
    let depth = editor.undo_depth();
    assert_eq!(editor.rotate_quarter(&[], 1).unwrap(), 0);
    assert_eq!(editor.undo_depth(), depth);
}
