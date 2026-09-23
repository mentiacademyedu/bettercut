//! The History window: every step of the edit, and a click to go back to any
//! of them.
//!
//! Undo one step at a time is fine for the last slip. It is hopeless for "the
//! colour looked better twenty changes ago": twenty presses of Ctrl+Z, counted
//! blind, overshooting, and back. This lists the steps by name — the same
//! names the Undo button's hover text uses — with the current one marked, so
//! going back is one click on the step that looked right.
//!
//! Undone steps stay listed, greyed, below the current one, until a new edit
//! replaces them: that is exactly when Redo stops being able to reach them, so
//! the list never offers a step that cannot be returned to.
//!
//! The list is a view of the editor's history, read each frame. A jump is made
//! of ordinary undos and redos ([`Editor::jump_in_history`]), so it lands only
//! where pressing the keys would have.

use bettercut_editor_core::Editor;

use crate::state::UiState;
use crate::theme;

/// Where a row sits relative to the project as it is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    /// Applied, and before the current step.
    Done,
    /// The step the project is at.
    Current,
    /// Undone; Redo can still bring it back.
    Undone,
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    pub label: String,
    /// How many steps are applied when this row is the current one — what a
    /// click on it jumps to.
    pub done: usize,
    pub state: StepState,
}

/// The list the window shows: the starting point, every step done, then every
/// step undone.
pub fn rows(editor: &Editor) -> Vec<HistoryRow> {
    let (done, undone) = editor.history_steps();
    let depth = done.len();
    // When the history is full the first row is no longer where the project
    // began, and calling it that would promise a way back that is gone.
    let start = if depth >= editor.history_limit() {
        "Earliest kept step"
    } else {
        "Start"
    };

    std::iter::once(start.to_owned())
        .chain(done)
        .chain(undone)
        .enumerate()
        .map(|(index, label)| HistoryRow {
            label,
            done: index,
            state: match index.cmp(&depth) {
                std::cmp::Ordering::Less => StepState::Done,
                std::cmp::Ordering::Equal => StepState::Current,
                std::cmp::Ordering::Greater => StepState::Undone,
            },
        })
        .collect()
}

pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.history_open {
        return;
    }

    let rows = rows(editor);
    let depth = editor.undo_depth();
    // Follow the current step only when it moves, so scrolling back through a
    // long history to read it is not yanked back to the bottom every frame.
    let moved = state.history_seen_depth != Some(depth);
    state.history_seen_depth = Some(depth);

    let mut jump: Option<usize> = None;
    let mut open = true;
    egui::Window::new("History")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([260.0, 360.0])
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("Click a step to go back to it. Greyed steps can be redone.")
                    .small()
                    .color(theme::disabled()),
            );
            ui.add(
                egui::TextEdit::singleline(&mut state.history_search)
                    .hint_text("Search steps: colour, split, title…")
                    .desired_width(f32::INFINITY),
            );
            ui.separator();

            // The current step is always shown, so a search never hides
            // where the project is now.
            let query = state.history_search.trim().to_lowercase();
            let shown: Vec<&HistoryRow> = rows
                .iter()
                .filter(|row| {
                    query.is_empty()
                        || row.state == StepState::Current
                        || matches(&row.label, &query)
                })
                .collect();
            if !query.is_empty() && shown.len() <= 1 {
                ui.label(
                    egui::RichText::new("No step matches that")
                        .small()
                        .color(theme::disabled()),
                );
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for row in shown {
                        let text = match row.state {
                            StepState::Current => egui::RichText::new(&row.label).strong(),
                            StepState::Done => egui::RichText::new(&row.label),
                            StepState::Undone => egui::RichText::new(&row.label)
                                .italics()
                                .color(theme::disabled()),
                        };
                        let response = ui
                            .add(
                                egui::Button::selectable(row.state == StepState::Current, text)
                                    .min_size(egui::vec2(ui.available_width(), 0.0)),
                            )
                            .on_hover_text(match row.state {
                                StepState::Current => "Where the project is now".to_owned(),
                                StepState::Done => format!("Undo back to '{}'", row.label),
                                StepState::Undone => format!("Redo up to '{}'", row.label),
                            });
                        if row.state == StepState::Current && moved {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() && row.state != StepState::Current {
                            jump = Some(row.done);
                        }
                    }
                });
        });
    state.history_open = open && state.history_open;

    if let Some(done) = jump {
        match editor.jump_in_history(done) {
            Ok(0) => {}
            Ok(steps) => state.info(format!(
                "Went {} {steps} step{}",
                if done < depth { "back" } else { "forward" },
                if steps == 1 { "" } else { "s" }
            )),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }
}

/// Whether a step's label answers a search: every word typed appears in
/// it, in any order, ignoring case — "colour clip" finds "Change Colour on
/// 3 Clips".
pub fn matches(label: &str, query: &str) -> bool {
    let label = label.to_lowercase();
    query
        .split_whitespace()
        .all(|word| label.contains(&word.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_editor_core::foundation::TimelineTime;

    /// Up to three edits with different names, so a row in the wrong place
    /// shows as the wrong label.
    fn editor_with_steps(n: usize) -> Editor {
        let (mut editor, _events) = Editor::new_project("History");
        let steps: [fn(&mut Editor); 3] = [
            |editor| {
                editor.toggle_marker(TimelineTime::from_seconds(1)).unwrap();
            },
            |editor| {
                editor.add_text("Title").unwrap();
            },
            |editor| {
                editor.add_adjustment().unwrap();
            },
        ];
        for step in &steps[..n] {
            step(&mut editor);
        }
        editor
    }

    fn states(rows: &[HistoryRow]) -> Vec<StepState> {
        rows.iter().map(|row| row.state).collect()
    }

    /// A start row, then each step; the newest is current, and clicking any
    /// row jumps to that row's position in the list.
    #[test]
    fn the_newest_step_is_current_and_every_row_knows_where_it_jumps() {
        let editor = editor_with_steps(2);
        let rows = rows(&editor);

        assert_eq!(rows[0].label, "Start");
        assert_eq!(
            states(&rows),
            vec![StepState::Done, StepState::Done, StepState::Current]
        );
        assert_eq!(
            rows.iter().map(|r| r.done).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(Some(rows[2].label.clone()), editor.undo_label());
    }

    /// Undone steps stay listed after the current one, so they can be clicked
    /// to redo — and "Start" becomes current when everything is undone.
    #[test]
    fn undone_steps_follow_the_current_one() {
        let mut editor = editor_with_steps(2);
        editor.undo().unwrap();
        assert_eq!(
            states(&rows(&editor)),
            vec![StepState::Done, StepState::Current, StepState::Undone]
        );

        editor.undo().unwrap();
        assert_eq!(
            states(&rows(&editor)),
            vec![StepState::Current, StepState::Undone, StepState::Undone]
        );
        // The undone rows keep the order Redo would take them in.
        let (_, undone) = editor.history_steps();
        assert_eq!(Some(rows(&editor)[1].label.clone()), editor.redo_label());
        assert_ne!(undone[0], undone[1], "setup: the steps need distinct names");
    }

    /// Clicking a row and reading the list back lands on that row.
    #[test]
    fn jumping_to_a_rows_position_makes_it_current() {
        let mut editor = editor_with_steps(3);
        let target = rows(&editor)[1].clone();

        editor.jump_in_history(target.done).unwrap();

        let after = rows(&editor);
        assert_eq!(after[1].state, StepState::Current);
        assert_eq!(after[1].label, target.label);
    }
}
