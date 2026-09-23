//! The export queue: what is being written, what is waiting, and a way to take
//! any of it back.
//!
//! Exporting every shape or every sequence in one press queues several files,
//! and a queue nobody can see is a queue nobody can fix — the wrong setting on
//! file one of six should not mean waiting through the other five. The job
//! manager belongs to the desktop shell, so the shell copies what is queued
//! into the interface state each frame and carries the requests back.

use crate::state::UiState;
use crate::theme;

/// What the window shows and asks for.
#[derive(Debug, Default)]
pub struct ExportQueueState {
    pub open: bool,
    /// The export being written, as its status line reads.
    pub running: Option<String>,
    /// The exports waiting behind it, in order.
    pub waiting: Vec<String>,
    /// A waiting export to take off the queue, by position.
    pub remove: Option<usize>,
    /// Take every waiting export off the queue.
    pub clear: bool,
    /// Move the waiting export at `.0` so it sits at `.1`, the others
    /// closing up around it.
    pub move_to: Option<(usize, usize)>,
}

/// Draw the window, if it is open.
pub fn show(ctx: &egui::Context, state: &mut UiState) {
    if !state.export_queue.open {
        return;
    }
    let mut open = true;
    let mut stop = false;
    egui::Window::new("Exports")
        .open(&mut open)
        .default_width(320.0)
        .show(ctx, |ui| {
            let queue = &mut state.export_queue;
            match &queue.running {
                Some(label) => {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(label).strong());
                        if ui
                            .small_button("Stop")
                            .on_hover_text("Cancel this export. The partial file is removed.")
                            .clicked()
                        {
                            stop = true;
                        }
                    });
                }
                None => {
                    ui.label(
                        egui::RichText::new("Nothing is exporting")
                            .small()
                            .color(theme::disabled()),
                    );
                }
            }
            if queue.waiting.is_empty() {
                return;
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("{} waiting", queue.waiting.len()))
                    .small()
                    .color(theme::disabled()),
            );
            let last = queue.waiting.len() - 1;
            let mut move_to = None;
            for (index, label) in queue.waiting.iter().enumerate() {
                ui.horizontal(|ui| {
                    // Up and down, one place at a time, so the order is
                    // always visible while it changes; the top one is next.
                    if ui
                        .add_enabled(index > 0, egui::Button::new("Up").small())
                        .on_hover_text("Run this export sooner")
                        .clicked()
                    {
                        move_to = Some((index, index - 1));
                    }
                    if ui
                        .add_enabled(index < last, egui::Button::new("Down").small())
                        .on_hover_text("Run this export later")
                        .clicked()
                    {
                        move_to = Some((index, index + 1));
                    }
                    ui.label(label);
                    if ui
                        .small_button("Remove")
                        .on_hover_text("Take this export off the queue")
                        .clicked()
                    {
                        queue.remove = Some(index);
                    }
                });
            }
            if move_to.is_some() {
                queue.move_to = move_to;
            }
            if queue.waiting.len() > 1 && ui.button("Remove all waiting").clicked() {
                queue.clear = true;
            }
        });
    if stop {
        state.export_stop_requested = true;
    }
    state.export_queue.open = open;
}
