//! Recent colours: the last few picked, offered as swatches beside every
//! colour button, so a title's colour is one click on a bar's and a border's.
//!
//! Remembered in the interface prefs (`crate::prefs`) rather than the
//! project, because "the colours I use" outlasts any one edit. A drag through
//! the picker is one colour, not a hundred: a change within a moment of the
//! last, from the same button, replaces the newest swatch instead of adding.

use crate::state::UiState;

/// How many are kept.
pub const MAX_SWATCHES: usize = 8;

/// A colour picked within this many seconds of the last, from the same
/// button, is the same pick still being dragged.
const SAME_PICK_SECONDS: f64 = 2.0;

/// Put `rgb` at the front, once.
pub fn remember(recent: &mut Vec<[u8; 3]>, rgb: [u8; 3]) {
    recent.retain(|c| *c != rgb);
    recent.insert(0, rgb);
    recent.truncate(MAX_SWATCHES);
}

/// A colour button with the recent colours beside it. Returns whether the
/// colour changed, from the picker or from a swatch.
pub fn colour_edit(ui: &mut egui::Ui, state: &mut UiState, rgb: &mut [u8; 3], hover: &str) -> bool {
    let mut response = ui.color_edit_button_srgb(rgb);
    if !hover.is_empty() {
        response = response.on_hover_text(hover);
    }
    let mut changed = false;
    if response.changed() {
        changed = true;
        let now = ui.input(|i| i.time);
        let same_pick = state
            .swatch_last
            .is_some_and(|(id, at)| id == response.id && now - at < SAME_PICK_SECONDS);
        let recent = &mut state.prefs.recent_colours;
        if same_pick && !recent.is_empty() {
            recent[0] = *rgb;
        } else {
            remember(recent, *rgb);
        }
        state.swatch_last = Some((response.id, now));
        // Written when the pick settles, not on every frame of a drag.
        if !ui.input(|i| i.pointer.any_down()) {
            let _ = state.prefs.save();
        }
    }

    let recent = state.prefs.recent_colours.clone();
    for colour in recent {
        if colour == *rgb {
            continue;
        }
        let (rect, swatch) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
        let [r, g, b] = colour;
        ui.painter()
            .rect_filled(rect, 3, egui::Color32::from_rgb(r, g, b));
        ui.painter().rect_stroke(
            rect,
            3,
            egui::Stroke::new(1.0, crate::theme::disabled()),
            egui::StrokeKind::Inside,
        );
        if swatch
            .on_hover_text(format!("Recent colour #{r:02x}{g:02x}{b:02x}"))
            .clicked()
        {
            *rgb = colour;
            remember(&mut state.prefs.recent_colours, colour);
            let _ = state.prefs.save();
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_is_first_once_and_the_list_is_short() {
        let mut recent = Vec::new();
        for i in 0..12u8 {
            remember(&mut recent, [i, 0, 0]);
        }
        assert_eq!(recent.len(), MAX_SWATCHES);
        assert_eq!(recent[0], [11, 0, 0]);
        remember(&mut recent, [5, 0, 0]);
        assert_eq!(recent[0], [5, 0, 0]);
        assert_eq!(recent.iter().filter(|c| **c == [5, 0, 0]).count(), 1);
        assert_eq!(recent.len(), MAX_SWATCHES);
    }
}
