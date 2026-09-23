//! "Save changes first?" — asked when New, Open, Open Recent or Try a Sample
//! would replace a project with unsaved work, instead of refusing outright.
//!
//! Save saves (asking where, for a project never saved) and then goes on;
//! Don't Save goes on and the changes are dropped; Cancel stays put.

use bettercut_editor_core::Editor;

use crate::state::UiState;

/// What was asked for when the prompt came up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Switch {
    New,
    /// Open, through the file dialog.
    Open,
    /// Open this project, from the recent list.
    OpenPath(std::path::PathBuf),
    Sample,
    /// Close the window.
    Quit,
}

/// Do what was asked for, whatever there is unsaved.
pub fn proceed(editor: &mut Editor, state: &mut UiState, switch: Switch) {
    state.discard_ok = true;
    match switch {
        Switch::New => crate::panels::new_project(editor, state),
        Switch::Open => crate::panels::open_project(editor, state),
        Switch::OpenPath(path) => crate::panels::open_project_at(editor, state, &path),
        Switch::Sample => crate::sample::open(editor, state),
        // The window closes itself: the app sees this and closes for real.
        Switch::Quit => state.quit_now = true,
    }
    state.discard_ok = false;
}

/// Draw the prompt, if something is waiting on it.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    let Some(switch) = state.pending_switch.clone() else {
        return;
    };
    let mut answer: Option<bool> = None; // Some(save first?)
    let mut cancel = false;
    egui::Window::new("Save changes?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(format!(
                "“{}” has changes that are not saved.",
                editor.project().name
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    answer = Some(true);
                }
                if ui.button("Don't Save").clicked() {
                    answer = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    if cancel {
        state.pending_switch = None;
        state.needs_repaint = true;
        return;
    }
    if let Some(save_first) = answer {
        state.pending_switch = None;
        if save_first {
            crate::panels::save_project(editor, state);
            // A save dialog closed without a name leaves the work unsaved:
            // then nothing is replaced.
            if editor.is_dirty() {
                return;
            }
        }
        proceed(editor, state, switch);
        state.needs_repaint = true;
    }
}
