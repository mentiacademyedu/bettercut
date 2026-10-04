//! What's new, once after an update: a short list of what a tester can now
//! do that they could not last time, so the new things are found.
//!
//! Shown when the build is newer than the one that last ran — never on a
//! first run, where the welcome window has the floor. The palette's
//! "What's New" opens it again.

use crate::state::UiState;
use crate::theme;

/// This build's version, as the prefs remember it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What is new in this build, most useful first.
pub const CHANGES: &[&str] = &[
    "Assistants can set a clip's EQ, voice and echo, the whole video's bars and progress bar, and each lane",
    "When an assistant deletes a shot, its sound goes with it",
];

/// Whether to show the window at start: an earlier build ran here before.
pub fn should_show(last_version: &str, seen_welcome: bool) -> bool {
    seen_welcome && !last_version.is_empty() && last_version != VERSION
}

/// Draw the window, if it is open.
pub fn show(ctx: &egui::Context, state: &mut UiState) {
    if !state.whats_new_open {
        return;
    }
    let mut open = true;
    let mut close = false;
    egui::Window::new(format!("What's new in {VERSION}"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(460.0, 0.0))
        .show(ctx, |ui| {
            for change in CHANGES {
                ui.horizontal_wrapped(|ui| {
                    ui.label("•");
                    ui.label(crate::keys::keys(change));
                });
            }
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(crate::keys::keys(
                    "Ctrl+K, then What's New, shows this again.",
                ))
                .small()
                .color(theme::disabled()),
            );
            if ui.button("Got it").clicked() {
                close = true;
            }
        });
    if close || !open {
        state.whats_new_open = false;
        state.needs_repaint = true;
    }
}
