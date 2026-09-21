//! Stickers: an emoji on the title lane (`Editor::add_sticker`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::stickers::{STICKER_SIZE, STICKERS};
use bettercut_editor_core::{Editor, EditorError};

#[test]
fn a_sticker_lands_on_the_title_lane_sized_as_a_sticker() {
    let (mut editor, _events) = Editor::new_project("Stickers");
    let depth = editor.undo_depth();

    let clip = editor.add_sticker("★").unwrap();

    let text = editor.text_clip(clip).unwrap();
    assert_eq!(text.text, "★");
    assert!((text.style.size - STICKER_SIZE).abs() < 1e-5);
    // No outline: it would trace the character's box, not the picture in it.
    assert!(text.style.stroke.is_none());
    assert!(editor.is_sticker(clip));
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert!(editor.text_clip(clip).is_none());
}

/// Two stickers in a row do not land on top of each other.
#[test]
fn a_second_sticker_goes_after_the_first() {
    let (mut editor, _events) = Editor::new_project("Stickers");
    let first = editor.add_sticker("❤").unwrap();
    let second = editor.add_sticker("☀").unwrap();

    let (a, b) = (
        editor.text_clip(first).unwrap().timeline,
        editor.text_clip(second).unwrap().timeline,
    );
    assert!(b.start >= a.end, "{a:?} then {b:?}");
}

/// A title is not a sticker, whatever the inspector might like to call it.
#[test]
fn words_are_not_stickers() {
    let (mut editor, _events) = Editor::new_project("Stickers");
    let title = editor.add_text("Hello").unwrap();
    assert!(!editor.is_sticker(title));
}

#[test]
fn a_sticker_needs_a_character() {
    let (mut editor, _events) = Editor::new_project("Stickers");
    assert!(matches!(
        editor.add_sticker("  "),
        Err(EditorError::NoSticker)
    ));
}

/// The picker's list is what it says it is: no duplicates, and every entry has
/// something to draw and something to call it.
#[test]
fn the_sticker_list_is_sound() {
    for (sticker, name) in STICKERS {
        assert!(!sticker.trim().is_empty(), "a sticker with no character");
        assert!(!name.trim().is_empty(), "{sticker} has no name");
    }
    for (index, (sticker, _)) in STICKERS.iter().enumerate() {
        assert!(
            !STICKERS[index + 1..]
                .iter()
                .any(|(other, _)| other == sticker),
            "{sticker} is in the list twice"
        );
    }
}
