//! The Markers window: every marker in one list — jump to it, name it, delete
//! it.
//!
//! Markers pile up. Beat detection drops a few hundred, and a person marking
//! "cut here" and "fix the colour" as they watch drops a few dozen more. On the
//! ruler they are little green triangles, and finding the one that said "drone
//! shot" means hovering over each. Here they are a list, in order, with their
//! names: click the timecode to go there, type to rename, × to delete.
//!
//! A name is typed into a draft and applied when the field is left — Enter,
//! Tab or a click elsewhere — so a renamed marker is one undo step, not one per
//! letter. Escape leaves the name as it was.
//!
//! Like the other lists, this is a view: rows are read from the editor each
//! frame, and every change goes through it.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;

use crate::state::UiState;
use crate::theme;

/// One marker, as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerRow {
    pub time: TimelineTime,
    pub label: String,
    pub color: bettercut_editor_core::timeline::ColorLabel,
}

/// Every marker on the active sequence, in time order.
pub fn rows(editor: &Editor) -> Vec<MarkerRow> {
    editor
        .markers()
        .iter()
        .map(|marker| MarkerRow {
            time: marker.time,
            label: marker.label.clone(),
            color: marker.color,
        })
        .collect()
}

/// The row to highlight: the last marker at or before the playhead — the
/// section of the edit the playhead is in, when markers are used to name them.
pub fn row_at(rows: &[MarkerRow], playhead: TimelineTime) -> Option<usize> {
    rows.iter().rposition(|row| row.time <= playhead)
}

/// What the text field shows for a row: the draft being typed, if it is this
/// row's, or the stored name.
pub fn shown_label<'a>(row: &'a MarkerRow, draft: Option<&'a (TimelineTime, String)>) -> &'a str {
    match draft {
        Some((time, text)) if *time == row.time => text,
        _ => &row.label,
    }
}

/// Something asked for in the window, applied after it is drawn.
enum Action {
    Jump(TimelineTime),
    Rename(TimelineTime, String),
    Colour(TimelineTime, bettercut_editor_core::timeline::ColorLabel),
    Delete(TimelineTime),
    AddAtPlayhead,
    ClearAll,
    CopyChapters,
}

/// What "Copy Chapters" says afterwards: how many were copied, and anything a
/// video site would reject the list for.
pub fn chapters_report(chapters: &bettercut_editor_core::timeline::Chapters) -> String {
    let mut report = format!(
        "Copied {} chapter{} for a video description",
        chapters.count,
        if chapters.count == 1 { "" } else { "s" }
    );
    for problem in &chapters.problems {
        report.push_str(" · ");
        report.push_str(&problem.describe());
    }
    report
}

