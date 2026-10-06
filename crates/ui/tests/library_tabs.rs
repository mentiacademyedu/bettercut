//! The left panel's Text tab: a title in a style, added as one undo step and
//! selected, ready to type into.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::text::TextPreset;
use bettercut_ui::UiState;
use bettercut_ui::library::add_styled_text;

#[test]
fn a_styled_title_is_one_step_and_wears_its_style() {
    let (mut editor, _events) = Editor::new_project("Library");
    let mut state = UiState::default();
    let depth = editor.undo_depth();

    add_styled_text(&mut editor, &mut state, Some(TextPreset::Highlight));
    assert_eq!(editor.undo_depth(), depth + 1, "one undo step");
    assert_eq!(editor.undo_label().as_deref(), Some("Add Highlight Text"));
    let clip = *state
        .selected_clips
        .iter()
        .next()
        .expect("the new title is selected");
    let style = &editor.text_clip(clip).expect("a title").style;
    assert_eq!(TextPreset::of(style), Some(TextPreset::Highlight));

    editor.undo().unwrap();
    assert!(editor.text_clip(clip).is_none(), "one undo takes it away");

    // Plain: the default look, still one step.
    add_styled_text(&mut editor, &mut state, None);
    assert_eq!(editor.undo_depth(), depth + 1);
}
