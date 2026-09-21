//! Full screen preview: F in, F or Escape out, and the window told once.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_ui::UiState;
use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, ViewportCommand, vec2};

/// One frame of the whole interface with `keys` pressed; returns whether the
/// window was asked to go full screen (`Some(on)`) that frame.
fn frame(editor: &mut Editor, state: &mut UiState, keys: &[Key]) -> Option<bool> {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
        events: keys
            .iter()
            .map(|key| Event::Key {
                key: *key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            })
            .collect(),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::draw(ui, editor, state, None);
    });
    output.textures_delta.clear();
    output
        .viewport_output
        .values()
        .flat_map(|viewport| viewport.commands.iter())
        .find_map(|command| match command {
            ViewportCommand::Fullscreen(on) => Some(*on),
            _ => None,
        })
}

#[test]
fn f_goes_full_screen_and_escape_comes_back() {
    let (mut editor, _events) = Editor::new_project("Full");
    let mut state = UiState::default();

    assert_eq!(frame(&mut editor, &mut state, &[]), None);
    assert_eq!(frame(&mut editor, &mut state, &[Key::F]), Some(true));
    assert!(state.fullscreen);
    // Asked once, not every frame.
    assert_eq!(frame(&mut editor, &mut state, &[]), None);

    assert_eq!(frame(&mut editor, &mut state, &[Key::Escape]), Some(false));
    assert!(!state.fullscreen);

    frame(&mut editor, &mut state, &[Key::F]);
    assert_eq!(frame(&mut editor, &mut state, &[Key::F]), Some(false));
}

/// Escape leaves full screen before it puts down a tool, so the way out is
/// always the first thing it does.
#[test]
fn escape_leaves_full_screen_before_anything_else() {
    let (mut editor, _events) = Editor::new_project("Full");
    let mut state = UiState::default();
    state.fullscreen = true;
    frame(&mut editor, &mut state, &[]);
    frame(&mut editor, &mut state, &[Key::Escape]);
    assert!(!state.fullscreen);
}
