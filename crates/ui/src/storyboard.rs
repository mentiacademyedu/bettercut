//! The storyboard window: the cut as a row of cards
//! (`editor_core::storyboard`).
//!
//! The timeline is the right tool for *when*; this is the right tool for
//! *what, and in what order*. A card is a shot: its poster frame, its name and
//! how long it runs. Dragging one in front of another reorders the cut without
//! anyone measuring anything, and a click puts the playhead on it so the
//! preview shows what was just moved.
//!
//! # It shows the spine, not the stack
//!
//! Only the first picture lane, because only that lane answers "what happens
//! next" (see the core module). Overlays, titles and music are edits about a
//! shot rather than shots themselves, and drawing them here would make this a
//! second timeline — which is a worse timeline, not a storyboard.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TICKS_PER_SECOND;

use crate::state::UiState;
use crate::theme;

/// How wide a card is drawn. Wide enough to tell two shots of the same room
/// apart, narrow enough that a dozen fit across a window.
const CARD_WIDTH: f32 = 150.0;

pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.storyboard_open {
        return;
    }
    let mut open = true;
    crate::theme::placed(egui::Window::new("Storyboard"), ctx)
        .open(&mut open)
        .default_width(680.0)
        .default_height(360.0)
        .resizable(true)
        .show(ctx, |ui| {
            body(ui, editor, state);
        });
    if !open {
        state.storyboard_open = false;
    }
}

fn body(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    let cards = editor.storyboard();
    if cards.is_empty() {
        ui.label(
            egui::RichText::new("Nothing on the first picture lane yet.").color(theme::disabled()),
        );
        return;
    }

    ui.label(
        egui::RichText::new("Drag a card to move that shot; click one to go to it")
            .small()
            .color(theme::disabled()),
    );
    ui.add_space(4.0);

    let playhead = editor.playhead();
    // Where the dragged card should land, decided while drawing and applied
    // after: reordering mid-draw would renumber the cards under the pointer.
    let mut moved: Option<(usize, usize)> = None;
    let mut go_to = None;

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (index, card) in cards.iter().enumerate() {
                let id = egui::Id::new(("storyboard card", card.clip));
                let playing = playhead >= card.start
                    && playhead.ticks() < card.start.ticks() + card.duration.ticks();

                let frame = egui::Frame::new()
                    .inner_margin(6.0)
                    .corner_radius(4)
                    .stroke(egui::Stroke::new(
                        1.0,
                        if playing {
                            theme::playhead()
                        } else {
                            theme::grid_line()
                        },
                    ));

                let (_, dropped) = ui.dnd_drop_zone::<usize, ()>(frame, |ui| {
                    ui.set_width(CARD_WIDTH);
                    ui.vertical(|ui| {
                        let response = ui
                            .dnd_drag_source(id, index, |ui| {
                                draw_card(ui, state, card, index);
                            })
                            .response;
                        if response.clicked() {
                            go_to = Some(card.start);
                        }
                    });
                });
                if let Some(from) = dropped {
                    moved = Some((*from, index));
                }
            }
        });
    });

    if let Some(at) = go_to {
        editor.set_playhead(at);
        state.needs_repaint = true;
    }
    if let Some((from, to)) = moved
        && from != to
        && let Err(err) = editor.reorder_storyboard(from, to)
    {
        state.error(err.to_string());
    }
}

fn draw_card(
    ui: &mut egui::Ui,
    state: &mut UiState,
    card: &bettercut_editor_core::storyboard::Card,
    index: usize,
) {
    let size = egui::vec2(CARD_WIDTH, CARD_WIDTH * 9.0 / 16.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 3, theme::timeline_background());

    // The poster frame, letterboxed rather than stretched: a squashed shot is
    // harder to recognise than a small one.
    if let Some(media) = card.media
        && let Some(texture) = state.thumbnails.texture(ui.ctx(), media).cloned()
    {
        let picture = texture.size_vec2();
        let scale = (rect.width() / picture.x).min(rect.height() / picture.y);
        ui.painter().image(
            texture.id(),
            egui::Rect::from_center_size(rect.center(), picture * scale),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }

    // The number, so a card can be talked about — "move six before four".
    ui.painter().text(
        rect.left_top() + egui::vec2(4.0, 3.0),
        egui::Align2::LEFT_TOP,
        format!("{}", index + 1),
        egui::FontId::proportional(12.0),
        theme::clip_text(),
    );

    ui.label(
        egui::RichText::new(&card.name)
            .small()
            .color(theme::clip_text()),
    );
    ui.label(
        egui::RichText::new(format!(
            "{:.1} s",
            card.duration.ticks() as f64 / TICKS_PER_SECOND as f64
        ))
        .small()
        .color(theme::disabled()),
    );
}
