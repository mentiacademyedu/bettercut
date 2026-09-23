//! Files dragged in from the desktop (§58).
//!
//! Dropping a file on the window is the first thing many people try, and an
//! editor that ignores it looks broken. Anywhere on the window imports; onto
//! the timeline also puts the clips there, one after another at the end — the
//! same as pressing "Add to timeline" on each, in the order they were dropped.
//!
//! While files are held over the window an overlay says which of the two will
//! happen, so the choice of where to let go is an informed one.

use bettercut_editor_core::Editor;

use crate::state::UiState;

/// Import whatever was dropped this frame, and draw the overlay while files
/// are held over the window.
///
/// `timeline` is where the timeline panel is on screen this frame.
pub fn handle(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState, timeline: egui::Rect) {
    let (hovering, dropped, pointer) = ctx.input(|i| {
        let dropped: Vec<std::path::PathBuf> = i
            .raw
            .dropped_files
            .iter()
            .map(|file| file.path().to_path_buf())
            .filter(|path| !path.as_os_str().is_empty())
            .collect();
        (
            !i.raw.hovered_files.is_empty(),
            dropped,
            i.pointer.latest_pos(),
        )
    });
    let over_timeline = pointer.is_some_and(|p| timeline.contains(p));

    if hovering {
        overlay(ctx, timeline, over_timeline);
    }
    if dropped.is_empty() {
        return;
    }

    let imported = crate::panels::import_paths(editor, state, &dropped);
    if over_timeline && !imported.is_empty() {
        place_all(editor, state, &imported);
    }
    state.needs_repaint = true;
}

/// Put each imported file at the end of the timeline, in order.
///
/// One `place_media` per file, which is one undo step per file — each is a
/// clip the user can take back on its own.
pub fn place_all(
    editor: &mut Editor,
    state: &mut UiState,
    media: &[bettercut_editor_core::foundation::MediaId],
) {
    let mut placed = 0;
    let mut failed = Vec::new();
    for id in media {
        match editor.place_media(*id) {
            Ok(_) => placed += 1,
            Err(err) => failed.push(err.to_string()),
        }
    }
    if failed.is_empty() {
        state.info(format!("Added {placed} clip(s) to the timeline"));
    } else {
        state.error(format!(
            "Added {placed} clip(s); {} could not be placed: {}",
            failed.len(),
            failed[0]
        ));
    }
}

fn overlay(ctx: &egui::Context, timeline: egui::Rect, over_timeline: bool) {
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("file drop overlay"),
    ));
    painter.rect_filled(screen, 0, egui::Color32::from_black_alpha(140));

    // The timeline lights up as the other place to let go, brighter when the
    // pointer is over it.
    let accent = crate::theme::selection();
    painter.rect_stroke(
        timeline.shrink(4.0),
        6,
        egui::Stroke::new(
            if over_timeline { 3.0 } else { 1.5 },
            if over_timeline {
                accent
            } else {
                accent.gamma_multiply(0.5)
            },
        ),
        egui::StrokeKind::Inside,
    );

    let message = if over_timeline {
        "Drop to import and add to the end of the timeline"
    } else {
        "Drop to import  ·  drop on the timeline to add it there too"
    };
    painter.text(
        screen.center(),
        egui::Align2::CENTER_CENTER,
        message,
        egui::FontId::proportional(22.0),
        egui::Color32::WHITE,
    );
}
