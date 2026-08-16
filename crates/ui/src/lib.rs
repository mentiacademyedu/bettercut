//! egui widgets and the canvas timeline (§53, §58).
//!
//! Top of §86's stack: depends on `editor-core`, and nothing depends on it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod context_menu;
pub mod export_dialog;
pub mod media_jobs;
pub mod panels;
pub mod preview;
pub mod preview_overlay;
pub mod shortcuts;
pub mod state;
pub mod theme;
pub mod thumbnails;
pub mod timeline;
pub mod waveforms;

pub use media_jobs::{MediaJobs, MediaUpdate};
pub use preview::Preview;
pub use state::UiState;

use bettercut_editor_core::{Editor, Event};

/// Draw one frame of the whole application (§58).
///
/// ```text
/// ┌────────────────────────────────────────────────────┐
/// │ Toolbar                                            │
/// ├──────────────┬──────────────────────┬──────────────┤
/// │ Media        │       Preview        │ Inspector    │
/// ├──────────────┴──────────────────────┴──────────────┤
/// │            Timeline (Painter canvas)               │
/// ├────────────────────────────────────────────────────┤
/// │ Status                                             │
/// └────────────────────────────────────────────────────┘
/// ```
pub fn draw(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    preview: Option<&mut Preview>,
) {
    // The recovery prompt is modal and must be answered before anything else
    // (§39): editing first would mean recovering over the top of new work.
    if state.pending_recovery.is_some() {
        panels::recovery_prompt(ui.ctx(), editor, state);
    }

    // Transport keys are handled first so play/pause feels immediate.
    let mut preview = preview;
    shortcuts::handle(ui.ctx(), editor, state, preview.as_deref_mut());

    egui::Panel::top("toolbar").show(ui, |ui| {
        panels::toolbar(ui, editor, state, preview.as_deref_mut());
    });

    egui::Panel::bottom("status")
        .resizable(false)
        .show(ui, |ui| {
            panels::status_bar(ui, editor, state);
        });

    egui::Panel::bottom("timeline")
        .resizable(true)
        .default_size(320.0)
        .min_size(140.0)
        .show(ui, |ui| {
            panels::transport(ui, editor, state, preview.as_deref_mut());
            ui.separator();
            timeline::draw(ui, editor, state);
        });

    egui::Panel::left("media")
        .resizable(true)
        .default_size(240.0)
        .min_size(160.0)
        .show(ui, |ui| {
            panels::media_browser(ui, editor, state);
        });

    egui::Panel::right("inspector")
        .resizable(true)
        // Wide enough that a slider and its label, the keyframe button and the
        // reset button all fit on one line without the slider collapsing to a
        // stub. At 290 the controls were technically present and practically
        // unusable.
        .default_size(420.0)
        .min_size(300.0)
        .show(ui, |ui| {
            panels::inspector(ui, editor, state);
        });

    egui::CentralPanel::default().show(ui, |ui| {
        panels::preview(ui, editor, state, preview.as_deref());
    });
}

/// Fold the frame's events into view state (§56).
///
/// Called once per frame with everything the core queued. Most events only need
/// to force a repaint; a few carry a message worth showing.
///
/// Takes `&Editor` so an event carrying only an ID can be reported in terms the
/// user recognises — a file name, not a UUID.
pub fn consume_events(events: Vec<Event>, editor: &Editor, state: &mut UiState) {
    let mut missing: Vec<String> = Vec::new();

    for event in events {
        match event {
            // A repaint alone is not enough for these. The preview only
            // re-composites when the playhead moves, so an edit that changes
            // how a clip *looks* — opacity, scale, colour, a keyframe, a track
            // hidden — would repaint the panels around a stale picture.
            Event::ProjectChanged | Event::ProjectLoaded | Event::ActiveSequenceChanged(_) => {
                state.needs_repaint = true;
                state.preview_is_stale = true;
            }

            Event::PlaybackPositionChanged(_) | Event::PlaybackStateChanged { .. } => {
                state.needs_repaint = true;
            }

            Event::ProjectSaved => state.info("Project saved"),
            Event::MediaImported(_) => state.needs_repaint = true,
            Event::MediaMissing(id) => {
                missing.push(
                    editor
                        .project()
                        .media_asset(id)
                        .map_or_else(|| id.short(), |m| m.file_name.clone()),
                );
            }
            Event::ProxyReady(_) => state.needs_repaint = true,

            Event::JobStarted { label, .. } => state.info(label),
            Event::JobProgress { .. } => state.needs_repaint = true,
            Event::JobFinished { .. } => state.needs_repaint = true,
            Event::JobFailed { message, .. } => state.error(message),
        }
    }

    // One message for the whole batch: opening a project with 40 relinked files
    // should not overwrite the status bar 40 times (§66).
    match missing.len() {
        0 => {}
        1 => state.error(format!("Missing from disk: {}", missing[0])),
        n => state.error(format!(
            "{n} media files are missing from disk, including {}",
            missing[0]
        )),
    }
}
