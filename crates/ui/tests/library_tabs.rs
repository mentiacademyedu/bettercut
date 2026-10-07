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

#[test]
fn a_title_design_is_one_step_styled_and_placed() {
    use bettercut_editor_core::text::{TextStyle, TitleLook};
    let (mut editor, _events) = Editor::new_project("Designs");
    let mut state = UiState::default();
    let depth = editor.undo_depth();

    bettercut_ui::library::add_title_design(&mut editor, &mut state, TitleLook::LowerThird);
    assert_eq!(editor.undo_depth(), depth + 1, "one undo step");
    let clip = *state.selected_clips.iter().next().unwrap();
    let title = editor.text_clip(clip).unwrap();
    assert_eq!(title.style, TextStyle::title(TitleLook::LowerThird));
    let (x, y) = TitleLook::LowerThird.anchor();
    assert!((title.transform.position.x - x).abs() < 1e-6);
    assert!((title.transform.position.y - y).abs() < 1e-6);
}

#[test]
fn every_animated_title_is_one_step_and_moves_as_it_says() {
    use bettercut_ui::library::{ANIMATED_TEXT, add_animated_text};
    let (mut editor, _events) = Editor::new_project("Animated");
    let mut state = UiState::default();
    for which in &ANIMATED_TEXT {
        let depth = editor.undo_depth();
        add_animated_text(&mut editor, &mut state, which);
        assert_eq!(editor.undo_depth(), depth + 1, "{} is one step", which.name);
        let clip = *state.selected_clips.iter().next().unwrap();
        let title = editor.text_clip(clip).unwrap();
        assert_eq!(
            title.animation.intro.map(|m| m.kind),
            which.intro.map(|(kind, _)| kind),
            "{}",
            which.name
        );
        assert_eq!(
            title.animation.outro.map(|m| m.kind),
            which.outro.map(|(kind, _)| kind)
        );
        assert_eq!(title.animation.looping, which.looping, "{}", which.name);
        if let Some(preset) = which.preset {
            assert_eq!(TextPreset::of(&title.style), Some(preset), "{}", which.name);
        }
        // Clear the playhead's spot so the next title has room of its own.
        editor.set_playhead(title.timeline.end);
    }
}
