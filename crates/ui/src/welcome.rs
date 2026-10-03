//! The welcome window, on first run: four steps from nothing to an exported
//! video, and where everything else is.
//!
//! Someone opening an editor for the first time wants to have made something
//! in five minutes, not to learn the panels. So this says the least that gets
//! there — bring files in, cut, find anything, export — and gets out of the
//! way. "Don't show this again" is remembered in the interface prefs; the
//! palette's "Welcome" brings it back.

use bettercut_editor_core::Editor;

use crate::state::UiState;
use crate::theme;

/// The steps, in order: a short title and one line of how.
pub const STEPS: [(&str, &str); 4] = [
    (
        "Bring in your files",
        "Drag videos, photos and music onto the window, or use Import in Media.",
    ),
    (
        "Cut",
        "Press S to split at the playhead, Delete to remove a piece. Drag clips to move them.",
    ),
    (
        "Find anything",
        "Press Ctrl+K and type what you want to do — split, speed, captions, transitions.",
    ),
    (
        "Export",
        "Choose Export at the top, pick a size, and your video is written to a file.",
    ),
];

/// Draw the window, if it is open.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.welcome_open {
        return;
    }
    let mut open = true;
    let mut dismiss_for_good = false;
    let mut close = false;
    let mut sample = false;
    egui::Window::new("Welcome to bettercut")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(440.0, 0.0))
        .show(ctx, |ui| {
            ui.label("Four steps from nothing to a finished video:");
            ui.add_space(6.0);
            for (number, (title, how)) in STEPS.iter().enumerate() {
                ui.horizontal_top(|ui| {
                    ui.label(egui::RichText::new(format!("{}", number + 1)).strong().size(18.0));
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(*title).strong());
                        ui.label(egui::RichText::new(crate::keys::keys(how)).small());
                    });
                });
                ui.add_space(4.0);
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(crate::keys::keys("Right-click anything for what can be done to it. Everything can be undone with Ctrl+Z."))
                    .small()
                    .color(theme::disabled()),
            );
            // A beta says so up front, with where to send what breaks.
            ui.label(
                egui::RichText::new(crate::keys::keys(
                    "This is a beta: save often, and when something breaks, Ctrl+K then \
                     \u{201c}Report a Bug on GitHub\u{201d} tells us about it.",
                ))
                .small()
                .color(theme::accent_text()),
            );
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Start editing").clicked() {
                    close = true;
                }
                if ui
                    .button("Try a sample")
                    .on_hover_text("A short edit made from colours and titles, to practise on")
                    .clicked()
                {
                    sample = true;
                }
                if ui
                    .button("Edit with an AI Assistant")
                    .on_hover_text("Connect Claude or another assistant to edit with you")
                    .clicked()
                {
                    state.assistant_open = true;
                    close = true;
                }
                if ui
                    .button("Don't show this again")
                    .on_hover_text(crate::keys::keys("Ctrl+K, then Welcome, brings it back"))
                    .clicked()
                {
                    dismiss_for_good = true;
                }
            });
        });
    if sample {
        crate::sample::open(editor, state);
    }
    if dismiss_for_good {
        state.prefs.seen_welcome = true;
        let _ = state.prefs.save();
        close = true;
    }
    if close || !open {
        state.welcome_open = false;
        state.needs_repaint = true;
    }
}
