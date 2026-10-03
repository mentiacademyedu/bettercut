//! What changed since an earlier save (`editor_core::compare`).
//!
//! The version list can say when each save happened and nothing about what is
//! in it, which makes going back a guess. This window answers the question
//! before anything is opened: the differences between the edit as it stands
//! and the save you are looking at, line by line, in the words the rest of the
//! interface uses.
//!
//! It never changes anything. Going back is still the button in the menu; this
//! is what you read before pressing it.

use bettercut_editor_core::compare::{Change, ChangeKind};

use crate::state::UiState;
use crate::theme;

/// A comparison, opened and waiting to be read.
#[derive(Debug, Default)]
pub struct ChangesView {
    pub open: bool,
    /// The version compared against, as its row in the menu named it.
    pub label: String,
    /// One line for what is different. In the project's own order.
    pub changes: Vec<Change>,
    /// Only the kinds ticked here are listed. All of them, to begin with.
    pub hidden: Vec<ChangeKind>,
}

impl ChangesView {
    /// Show `changes` against the save called `label`.
    pub fn show_changes(&mut self, label: impl Into<String>, changes: Vec<Change>) {
        self.open = true;
        self.label = label.into();
        self.changes = changes;
        self.hidden.clear();
    }

    fn showing(&self, kind: ChangeKind) -> bool {
        !self.hidden.contains(&kind)
    }

    fn toggle(&mut self, kind: ChangeKind) {
        if let Some(at) = self.hidden.iter().position(|hidden| *hidden == kind) {
            self.hidden.remove(at);
        } else {
            self.hidden.push(kind);
        }
    }
}

/// The colour a kind of change is drawn in: the same three the timeline uses
/// for the same ideas, so nothing new has to be learnt to read this.
fn colour(kind: ChangeKind) -> egui::Color32 {
    match kind {
        ChangeKind::Added => theme::ok_text(),
        ChangeKind::Removed => theme::error_text(),
        ChangeKind::Moved | ChangeKind::Trimmed | ChangeKind::Retimed => theme::caution(),
        ChangeKind::Changed => theme::clip_text(),
    }
}

pub fn show(ctx: &egui::Context, state: &mut UiState) {
    if !state.version_changes.open {
        return;
    }
    let mut open = true;
    crate::theme::placed(egui::Window::new("What Changed"), ctx)
        .open(&mut open)
        .default_width(460.0)
        .default_height(420.0)
        .resizable(true)
        .show(ctx, |ui| body(ui, state));
    if !open {
        state.version_changes.open = false;
    }
}

fn body(ui: &mut egui::Ui, state: &mut UiState) {
    let view = &mut state.version_changes;
    ui.label(
        egui::RichText::new(format!("This edit, against the save of {}", view.label))
            .small()
            .color(theme::disabled()),
    );
    ui.label(egui::RichText::new(bettercut_editor_core::compare::summary(&view.changes)).strong());
    if view.changes.is_empty() {
        return;
    }

    // One tick a kind: a list of two hundred moves is unreadable when what you
    // came to find out is what was *removed*.
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Show").small().color(theme::disabled()));
        for kind in [
            ChangeKind::Added,
            ChangeKind::Removed,
            ChangeKind::Moved,
            ChangeKind::Trimmed,
            ChangeKind::Retimed,
            ChangeKind::Changed,
        ] {
            let count = view
                .changes
                .iter()
                .filter(|change| change.kind == kind)
                .count();
            if count == 0 {
                continue;
            }
            let showing = view.showing(kind);
            if ui
                .add(egui::Button::selectable(
                    showing,
                    format!("{} {count}", kind.label()),
                ))
                .clicked()
            {
                view.toggle(kind);
            }
        }
    });

    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut place = String::new();
        for change in &view.changes {
            if !view.showing(change.kind) {
                continue;
            }
            // The lane's name once, over its own lines, rather than repeated
            // down every row of the list.
            if change.place != place {
                place = change.place.clone();
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&place).small().strong());
            }
            ui.label(egui::RichText::new(format!("• {}", change.what)).color(colour(change.kind)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(kind: ChangeKind) -> Change {
        Change {
            kind,
            place: "V1".to_owned(),
            what: "something".to_owned(),
        }
    }

    #[test]
    fn opening_a_comparison_shows_every_kind() {
        let mut view = ChangesView::default();
        view.show_changes("2026-09-20 01:00:00", vec![change(ChangeKind::Added)]);

        assert!(view.open);
        assert!(view.showing(ChangeKind::Added));
        assert!(view.showing(ChangeKind::Removed));
    }

    #[test]
    fn a_kind_can_be_hidden_and_shown_again() {
        let mut view = ChangesView::default();
        view.show_changes("a save", vec![change(ChangeKind::Moved)]);

        view.toggle(ChangeKind::Moved);
        assert!(!view.showing(ChangeKind::Moved));
        assert!(view.showing(ChangeKind::Added), "the others went with it");
        view.toggle(ChangeKind::Moved);
        assert!(view.showing(ChangeKind::Moved));
    }

    /// A second comparison starts fresh: filters left over from the last one
    /// would hide lines the person never chose to hide.
    #[test]
    fn a_new_comparison_forgets_the_filters() {
        let mut view = ChangesView::default();
        view.show_changes("first", vec![change(ChangeKind::Added)]);
        view.toggle(ChangeKind::Added);

        view.show_changes("second", vec![change(ChangeKind::Added)]);
        assert!(view.showing(ChangeKind::Added));
        assert_eq!(view.label, "second");
    }
}
