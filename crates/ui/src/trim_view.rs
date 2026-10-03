//! The trim window: both sides of a cut, big, and a frame at a time
//! (`editor_core::trim_window`).
//!
//! The two frames that decide a cut are the last of the shot going out and the
//! first of the shot coming in. On the timeline they are a few pixels tall and
//! never seen together; here they are side by side, at the size of a preview,
//! and the buttons under them move the cut a frame or five at a time.
//!
//! # The frames are rendered, not guessed
//!
//! Each is the export's own render of that instant (`FrameGrabJob`), which is
//! what the preview and the export would show (§46) — grade, effects, titles
//! and all. A thumbnail of the source file would be a picture of the footage
//! rather than of the cut.
//!
//! Rendering two frames costs a decoder seek each, so they are asked for once
//! per cut and kept until the cut moves.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{TimelineTime, ticks_per_frame};

use crate::state::UiState;
use crate::theme;

/// The window's state: what it is showing, and what it is waiting for.
///
#[derive(Default)]
pub struct TrimView {
    pub open: bool,
    /// Which cut the frames on screen are of. A cut that has moved makes them
    /// stale, and new ones are asked for.
    of: Option<TimelineTime>,
    outgoing: Option<egui::TextureHandle>,
    incoming: Option<egui::TextureHandle>,
    /// The two instants the shell should render, once. Taken by the shell,
    /// which owns the job scheduler (§74).
    pub request: Option<(TimelineTime, TimelineTime)>,
    /// How many of the pair are still on their way.
    pending: u8,
}

/// Printed as what it is doing rather than as its pixels: a texture handle
/// has no `Debug` of its own, and the interface's state is printed in places
/// where two rendered frames would bury everything else.
impl std::fmt::Debug for TrimView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrimView")
            .field("open", &self.open)
            .field("of", &self.of)
            .field(
                "frames",
                &(self.outgoing.is_some(), self.incoming.is_some()),
            )
            .field("pending", &self.pending)
            .finish()
    }
}

impl TrimView {
    /// A rendered frame has arrived: `incoming` says which side of the cut.
    pub fn arrived(
        &mut self,
        ctx: &egui::Context,
        incoming: bool,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        let expected = width as usize * height as usize * 4;
        if rgba.len() < expected || width == 0 || height == 0 {
            self.pending = self.pending.saturating_sub(1);
            return;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            &rgba[..expected],
        );
        let name = if incoming {
            "trim incoming"
        } else {
            "trim outgoing"
        };
        let texture = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
        if incoming {
            self.incoming = Some(texture);
        } else {
            self.outgoing = Some(texture);
        }
        self.pending = self.pending.saturating_sub(1);
    }

    /// Nothing on screen is of this cut any more.
    fn forget(&mut self) {
        self.of = None;
        self.outgoing = None;
        self.incoming = None;
        self.pending = 0;
    }
}

pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.trim.open {
        return;
    }
    let mut open = true;
    crate::theme::placed(egui::Window::new("Trim"), ctx)
        .open(&mut open)
        .default_width(720.0)
        .resizable(true)
        .show(ctx, |ui| {
            body(ui, editor, state);
        });
    if !open {
        state.trim.open = false;
        state.trim.forget();
    }
}

fn body(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    let Some(cut) = editor.cut_near(editor.playhead()) else {
        state.trim.forget();
        ui.label(
            egui::RichText::new(
                "No cut to trim: the first picture lane needs two shots that touch.",
            )
            .color(theme::disabled()),
        );
        return;
    };

    // The frames are of a cut; a cut that has moved needs new ones.
    if state.trim.of != Some(cut.at) && state.trim.pending == 0 {
        let frame = editor
            .active_sequence()
            .and_then(|sequence| ticks_per_frame(sequence.frame_rate))
            .unwrap_or(1)
            .max(1);
        state.trim.of = Some(cut.at);
        state.trim.pending = 2;
        state.trim.request = Some((
            // The last frame of the outgoing shot is the one *before* the cut.
            TimelineTime::from_ticks((cut.at.ticks() - frame).max(0)),
            cut.at,
        ));
    }

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(cut.at.format_timecode()).monospace());
        ui.label(
            egui::RichText::new(if cut.can_move() {
                format!(
                    "room: {} to {}",
                    cut.earliest.format_timecode(),
                    cut.latest.format_timecode()
                )
            } else {
                "no footage to trim into".to_owned()
            })
            .small()
            .color(theme::disabled()),
        );
    });

    let width = ((ui.available_width() - 12.0) / 2.0).max(120.0);
    ui.horizontal(|ui| {
        for (label, texture) in [
            ("going out", &state.trim.outgoing),
            ("coming in", &state.trim.incoming),
        ] {
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.label(egui::RichText::new(label).small().color(theme::disabled()));
                let size = egui::vec2(width, width * 9.0 / 16.0);
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 3, theme::timeline_background());
                match texture {
                    Some(texture) => {
                        let picture = texture.size_vec2();
                        let scale = (rect.width() / picture.x).min(rect.height() / picture.y);
                        ui.painter().image(
                            texture.id(),
                            egui::Rect::from_center_size(rect.center(), picture * scale),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    None => {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "rendering…",
                            egui::FontId::proportional(12.0),
                            theme::disabled(),
                        );
                    }
                }
            });
        }
    });

    // The cut, a frame or five at a time. Both shots change together: this is
    // a roll, so nothing after the pair moves (see the core module).
    ui.add_space(6.0);
    let mut by = None;
    ui.horizontal(|ui| {
        for frames in [-5_i64, -1, 1, 5] {
            let label = if frames > 0 {
                format!("+{frames}")
            } else {
                frames.to_string()
            };
            if ui
                .add_enabled(cut.can_move(), egui::Button::new(label))
                .on_hover_text(if frames.abs() == 1 {
                    "Move the cut one frame"
                } else {
                    "Move the cut five frames"
                })
                .clicked()
            {
                by = Some(frames);
            }
        }
        ui.label(
            egui::RichText::new("the shots either side grow and shrink; the rest stays put")
                .small()
                .color(theme::disabled()),
        );
    });

    if let Some(frames) = by {
        match editor.trim_cut_by(cut.outgoing, frames) {
            Ok(at) => {
                // Follow the cut, so the preview shows what was just decided.
                editor.set_playhead(at);
                state.needs_repaint = true;
            }
            Err(err) => state.error(err.to_string()),
        }
    }
}
