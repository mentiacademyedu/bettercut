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
                            .color(theme::DISABLED),
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
                    .color(theme::DISABLED),
            );
            for (index, label) in queue.waiting.iter().enumerate() {
                ui.horizontal(|ui| {
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
            if queue.waiting.len() > 1 && ui.button("Remove all waiting").clicked() {
                queue.clear = true;
            }
        });
    if stop {
        state.export_stop_requested = true;
    }
    state.export_queue.open = open;
}
