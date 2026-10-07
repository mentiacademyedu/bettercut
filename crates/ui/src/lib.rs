//! egui widgets and the canvas timeline (§53, §58).
//!
//! Top of §86's stack: depends on `editor-core`, and nothing depends on it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod assistant;
pub mod bounce;
pub mod bug_report;
pub mod caption_list;
pub mod context_menu;
pub mod crash;
pub mod disk_change;
pub mod effects;
pub mod export_dialog;
pub mod export_presets;
pub mod export_queue;
pub mod file_details;
pub mod file_drop;
pub mod highlight_dialog;
pub mod history_panel;
pub mod icon;
pub mod keymap;
pub mod keys;
pub mod library;
pub mod looks;
pub mod loudness;
pub mod marker_list;
pub mod media_jobs;
pub mod motion_path;
pub mod notes_panel;
pub mod palette;
pub mod panels;
pub mod prefs;
pub mod preview;
pub mod preview_overlay;
pub mod recent;
pub mod render;
pub mod reveal;
pub mod sample;
pub mod save_prompt;
pub mod scene_dialog;
pub mod scopes;
pub mod shortcuts;
pub mod shuttle;
pub mod silence_dialog;
pub mod speech;
pub mod state;
pub mod storyboard;
pub mod swatches;
pub mod template_dialog;
pub mod theme;
pub mod thumbnails;
pub mod timecode_entry;
pub mod timeline;
pub mod title_styles;
pub mod tracking;
pub mod trim_view;
pub mod updates;
pub mod version_changes;
pub mod voiceover;
pub mod waveform_view;
pub mod waveforms;
pub mod welcome;
pub mod whats_new;
pub mod wheels;

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
/// A window that can be waiting when the app starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupWindow {
    Recovery,
    CrashReport,
    Welcome,
    WhatsNew,
}

/// Which start-up window to show now: one at a time, the most urgent first —
/// getting work back, then what went wrong last time, then the greetings.
/// Each centres itself, so two at once covered each other's words; the
/// others wait until the one before is answered.
pub fn startup_window(state: &UiState) -> Option<StartupWindow> {
    if state.pending_recovery.is_some() {
        Some(StartupWindow::Recovery)
    } else if state.crash_report.is_some() {
        Some(StartupWindow::CrashReport)
    } else if state.welcome_open {
        Some(StartupWindow::Welcome)
    } else if state.whats_new_open {
        Some(StartupWindow::WhatsNew)
    } else {
        None
    }
}

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

    // Full screen: the window goes full screen and the picture is all there is.
    if state.fullscreen != state.fullscreen_applied {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(state.fullscreen));
        state.fullscreen_applied = state.fullscreen;
    }
    if state.fullscreen {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ui, |ui| {
                panels::fullscreen_preview(ui, editor, state, preview.as_deref());
            });
        return;
    }

    egui::Panel::top("toolbar").show(ui, |ui| {
        panels::toolbar(ui, editor, state, preview.as_deref_mut());
    });

    egui::Panel::bottom("status")
        .resizable(false)
        .show(ui, |ui| {
            panels::status_bar(ui, editor, state);
            if std::mem::take(&mut state.undo_request) {
                match editor.undo() {
                    Ok(()) => {
                        state.status = None;
                        state.needs_repaint = true;
                    }
                    Err(err) => state.error(err.to_string()),
                }
            }
        });

    // Two fifths of the window to start with, at most the 320 it always had:
    // a fixed 320 left a small window's preview a postage stamp.
    let timeline_height = (ui.ctx().content_rect().height() * 0.4).clamp(140.0, 320.0);
    let timeline_rect = egui::Panel::bottom("timeline")
        .resizable(true)
        .default_size(timeline_height)
        .min_size(140.0)
        .show(ui, |ui| {
            panels::sequence_tabs(ui, editor, state);
            panels::transport(ui, editor, state, preview.as_deref_mut());
            ui.separator();
            timeline::draw(ui, editor, state);
        })
        .response
        .rect;

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
        // And no wider than this unless dragged: the picture is what the
        // screen is for, and a long row of choices wraps rather than pushing
        // the preview into a strip.
        .max_size(460.0)
        .show(ui, |ui| {
            panels::inspector(ui, editor, state);
        });

    egui::CentralPanel::default().show(ui, |ui| {
        panels::preview(ui, editor, state, preview.as_deref());
    });

    template_dialog::show(ui.ctx(), editor, state);
    shortcuts::help_window(ui.ctx(), state);
    silence_dialog::show(ui.ctx(), editor, state);
    highlight_dialog::show(ui.ctx(), editor, state);
    scene_dialog::show(ui.ctx(), editor, state);
    palette::show(ui.ctx(), editor, state);
    match startup_window(state) {
        Some(StartupWindow::CrashReport) => crash::show(ui.ctx(), state),
        Some(StartupWindow::Welcome) => welcome::show(ui.ctx(), editor, state),
        Some(StartupWindow::WhatsNew) => whats_new::show(ui.ctx(), state),
        // The recovery prompt, drawn above; or nothing waiting.
        Some(StartupWindow::Recovery) | None => {}
    }
    save_prompt::show(ui.ctx(), editor, state);
    // Saved by something else — an assistant through bettercut-mcp, say.
    disk_change::check(editor, state);
    disk_change::show(ui.ctx(), editor, state);
    assistant::show(ui.ctx(), state);
    // A file on its way from the media panel: its name follows the pointer,
    // and a release anywhere but the timeline (which handles its own) drops
    // the drag.
    if let Some(media) = state.dragging_media {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.pointer.any_released()) {
            state.dragging_media = None;
        } else if let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) {
            let name = editor
                .project()
                .media_asset(media)
                .map_or_else(String::new, |m| m.display_name().to_owned());
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("media drag"),
            ));
            let text = painter.layout_no_wrap(
                name,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
            let chip = egui::Rect::from_min_size(
                pointer + egui::vec2(14.0, 10.0),
                text.size() + egui::vec2(16.0, 8.0),
            );
            painter.rect_filled(chip, 6.0, theme::accent());
            painter.galley(chip.min + egui::vec2(8.0, 4.0), text, egui::Color32::WHITE);
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            ctx.request_repaint();
        }
    }

    // Text an action asked to have put on the clipboard.
    if let Some(text) = state.copy_out.take() {
        ui.ctx().copy_text(text);
    }
    if let Some(url) = state.open_url.take() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
    caption_list::show(ui.ctx(), editor, state);
    marker_list::show(ui.ctx(), editor, state);
    export_queue::show(ui.ctx(), state);
    scopes::show(ui.ctx(), editor, state);
    waveform_view::show(ui.ctx(), editor, state);
    storyboard::show(ui.ctx(), editor, state);
    trim_view::show(ui.ctx(), editor, state);
    history_panel::show(ui.ctx(), editor, state);
    notes_panel::show(ui.ctx(), editor, state);
    version_changes::show(ui.ctx(), state);
    file_drop::handle(ui.ctx(), editor, state, timeline_rect);
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
