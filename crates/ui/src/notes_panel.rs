//! The Notes window: free text about the edit, kept in the project file.
//!
//! What is left to do, what the client asked for, which take was the good
//! one — the things that otherwise live on a sticky note beside the screen.
//! Typed into a draft and applied when the window loses the keyboard or is
//! closed, so a paragraph is one undo step rather than one per letter.

use bettercut_editor_core::Editor;

use crate::state::UiState;
use crate::theme;

/// Draw the window, if it is open, and apply the draft when it is left.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.notes_open {
        // Closing applies whatever was typed.
        if let Some(draft) = state.notes_draft.take() {
            apply(editor, state, &draft);
        }
        return;
    }
    let mut open = true;
    let mut left = false;
    egui::Window::new("Notes")
        .open(&mut open)
        .resizable(true)
        .default_size([320.0, 280.0])
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "Saved with the project. To-dos, the client's asks, which take was good.",
                )
                .small()
                .color(theme::disabled()),
            );
            let draft = state
                .notes_draft
                .get_or_insert_with(|| editor.project_notes().to_owned());
            let field = egui::ScrollArea::vertical()
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(draft)
                            .desired_width(f32::INFINITY)
                            .desired_rows(12)
                            .hint_text("Nothing yet. Type what is left to do."),
                    )
                })
                .inner;
            left = field.lost_focus();
        });
    if !open {
        state.notes_open = false;
        left = true;
    }
    if left && let Some(draft) = state.notes_draft.take() {
        apply(editor, state, &draft);
    }
}

fn apply(editor: &mut Editor, state: &mut UiState, draft: &str) {
    match editor.set_project_notes(draft) {
        Ok(()) => state.needs_repaint = true,
        Err(err) => state.error(err.to_string()),
    }
}