/// Draw the window, if it is open.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.markers_open {
        // Closed with a name half typed — by its X or the toolbar button —
        // is leaving the field: keep what was typed.
        if let Some((time, text)) = state.marker_draft.take() {
            if let Err(err) = editor.set_marker_label(time, &text) {
                state.error(err.to_string());
            }
            state.needs_repaint = true;
        }
        return;
    }
    let rows = rows(editor);
    let current = row_at(&rows, editor.playhead());
    let mut actions: Vec<Action> = Vec::new();
    let mut open = true;

    egui::Window::new("Markers")
        .open(&mut open)
        .default_width(320.0)
        .default_height(360.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button("Add at Playhead")
                    .on_hover_text("Mark the playhead (M)")
                    .clicked()
                {
                    actions.push(Action::AddAtPlayhead);
                }
                if !rows.is_empty()
                    && ui
                        .button("Copy Chapters")
                        .on_hover_text(
                            "Copy the markers as \"0:00 Intro\" lines, ready to paste into a video description",
                        )
                        .clicked()
                {
                    actions.push(Action::CopyChapters);
                }
                if !rows.is_empty()
                    && ui
                        .button(egui::RichText::new("Clear All").color(theme::ERROR_TEXT))
                        .on_hover_text("Delete every marker. Undoable.")
                        .clicked()
                {
                    actions.push(Action::ClearAll);
                }
            });

            if rows.is_empty() {
                ui.add_space(6.0);
                ui.label("No markers yet.");
                ui.label(
                    egui::RichText::new(
                        "Press M to mark the playhead, right-click the timeline, or detect \
                         beats on a music clip.",
                    )
                    .small()
                    .color(theme::DISABLED),
                );
                return;
            }

            ui.label(
                egui::RichText::new(format!(
                    "{} marker{} · click a time to go there",
                    rows.len(),
                    if rows.len() == 1 { "" } else { "s" }
                ))
                .small()
                .color(theme::DISABLED),
            );
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, row) in rows.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let time = egui::RichText::new(row.time.format_timecode()).monospace();
                            // Its colour, and a menu to change it: the word
                            // drawn in the colour it is, each choice in its own.
                            let swatch = crate::timeline::marker_colour(row.color);
                            ui.menu_button(egui::RichText::new("colour").small().color(swatch), |ui| {
                                for label in bettercut_editor_core::timeline::ColorLabel::ALL {
                                    let text = egui::RichText::new(label.name())
                                        .color(crate::timeline::marker_colour(label));
                                    if ui.selectable_label(row.color == label, text).clicked() {
                                        ui.close();
                                        actions.push(Action::Colour(row.time, label));
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Colour this marker");
                            let time = if current == Some(index) {
                                time.color(theme::PLAYHEAD)
                            } else {
                                time.color(swatch)
                            };
                            if ui
                                .add(egui::Button::new(time).frame(false))
                                .on_hover_text("Move the playhead here")
                                .clicked()
                            {
                                actions.push(Action::Jump(row.time));
                            }

                            // The delete button first, laid out from the right,
                            // so the name field takes whatever width is left.
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button("×")
                                        .on_hover_text("Delete this marker")
                                        .clicked()
                                    {
                                        actions.push(Action::Delete(row.time));
                                    }

                                    let mut text =
                                        shown_label(row, state.marker_draft.as_ref()).to_owned();
                                    let field = ui.add(
                                        egui::TextEdit::singleline(&mut text)
                                            .id(egui::Id::new(("marker", row.time.ticks())))
                                            .desired_width(f32::INFINITY)
                                            .char_limit(Editor::MAX_MARKER_LABEL)
                                            .hint_text("name this marker"),
                                    );
                                    if field.changed() {
                                        state.marker_draft = Some((row.time, text));
                                    }
                                    if field.lost_focus() {
                                        let draft = state
                                            .marker_draft
                                            .take_if(|(time, _)| *time == row.time);
                                        let escaped =
                                            ui.input(|i| i.key_pressed(egui::Key::Escape));
                                        if let Some((time, text)) = draft
                                            && !escaped
                                        {
                                            actions.push(Action::Rename(time, text));
                                        }
                                    }
                                },
                            );
                        });
                    }
                });
        });
    if !open {
        state.markers_open = false;
        if let Some((time, text)) = state.marker_draft.take() {
            actions.push(Action::Rename(time, text));
        }
    }

    for action in actions {
        let result = match action {
            Action::Jump(time) => {
                editor.set_playhead(time);
                Ok(())
            }
            Action::Rename(time, text) => editor.set_marker_label(time, &text).map(|_| ()),
            Action::Colour(time, label) => editor.set_marker_color(time, label).map(|_| ()),
            Action::Delete(time) => editor.remove_marker(time).map(|_| ()),
            // Not the M key's toggle: a button that says "add" never deletes.
            Action::AddAtPlayhead => editor.add_markers(&[editor.playhead()]).map(|added| {
                if added == 0 {
                    state.info("There is already a marker at the playhead");
                }
            }),
            Action::ClearAll => editor.clear_markers(),
            Action::CopyChapters => {
                let duration = editor
                    .active_sequence()
                    .map_or(TimelineTime::ZERO, |s| s.duration());
                let chapters =
                    bettercut_editor_core::timeline::chapter_list(editor.markers(), duration);
                ctx.copy_text(chapters.text.clone());
                if chapters.problems.is_empty() {
                    state.info(chapters_report(&chapters));
                } else {
                    state.error(chapters_report(&chapters));
                }
                Ok(())
            }
        };
        if let Err(err) = result {
            state.error(err.to_string());
        }
        state.needs_repaint = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seconds: i64, label: &str) -> MarkerRow {
        MarkerRow {
            time: TimelineTime::from_seconds(seconds),
            label: label.to_owned(),
            color: bettercut_editor_core::timeline::ColorLabel::None,
        }
    }

    #[test]
    fn the_highlight_is_the_last_marker_reached() {
        let rows = [row(2, ""), row(5, "chorus"), row(9, "")];
        let at = |s| row_at(&rows, TimelineTime::from_seconds(s));
        assert_eq!(at(0), None, "before the first marker");
        assert_eq!(at(2), Some(0), "on a marker counts as reached");
        assert_eq!(at(7), Some(1));
        assert_eq!(at(60), Some(2));
    }

    #[test]
    fn a_draft_shows_only_on_its_own_row() {
        let first = row(2, "intro");
        let second = row(5, "chorus");
        let draft = (TimelineTime::from_seconds(5), "verse".to_owned());
        assert_eq!(shown_label(&first, Some(&draft)), "intro");
        assert_eq!(shown_label(&second, Some(&draft)), "verse");
        assert_eq!(shown_label(&second, None), "chorus");
    }
}
