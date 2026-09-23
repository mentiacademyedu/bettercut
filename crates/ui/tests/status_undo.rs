//! The status bar offers Undo only beside a message an edit made.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_ui::UiState;
use egui::{Pos2, RawInput, Rect, vec2};

fn words(editor: &Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 200.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::panels::status_bar(ui, editor, state)
    });
    output.textures_delta.clear();
    let mut words = String::new();
    for clipped in &output.shapes {
        if let egui::Shape::Text(text) = &clipped.shape {
            words.push_str(text.galley.text());
            words.push(' ');
        }
    }
    words
}

#[test]
fn undo_is_offered_after_an_edit_and_not_after_a_toggle() {
    let (mut editor, _events) = Editor::new_project("Status");
    let mut state = UiState::default();
    words(&editor, &mut state);

    // An edit and its message in the same frame.
    editor
        .add_markers(&[TimelineTime::from_seconds(1)])
        .unwrap();
    state.info("Added 1 marker");
    assert!(
        words(&editor, &mut state).contains("Undo"),
        "no Undo after an edit"
    );

    // A message with no new step: no Undo.
    state.info("Snapping on");
    assert!(
        !words(&editor, &mut state).contains("Undo"),
        "Undo offered after a toggle"
    );
}
