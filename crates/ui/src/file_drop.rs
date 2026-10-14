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
    #[cfg(target_arch = "wasm32")]
    {
        browser(ctx, editor, state, timeline);
    }
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

/// The web build: files dropped on the page or picked arrive once the
/// browser has opened them (`bettercut_media::browser`), not through egui —
/// which would copy every byte of them into memory first.
#[cfg(target_arch = "wasm32")]
fn browser(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState, timeline: egui::Rect) {
    use bettercut_editor_core::media::browser;
    // Files picked for what is in them, not as media.
    for (purpose, name, bytes) in browser::take_given() {
        match purpose.as_str() {
            "captions" => match editor.import_caption_bytes(bytes, &name) {
                Ok(count) => state.info(format!("Imported {count} captions")),
                Err(err) => state.error(format!("Could not import captions: {err}")),
            },
            "lut" => import_lut(editor, state, &name, &bytes),
            // Kept by the page among the user's fonts, then loaded as the
            // desktop loads one it has copied there.
            "font" => {
                let folder = bettercut_editor_core::text::user_fonts_dir();
                let path = folder.join(name.replace(['/', '\\'], "-"));
                match bettercut_editor_core::foundation::files::write(&path, &bytes) {
                    Ok(()) => state.font_import = Some(path),
                    Err(err) => state.error(format!("Could not keep the font: {err}")),
                }
            }
            "markers" => match editor.import_marker_text(&String::from_utf8_lossy(&bytes)) {
                Ok(0) => state.info(
                    "No markers found in that file — it needs a timecode or seconds column",
                ),
                Ok(count) => state.info(format!("Added {count} markers")),
                Err(err) => state.error(format!("Could not read the markers: {err}")),
            },
            _ => {}
        }
        state.needs_repaint = true;
    }
    for path in browser::take_opened() {
        crate::panels::open_project_at(editor, state, &path);
        state.needs_repaint = true;
    }
    let mut ready = browser::take_ready();
    // A file the project is missing, given again: back in its place rather
    // than imported a second time.
    let mut found = 0;
    ready.retain(|path| {
        let missing = editor
            .project()
            .media
            .iter()
            .find(|asset| asset.missing && asset.matches_relink_candidate(path))
            .map(|asset| asset.id);
        let relinked = missing.is_some_and(|id| editor.relink_media(id, path).is_ok());
        found += usize::from(relinked);
        !relinked
    });
    if found > 0 {
        state.info(format!("Found {found} missing file(s)"));
        state.needs_repaint = true;
    }
    if ready.is_empty() {
        return;
    }
    // Page pixels to the interface's points.
    let zoom = ctx.zoom_factor();
    let over_timeline = browser::drop_point()
        .is_some_and(|(x, y)| timeline.contains(egui::pos2(x / zoom, y / zoom)));
    let imported = crate::panels::import_paths(editor, state, &ready);
    if over_timeline && !imported.is_empty() {
        place_all(editor, state, &imported);
    }
    state.needs_repaint = true;
}

/// A LUT picked in a browser: kept by the page under a path of its own,
/// imported from there, and given to the clip it was picked for.
#[cfg(target_arch = "wasm32")]
fn import_lut(editor: &mut Editor, state: &mut UiState, name: &str, bytes: &[u8]) {
    use bettercut_editor_core::foundation::files;
    let path = std::path::PathBuf::from(format!("/files/luts/{}", name.replace(['/', '\\'], "-")));
    if let Err(err) = files::write(&path, bytes) {
        state.error(format!("Could not keep the LUT: {err}"));
        return;
    }
    match editor.import_lut(&path) {
        Ok(id) => {
            if let Some(clip) = state.lut_for.take() {
                let lut = bettercut_editor_core::timeline::ClipLut::new(id);
                if let Err(err) = editor.set_clip_property(
                    clip,
                    bettercut_editor_core::ClipProperty::Lut(Some(lut)),
                    false,
                ) {
                    state.error(err.to_string());
                    return;
                }
            }
            state.info("LUT imported");
        }
        Err(err) => state.error(err.to_string()),
    }
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
