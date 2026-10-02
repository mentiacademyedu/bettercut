//! The project file changed on disk while it was open — saved by an assistant
//! through `bettercut-mcp`, a sync folder, or another copy of bettercut.
//!
//! Checked once a second, by the file's modification time against the one
//! the editor last wrote or read (`Editor::changed_on_disk`), which costs one
//! metadata read. When it changes, a small window offers to load the new
//! version or keep the one on screen; the app's own saves never raise it.

use std::time::{Duration, Instant};

use bettercut_editor_core::Editor;

use crate::state::UiState;
use crate::theme;

/// How often the file is looked at.
const EVERY: Duration = Duration::from_secs(1);

/// Look at the file if it is time to, and note a change for [`show`].
pub fn check(editor: &Editor, state: &mut UiState) {
    let due = state.disk_checked.is_none_or(|at| at.elapsed() >= EVERY);
    if !due || state.disk_changed {
        return;
    }
    state.disk_checked = Some(Instant::now());
    if editor.changed_on_disk() {
        state.disk_changed = true;
        state.needs_repaint = true;
    }
}

/// The window, while a change is waiting for an answer.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.disk_changed {
        return;
    }
    let Some(path) = editor.path().map(std::path::Path::to_path_buf) else {
        state.disk_changed = false;
        return;
    };
    let mut reload = false;
    let mut keep = false;
    egui::Window::new("The project changed on disk")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(420.0, 0.0))
        .show(ctx, |ui| {
            ui.label(format!(
                "{} was saved by another program — an assistant using bettercut-mcp, \
                 perhaps, or a sync folder.",
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned()
                )
            ));
            if editor.is_dirty() {
                ui.label(
                    egui::RichText::new(
                        "You have unsaved changes here: loading the new version discards them.",
                    )
                    .color(theme::error_text()),
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add(theme::primary_button("Load the New Version"))
                    .clicked()
                {
                    reload = true;
                }
                if ui
                    .button("Keep Mine")
                    .on_hover_text("Carry on with the project as it is on screen")
                    .clicked()
                {
                    keep = true;
                }
            });
        });
    if reload {
        state.disk_changed = false;
        state.discard_ok = true;
        crate::panels::open_project_at(editor, state, &path);
        state.discard_ok = false;
    } else if keep {
        editor.accept_disk_state();
        state.disk_changed = false;
        state.needs_repaint = true;
    }
}
