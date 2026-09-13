//! Keyboard shortcuts (§57).
//!
//! The keys are listed once, in [`SECTIONS`], which is also what the
//! Shortcuts window (`?` or F1) shows — so the list a user reads and the keys
//! that work are edited side by side.
//!
//! The clip actions are also on the timeline's right-click menu
//! (`context_menu`), and both routes call the same functions below — a menu
//! item that quietly did something slightly different from its shortcut is a
//! bug waiting to happen.
//!
//! Actions belonging to milestones that have not landed report that clearly in
//! the status bar instead of doing nothing — silence reads as a broken key.

use bettercut_editor_core::foundation::{ClipId, TrackId};
use bettercut_editor_core::{Command, Editor, TrimEdge};

use egui::Key;

use crate::panels;
use crate::state::UiState;

/// Every shortcut, as the Shortcuts window lists it: grouped, keys then what
/// they do.
///
/// One table, next to the handler, so the list a user reads and the keys that
/// actually work are edited in the same place.
pub const SECTIONS: &[Section] = &[
    Section {
        title: "Playback",
        shortcuts: &[
            Shortcut::new("Space", "Play / pause", &[Key::Space]),
            Shortcut::new(
                "J / K / L",
                "Reverse / stop / forward; press again to go faster (hold K: step a frame)",
                &[Key::J, Key::K, Key::L],
            ),
            Shortcut::new(
                "Left / Right",
                "Step one frame (Shift: ten; Alt: swap the selected clip with its neighbour)",
                &[Key::ArrowLeft, Key::ArrowRight],
            ),
            Shortcut::new(
                "Up / Down",
                "Jump to the previous / next cut or marker",
                &[Key::ArrowUp, Key::ArrowDown],
            ),
            Shortcut::new(
                "Home / End",
                "Jump to the start / end",
                &[Key::Home, Key::End],
            ),
        ],
    },
    Section {
        title: "Cutting",
        shortcuts: &[
            Shortcut::new("S", "Split at the playhead", &[Key::S]),
            Shortcut::new(
                "[ and ]",
                "Trim the selection's start / end to the playhead",
                &[Key::OpenBracket, Key::CloseBracket],
            ),
            Shortcut::new(
                "Q / W",
                "Cut away the clip's start / end at the playhead and close the gap",
                &[Key::Q, Key::W],
            ),
            Shortcut::new(
                ", and .",
                "Nudge the selection a frame (Shift: ten)",
                &[Key::Comma, Key::Period],
            ),
            Shortcut::new("M", "Add / remove a marker at the playhead", &[Key::M]),
            Shortcut::new(
                "I / O",
                "Mark in / out at the playhead (Alt: clear both)",
                &[Key::I, Key::O],
            ),
            Shortcut::new("N", "Snapping on / off", &[Key::N]),
        ],
    },
    Section {
        title: "Clips",
        shortcuts: &[
            Shortcut::new(
                "Delete",
                "Delete the selection (Backspace too)",
                &[Key::Delete, Key::Backspace],
            ),
            Shortcut::new("Shift + Delete", "Delete and close the gap", &[]),
            Shortcut::new(
                "Ctrl + X / C / V",
                "Cut / copy / paste",
                &[Key::X, Key::C, Key::V],
            ),
            Shortcut::new("Ctrl + Alt + C / V", "Copy / paste a clip's look", &[]),
            Shortcut::new("Ctrl + D", "Duplicate", &[Key::D]),
            Shortcut::new(
                "Ctrl + G",
                "Group the selection to move together (Shift: ungroup)",
                &[Key::G],
            ),
            Shortcut::new(
                "Ctrl + A",
                "Select every clip (Shift: every clip after the playhead)",
                &[Key::A],
            ),
        ],
    },
    Section {
        title: "Project",
        shortcuts: &[
            Shortcut::new("Ctrl + Z", "Undo", &[Key::Z]),
            Shortcut::new("Ctrl + Shift + Z", "Redo", &[]),
            Shortcut::new("Ctrl + H", "History: go back to any step", &[Key::H]),
            Shortcut::new("Ctrl + S", "Save (Shift: save as)", &[]),
            Shortcut::new(
                "Esc",
                "Put down the eyedropper, or finish cropping",
                &[Key::Escape],
            ),
            Shortcut::new("? or F1", "This list", &[Key::Questionmark, Key::F1]),
        ],
    },
    Section {
        title: "Mouse",
        shortcuts: &[
            Shortcut::new("Wheel", "Scroll the timeline", &[]),
            Shortcut::new("Ctrl + wheel", "Zoom the timeline", &[]),
            Shortcut::new("Ctrl + click", "Add a clip to the selection", &[]),
            Shortcut::new("Drag on empty space", "Select every clip in a box", &[]),
            Shortcut::new("Alt + drag", "Move without snapping", &[]),
            Shortcut::new("Right-click", "Actions for what is under the pointer", &[]),
            Shortcut::new(
                "Double-click a volume line",
                "Add a volume point (on a point: remove it)",
                &[],
            ),
        ],
    },
];

/// Every key [`handle`] listens for. One list, read by the handler and by a
/// test that holds the sheet to it: a key that works but is not in the sheet
/// is a feature nobody finds, and a row for a key that is not handled is a
/// promise the editor breaks.
pub const HANDLED_KEYS: &[Key] = &[
    Key::Space,
    Key::J,
    Key::K,
    Key::L,
    Key::Z,
    Key::H,
    Key::S,
    Key::C,
    Key::V,
    Key::X,
    Key::D,
    Key::A,
    Key::G,
    Key::N,
    Key::M,
    Key::I,
    Key::O,
    Key::Comma,
    Key::Period,
    Key::OpenBracket,
    Key::CloseBracket,
    Key::Q,
    Key::W,
    Key::Delete,
    Key::Backspace,
    Key::ArrowLeft,
    Key::ArrowRight,
    Key::Home,
    Key::End,
    Key::ArrowUp,
    Key::ArrowDown,
    Key::F1,
    Key::Questionmark,
    Key::Escape,
];

/// A group of rows in the sheet, so twenty-odd keys read as five short lists.
#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub title: &'static str,
    pub shortcuts: &'static [Shortcut],
}

/// One row of the sheet.
#[derive(Debug, Clone, Copy)]
pub struct Shortcut {
    /// As the user presses it: "Ctrl + D".
    pub keys: &'static str,
    pub action: &'static str,
    /// The keys this row is where the sheet explains, for the test that holds
    /// the sheet and the handler together. A modifier variant of a key already
    /// claimed elsewhere (Shift + Delete) and mouse gestures claim none.
    pub bound: &'static [Key],
}

impl Shortcut {
    const fn new(keys: &'static str, action: &'static str, bound: &'static [Key]) -> Self {
        Self {
            keys,
            action,
            bound,
        }
    }
}

/// The rows whose keys or action contain `query`, ignoring case, still in their
/// sections. An empty query is every row. Sections left with no rows are left
/// out, so a search never shows a heading over nothing.
pub fn matching(query: &str) -> Vec<(&'static str, Vec<Shortcut>)> {
    let query = query.trim().to_lowercase();
    SECTIONS
        .iter()
        .map(|section| {
            let rows: Vec<Shortcut> = section
                .shortcuts
                .iter()
                .filter(|row| {
                    query.is_empty()
                        || row.keys.to_lowercase().contains(&query)
                        || row.action.to_lowercase().contains(&query)
                })
                .copied()
                .collect();
            (section.title, rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

pub fn handle(
    ctx: &egui::Context,
    editor: &mut Editor,
    state: &mut UiState,
    mut preview: Option<&mut crate::Preview>,
) {
    // Never steal keys while a text field has focus.
    if ctx.egui_wants_keyboard_input() {
        return;
    }

    let (keys, modifiers, holding_k) = ctx.input(|i| {
        let pressed: Vec<egui::Key> = HANDLED_KEYS
            .iter()
            .copied()
            .filter(|k| i.key_pressed(*k))
            .collect();
        (pressed, i.modifiers, i.key_down(egui::Key::K))
    });

    for key in keys {
        match key {
            egui::Key::Space => match preview.as_deref_mut() {
                Some(preview) => {
                    let playing = !preview.is_playing();
                    preview.set_playing(editor, playing);
                    state.needs_repaint = true;
                    state.info(if playing { "Playing" } else { "Paused" });
                }
                None => state.error("No preview renderer"),
            },

            // K held with J or L: one frame at a time, stopped — how the frame
            // to cut on is found by hand.
            egui::Key::J | egui::Key::L if holding_k && !modifiers.command => {
                if let Some(preview) = preview.as_deref_mut() {
                    preview.set_playing(editor, false);
                }
                editor.step_frames(if key == egui::Key::L { 1 } else { -1 });
                if let Some(preview) = preview.as_deref_mut() {
                    preview.seek_to(editor.playhead());
                }
                state.needs_repaint = true;
            }
            egui::Key::J | egui::Key::K | egui::Key::L if !modifiers.command => {
                let pressed = match key {
                    egui::Key::J => crate::shuttle::ShuttleKey::Back,
                    egui::Key::K => crate::shuttle::ShuttleKey::Stop,
                    _ => crate::shuttle::ShuttleKey::Forward,
                };
                match preview.as_deref_mut() {
                    Some(preview) => {
                        let rate = crate::shuttle::after_press(preview.shuttle_rate(), pressed);
                        preview.set_shuttle(editor, rate);
                        state.info(crate::shuttle::describe(rate));
                        state.needs_repaint = true;
                    }
                    None => state.error("No preview renderer"),
                }
            }

            egui::Key::Z if modifiers.command && modifiers.shift => match editor.redo() {
                Ok(()) => state.needs_repaint = true,
                Err(err) => state.info(err.to_string()),
            },
            egui::Key::Z if modifiers.command => match editor.undo() {
                Ok(()) => state.needs_repaint = true,
                Err(err) => state.info(err.to_string()),
            },

            egui::Key::H if modifiers.command => {
                state.history_open = !state.history_open;
                state.needs_repaint = true;
            }

            egui::Key::S if modifiers.command && modifiers.shift => {
                panels::save_project_as(editor, state);
            }
            egui::Key::S if modifiers.command => panels::save_project(editor, state),
            egui::Key::S => split_at_playhead(editor, state),

            egui::Key::X if modifiers.command => cut_selection(editor, state),
            // §45: the same pair with Alt carries the *look* rather than the
            // clip. Checked first, because `command` is set for both and the
            // plainer copy would otherwise swallow it.
            egui::Key::C if modifiers.command && modifiers.alt => copy_look(editor, state),
            egui::Key::V if modifiers.command && modifiers.alt => paste_look(editor, state),
            egui::Key::C if modifiers.command => copy_selection(editor, state),
            egui::Key::V if modifiers.command => paste_at_playhead(editor, state),
            egui::Key::D if modifiers.command => duplicate_selection(editor, state),
            egui::Key::G if modifiers.command && modifiers.shift => {
                ungroup_selection(editor, state);
            }
            egui::Key::G if modifiers.command => group_selection(editor, state),
            // Everything after the playhead is how the rest of an edit is
            // pushed along to make room, or cleared away to start again.
            egui::Key::A if modifiers.command => {
                let from = if modifiers.shift {
                    editor.playhead()
                } else {
                    bettercut_editor_core::foundation::TimelineTime::ZERO
                };
                select_from(editor, state, from, None);
            }

            // The way out of any armed mode. A control that puts the
            // interface into a state has to say how to leave it, and Escape is
            // the one key everybody already tries — without it an armed
            // eyedropper leaves the preview's handles dead with no obvious
            // way back, because the button that armed it has scrolled away.
            egui::Key::Escape => {
                cancel_modes(state);
            }

            // §10 "Snapping" — a toggle worth a bare key, since it gets flipped
            // constantly while laying out a cut.
            egui::Key::N => {
                state.snapping = !state.snapping;
                state.info(if state.snapping {
                    "Snapping on"
                } else {
                    "Snapping off"
                });
            }

            // Shift+Delete ripples: delete and close the gap (§10).
            egui::Key::Delete | egui::Key::Backspace if modifiers.shift => {
                ripple_delete_selection(editor, state);
            }
            egui::Key::Delete | egui::Key::Backspace => delete_selection(editor, state),

            egui::Key::ArrowLeft | egui::Key::ArrowRight if modifiers.alt => {
                let side = if key == egui::Key::ArrowLeft {
                    bettercut_editor_core::Neighbour::Previous
                } else {
                    bettercut_editor_core::Neighbour::Next
                };
                match state
                    .selected_clips
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .as_slice()
                {
                    [clip] => swap_clip(editor, state, *clip, side),
                    [] => state.info("Select a clip to swap"),
                    _ => state.info("Select one clip to swap"),
                }
            }
            egui::Key::ArrowLeft => {
                editor.step_frames(if modifiers.shift { -10 } else { -1 });
                state.needs_repaint = true;
            }
            egui::Key::ArrowRight => {
                editor.step_frames(if modifiers.shift { 10 } else { 1 });
                state.needs_repaint = true;
            }

            egui::Key::Home => {
                editor.set_playhead(bettercut_editor_core::foundation::TimelineTime::ZERO);
                state.scroll_ticks = 0;
                state.needs_repaint = true;
            }
            egui::Key::End => {
                if let Some(duration) = editor.active_sequence().map(|s| s.duration()) {
                    editor.set_playhead(duration);
                    state.needs_repaint = true;
                }
            }

            // The cut before or after the playhead, on any track: how a trim is
            // lined up with a neighbour without zooming in to find the edge.
            egui::Key::ArrowUp | egui::Key::ArrowDown => {
                let forward = key == egui::Key::ArrowDown;
                if let Some(at) = next_cut(editor, forward) {
                    editor.set_playhead(at);
                    state.needs_repaint = true;
                }
            }

            // In and out: the part of the edit an export will write on its
            // own. Alt with either clears both, as it does in most editors.
            egui::Key::I | egui::Key::O if modifiers.alt => match editor.clear_marks() {
                Ok(()) => {
                    state.info("In and out cleared");
                    state.needs_repaint = true;
                }
                Err(err) => state.error(err.to_string()),
            },
            egui::Key::I | egui::Key::O if !modifiers.command => {
                let at = editor.playhead();
                let result = if key == egui::Key::I {
                    editor.set_mark_in(at)
                } else {
                    editor.set_mark_out(at)
                };
                match result {
                    Ok(()) => {
                        state.info(if key == egui::Key::I {
                            "In marked"
                        } else {
                            "Out marked"
                        });
                        state.needs_repaint = true;
                    }
                    Err(err) => state.error(err.to_string()),
                }
            }

            egui::Key::M if !modifiers.command => match editor.toggle_marker(editor.playhead()) {
                Ok(added) => {
                    state.info(if added {
                        "Marker added"
                    } else {
                        "Marker removed"
                    });
                    state.needs_repaint = true;
                }
                Err(err) => state.error(err.to_string()),
            },

            // Nudging: the keyboard equivalent of dragging, and the only way
            // to move a clip by exactly one frame.
            egui::Key::Comma | egui::Key::Period => {
                let frames = if modifiers.shift { 10 } else { 1 };
                let frames = if key == egui::Key::Comma {
                    -frames
                } else {
                    frames
                };
                nudge_selection(editor, state, frames);
            }

            // Trimming to the playhead: how an edit is tightened without
            // aiming at a clip edge with the mouse.
            egui::Key::OpenBracket => trim_selection(editor, state, TrimEdge::Start),
            egui::Key::CloseBracket => trim_selection(editor, state, TrimEdge::End),
            // The trim that leaves no hole: everything after moves up.
            egui::Key::Q if !modifiers.command => ripple_trim(editor, state, TrimEdge::Start),
            egui::Key::W if !modifiers.command => ripple_trim(editor, state, TrimEdge::End),

            egui::Key::F1 | egui::Key::Questionmark => {
                state.shortcuts_open = !state.shortcuts_open;
                state.needs_repaint = true;
            }

            _ => {}
        }
    }
}

/// Move everything selected by `frames`, negative for earlier (§57).
pub fn nudge_selection(editor: &mut Editor, state: &mut UiState, frames: i64) {
    if state.selected_clips.is_empty() {
        state.info("Nothing selected");
        return;
    }
    let selected: Vec<ClipId> = state.selected_clips.iter().copied().collect();
    match editor.nudge_clips(&selected, frames) {
        Ok(0) => state.info("Nothing to nudge"),
        Ok(_) => state.needs_repaint = true,
        // A nudge into a neighbour is refused by the track, and saying so
        // beats a key that silently does nothing.
        Err(err) => state.error(err.to_string()),
    }
}

/// Trim the selection's start or end to the playhead (§57).
pub fn trim_selection(editor: &mut Editor, state: &mut UiState, edge: TrimEdge) {
    if state.selected_clips.is_empty() {
        state.info("Nothing selected");
        return;
    }
    let selected: Vec<ClipId> = state.selected_clips.iter().copied().collect();
    match editor.trim_to_playhead(&selected, edge) {
        Ok(0) => state.info("Put the playhead inside the clip to trim it there"),
        Ok(n) => {
            state.needs_repaint = true;
            state.info(format!("Trimmed {n} clip(s)"));
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Ctrl+G: group the selection so it moves together.
pub(crate) fn group_selection(editor: &mut Editor, state: &mut UiState) {
    let selected: Vec<ClipId> = state.selected_clips.iter().copied().collect();
    match editor.group_clips(&selected) {
        Ok(count) => {
            state.info(format!("Grouped {count} clips"));
            state.needs_repaint = true;
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Ctrl+Shift+G: break up the groups the selection is in.
pub(crate) fn ungroup_selection(editor: &mut Editor, state: &mut UiState) {
    let selected: Vec<ClipId> = state.selected_clips.iter().copied().collect();
    match editor.ungroup_clips(&selected) {
        Ok(0) => state.info("Nothing selected is grouped"),
        Ok(_) => {
            // The selection stays as it was; the members are simply free to
            // be selected, moved and deleted on their own again.
            state.info("Ungrouped");
            state.needs_repaint = true;
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Trade places with the clip beside it, reporting either way.
pub(crate) fn swap_clip(
    editor: &mut Editor,
    state: &mut UiState,
    clip: ClipId,
    side: bettercut_editor_core::Neighbour,
) {
    match editor.swap_with_neighbour(clip, side) {
        Ok(_) => {
            state.info("Swapped");
            state.needs_repaint = true;
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Q and W: cut away the part of the selection — or of the clip under the
/// playhead, when nothing is selected — before or after the playhead, and
/// close the gap.
pub fn ripple_trim(editor: &mut Editor, state: &mut UiState, edge: TrimEdge) {
    let selected: Vec<ClipId> = state.selected_clips.iter().copied().collect();
    match editor.ripple_trim_to_playhead(&selected, edge) {
        Ok(0) => state.info("Put the playhead inside a clip to ripple trim it"),
        Ok(n) => {
            // A trimmed start is a new clip id for what is left, so the old
            // selection points at nothing.
            state.clear_selection();
            state.needs_repaint = true;
            state.info(format!("Ripple trimmed {n} clip(s)"));
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// The nearest clip edge after the playhead (`forward`) or before it, on any
/// track. `None` when there is nothing further that way.
pub fn next_cut(
    editor: &Editor,
    forward: bool,
) -> Option<bettercut_editor_core::foundation::TimelineTime> {
    use bettercut_editor_core::timeline::Clip;

    let sequence = editor.active_sequence()?;
    let playhead = editor.playhead();
    let edges = sequence
        .video_tracks
        .iter()
        .flat_map(|t| t.clips().iter().map(Clip::timeline))
        .chain(
            sequence
                .audio_tracks
                .iter()
                .flat_map(|t| t.clips().iter().map(Clip::timeline)),
        )
        .chain(
            sequence
                .text_tracks
                .iter()
                .flat_map(|t| t.clips().iter().map(Clip::timeline)),
        )
        .flat_map(|range| [range.start, range.end])
        // Markers are stops too: they are the places someone chose to mark.
        .chain(sequence.markers.iter().map(|m| m.time));

    if forward {
        edges.filter(|&t| t > playhead).min()
    } else {
        edges.filter(|&t| t < playhead).max()
    }
}

/// Titled groups of rows, as the sheet lays them out.
pub type Sections<'a, T> = &'a [(&'static str, Vec<T>)];

/// Sections split into `columns` columns of roughly equal rows, keeping their
/// order: a column is closed once the columns so far hold their share of the
/// rows. A section is never split across two columns, and no column is empty
/// unless there are fewer sections than columns.
pub fn split_columns<'a, T>(sections: Sections<'a, T>, columns: usize) -> Vec<Sections<'a, T>> {
    let columns = columns.max(1);
    let total: usize = sections.iter().map(|(_, rows)| rows.len()).sum();
    let mut out = Vec::with_capacity(columns);
    let mut start = 0;
    let mut taken = 0;
    for (index, (_, rows)) in sections.iter().enumerate() {
        let column = out.len() + 1;
        // Close the column before this section once the columns so far hold
        // their share — or when every column still to come needs a section.
        let remaining_sections = sections.len() - index;
        let remaining_columns = columns - out.len() - 1;
        if index > start
            && column < columns
            && (taken * columns >= total * column || remaining_sections <= remaining_columns)
        {
            out.push(&sections[start..index]);
            start = index;
        }
        taken += rows.len();
    }
    out.push(&sections[start..]);
    out
}

/// The widest an action's description runs before it wraps.
const ACTION_WIDTH: f32 = 210.0;

fn section_grid(ui: &mut egui::Ui, title: &str, rows: &[Shortcut]) {
    ui.label(egui::RichText::new(title).strong());
    egui::Grid::new(("shortcuts", title))
        .num_columns(2)
        // Tight rows: the whole sheet has to fit a laptop screen without
        // scrolling, and every shortcut added makes it taller.
        .spacing([16.0, 1.0])
        .striped(true)
        .show(ui, |ui| {
            for row in rows {
                ui.label(egui::RichText::new(row.keys).monospace());
                // Wrapped to a fixed width: three columns of unwrapped rows
                // ran past the right edge of a 1280-pixel screen.
                ui.scope(|ui| {
                    ui.set_max_width(ACTION_WIDTH);
                    ui.add(egui::Label::new(row.action).wrap());
                });
                ui.end_row();
            }
        });
    ui.add_space(4.0);
}

/// The Shortcuts window: every key in [`SECTIONS`], grouped, with a search box
/// so "how do I zoom" is a word typed rather than a list read.
pub fn help_window(ctx: &egui::Context, state: &mut UiState) {
    if !state.shortcuts_open {
        return;
    }
    let mut open = true;
    egui::Window::new("Keyboard shortcuts")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.shortcut_search)
                    .hint_text("Search: zoom, undo, split...")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);

            let found = matching(&state.shortcut_search);
            if found.is_empty() {
                ui.label(
                    egui::RichText::new("No shortcut matches that").color(crate::theme::DISABLED),
                );
            }
            // Three columns on a wide screen, so the whole sheet fits a laptop
            // screen at a glance; one long column needed scrolling to reach the
            // mouse gestures, and two ran out of height as keys were added.
            // Two on a narrow window, and the scroll area for one smaller still.
            let screen = ctx.content_rect();
            let columns = if screen.width() >= 1200.0 { 3 } else { 2 };
            egui::ScrollArea::vertical()
                .max_height(screen.height() * 0.8)
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        for column in split_columns(&found, columns) {
                            ui.vertical(|ui| {
                                for (title, rows) in column {
                                    section_grid(ui, title, rows);
                                }
                            });
                            ui.add_space(24.0);
                        }
                    });
                });
            ui.label(
                egui::RichText::new(
                    "The clip actions are also on the timeline's right-click menu.",
                )
                .small()
                .color(crate::theme::DISABLED),
            );
        });
    state.shortcuts_open = open && state.shortcuts_open;
}

/// Select every clip starting at or after `from`, on `track` or on every track,
/// replacing the selection. From zero, that is every clip.
pub(crate) fn select_from(
    editor: &Editor,
    state: &mut UiState,
    from: bettercut_editor_core::foundation::TimelineTime,
    track: Option<TrackId>,
) {
    let clips = editor.clips_starting_from(from, track);
    state.needs_repaint = true;
    if clips.is_empty() {
        state.clear_selection();
        state.info(
            if from == bettercut_editor_core::foundation::TimelineTime::ZERO {
                "No clips to select"
            } else {
                "No clips start after that point"
            },
        );
        return;
    }
    state.info(format!("Selected {} clip(s)", clips.len()));
    state.selected_clips = clips.into_iter().collect();
}

/// Cut: copy to the clipboard, then delete (§10).
///
/// One user-visible action, but two commands — the delete half is what goes in
/// the undo history, since the clipboard is not project state.
pub(crate) fn cut_selection(editor: &mut Editor, state: &mut UiState) {
    let selected: Vec<_> = state.selected_clips.iter().copied().collect();
    if selected.is_empty() {
        state.info("Nothing selected");
        return;
    }
    let copied = editor.copy_clips(&selected);
    if copied == 0 {
        state.error("Selected clips are no longer on the timeline");
        return;
    }
    delete_selection(editor, state);
}

pub(crate) fn copy_selection(editor: &mut Editor, state: &mut UiState) {
    let selected: Vec<_> = state.selected_clips.iter().copied().collect();
    match editor.copy_clips(&selected) {
        0 => state.info("Nothing selected to copy"),
        n => state.info(format!("Copied {n} clip(s)")),
    }
}

/// Take the look off the one selected clip.
///
/// One clip, deliberately. A selection of several has several looks, and
/// picking one of them to copy would be a coin toss the user cannot see.
/// Put down whatever tool is armed. Returns whether anything was.
///
/// Deliberately does *not* clear the selection, which is what Escape does in
/// some editors: losing a careful multi-selection to a key press aimed at a
/// mode is a worse surprise than a mode that needs one more press.
pub(crate) fn cancel_modes(state: &mut UiState) -> bool {
    let eyedropper = state.picking_key.take().is_some();
    if eyedropper {
        state.info("Eyedropper off");
    }
    // Leaving crop mode keeps the crop: Escape ends the tool, it does not undo
    // what the tool did. Undo is what undoes it.
    let cropping = state.cropping.take().is_some();
    if cropping {
        state.info("Crop finished");
    }
    let armed = eyedropper || cropping;
    if armed {
        state.needs_repaint = true;
    }
    armed
}

pub(crate) fn copy_look(editor: &mut Editor, state: &mut UiState) {
    let mut selected = state.selected_clips.iter().copied();
    let (Some(clip), None) = (selected.next(), selected.next()) else {
        state.error("Select one clip to copy a look from");
        return;
    };
    match editor.clip_look(clip) {
        Some(look) => {
            state.copied_look = Some(look);
            state.info("Look copied — paste it onto other clips");
        }
        None => state.error("A sound has no look to copy"),
    }
}

/// Put the copied look onto everything selected.
pub(crate) fn paste_look(editor: &mut Editor, state: &mut UiState) {
    let Some(look) = state.copied_look.clone() else {
        state.error("No look copied yet — Ctrl+Alt+C takes one off a clip");
        return;
    };
    let onto: Vec<_> = state.selected_clips.iter().copied().collect();
    match editor.paste_look(&look, onto) {
        Ok(0) => state.error("Select the clips to paste the look onto"),
        Ok(1) => state.info("Look pasted"),
        Ok(n) => state.info(format!("Look pasted onto {n} clips")),
        Err(err) => state.error(err.to_string()),
    }
}

pub(crate) fn paste_at_playhead(editor: &mut Editor, state: &mut UiState) {
    match editor.paste_at_playhead() {
        Ok(n) => {
            state.clear_selection();
            state.info(format!("Pasted {n} clip(s)"));
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// §76: split at the playhead. With a selection, only those clips are cut;
/// with none, whatever the playhead is over.
pub(crate) fn split_at_playhead(editor: &mut Editor, state: &mut UiState) {
    let selected: Vec<_> = state.selected_clips.iter().copied().collect();
    match editor.split_at_playhead(&selected) {
        Ok(0) => state.info("Nothing under the playhead to split"),
        Ok(n) => {
            // The old clip ids are gone; keeping them selected would leave the
            // inspector pointing at clips that no longer exist.
            state.clear_selection();
            state.info(format!("Split {n} clip(s)"));
        }
        Err(err) => state.error(err.to_string()),
    }
}

pub(crate) fn duplicate_selection(editor: &mut Editor, state: &mut UiState) {
    let selected: Vec<_> = state.selected_clips.iter().copied().collect();
    if selected.is_empty() {
        state.info("Nothing selected");
        return;
    }
    let mut done = 0;
    for clip in selected {
        match editor.duplicate_clip(clip) {
            Ok(()) => done += 1,
            // A duplicate lands right after the original, so it collides when
            // something is already there. Report rather than shuffling clips
            // the user did not ask to move.
            Err(err) => state.error(err.to_string()),
        }
    }
    if done > 0 {
        state.info(format!("Duplicated {done} clip(s)"));
    }
}

/// Delete and close the gap, one undo step for the whole selection (§10, §79).
pub(crate) fn ripple_delete_selection(editor: &mut Editor, state: &mut UiState) {
    let mut selected = with_partners(editor, &state.selected_clips);
    if selected.is_empty() {
        state.info("Nothing selected");
        return;
    }
    let Some(sequence) = editor.active_sequence().map(|s| s.id) else {
        return;
    };

    // Delete right-to-left: removing an earlier clip shifts the later ones, so
    // going the other way would move the targets out from under us.
    selected.sort_by_key(|c| {
        editor
            .clip_payload(*c)
            .map_or(i64::MAX, |p| p.start().ticks())
    });
    selected.reverse();

    let commands: Vec<Command> = selected
        .into_iter()
        .filter_map(|clip| {
            let track = editor.track_of(clip)?;
            Some(Command::RippleDeleteClip {
                sequence,
                track,
                clip,
            })
        })
        .collect();

    if commands.is_empty() {
        state.error("Selected clips are no longer on the timeline");
        state.clear_selection();
        return;
    }

    // One undo step (§79). This used to be one per clip, which made undoing a
    // multi-clip ripple a matter of pressing Ctrl+Z as many times as there were
    // clips — and a linked pair twice.
    let count = commands.len();
    match editor.dispatch_group(format!("Ripple Delete {count} Clip(s)"), commands) {
        Ok(()) => {
            state.clear_selection();
            state.info(format!("Ripple deleted {count} clip(s)"));
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Delete every selected clip as one undo step (§79).
pub(crate) fn delete_selection(editor: &mut Editor, state: &mut UiState) {
    if state.selected_clips.is_empty() {
        state.info("Nothing selected");
        return;
    }

    let Some(sequence_id) = editor.active_sequence().map(|s| s.id) else {
        return;
    };

    let mut commands = Vec::new();
    let mut missing = 0;
    for clip in with_partners(editor, &state.selected_clips) {
        // §26's titles have their own removal, because they are not a
        // `ClipPayload` a media track holds. Missing this is how pressing
        // Delete on a selected title used to report it as already gone.
        if editor.is_text_clip(clip) {
            match editor.active_sequence().and_then(|s| s.text_track_of(clip)) {
                Some(track) => commands.push(Command::RemoveText {
                    sequence: sequence_id,
                    track,
                    clip,
                }),
                None => missing += 1,
            }
            continue;
        }

        // Adjustments too, for the same reason titles have the branch above:
        // falling through to `RemoveClip` would report a selected adjustment
        // as already gone.
        if let Some(track) = editor
            .active_sequence()
            .and_then(|s| s.adjustment_track_of(clip))
        {
            commands.push(Command::RemoveAdjustment {
                sequence: sequence_id,
                track,
                clip,
            });
            continue;
        }

        match locate(editor, clip) {
            Some(track) => commands.push(Command::RemoveClip {
                sequence: sequence_id,
                track,
                clip,
            }),
            None => missing += 1,
        }
    }

    if commands.is_empty() {
        state.error("Selected clips are no longer on the timeline");
        state.clear_selection();
        return;
    }

    let count = commands.len();
    let label = if count == 1 {
        "Delete Clip".to_owned()
    } else {
        format!("Delete {count} Clips")
    };

    match editor.dispatch_group(label, commands) {
        Ok(()) => {
            state.clear_selection();
            state.info(if missing > 0 {
                format!("Deleted {count} clip(s); {missing} were already gone")
            } else {
                format!("Deleted {count} clip(s)")
            });
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// The selection plus every clip linked to something in it (§12), without
/// duplicates.
///
/// Deleting a video's picture and leaving its sound would leave audio playing
/// under whatever came next. The selection itself is not widened — the
/// Inspector would then show "2 clips selected" for every imported video and
/// lose its controls — only the edits reach the partners.
fn with_partners(editor: &Editor, selected: &std::collections::HashSet<ClipId>) -> Vec<ClipId> {
    let mut out: Vec<ClipId> = Vec::new();
    for clip in selected {
        for linked in editor.linked_with(*clip) {
            if !out.contains(&linked) {
                out.push(linked);
            }
        }
    }
    out
}

/// Which track holds a clip. Linear over tracks, not over clips — there are a
/// handful of tracks and potentially thousands of clips.
fn locate(editor: &Editor, clip: ClipId) -> Option<TrackId> {
    let sequence = editor.active_sequence()?;
    sequence
        .video_tracks
        .iter()
        .find(|t| t.get(clip).is_some())
        .map(|t| t.id)
        .or_else(|| {
            sequence
                .audio_tracks
                .iter()
                .find(|t| t.get(clip).is_some())
                .map(|t| t.id)
        })
}

#[cfg(test)]
mod escape_tests {
    use super::*;

    /// A control that puts the interface into a state has to say how to
    /// leave it. Arming the eyedropper disables the preview's handles, and the
    /// button that armed it is in a panel the user may have scrolled past.
    #[test]
    fn escape_puts_down_the_eyedropper() {
        let mut state = UiState::default();
        state.picking_key = Some(bettercut_editor_core::foundation::ClipId::new());

        assert!(cancel_modes(&mut state), "nothing was reported as armed");
        assert_eq!(state.picking_key, None, "the eyedropper stayed armed");
    }

    /// And with nothing armed it is not a key that does something else by
    /// surprise — notably it does not throw away a selection that took work to
    /// build, which is what Escape does in some editors.
    /// Crop mode is armed the same way and has to be left the same way: it
    /// hides the move, scale and rotate handles, so a user who cannot find the
    /// button again would otherwise have lost them.
    #[test]
    fn escape_leaves_crop_mode() {
        let mut state = UiState::default();
        state.cropping = Some(bettercut_editor_core::foundation::ClipId::new());

        assert!(
            cancel_modes(&mut state),
            "crop mode was not reported as armed"
        );
        assert_eq!(state.cropping, None, "crop mode stayed on");
    }

    #[test]
    fn escape_with_nothing_armed_leaves_the_selection_alone() {
        let mut state = UiState::default();
        let clip = bettercut_editor_core::foundation::ClipId::new();
        state.selected_clips.insert(clip);

        assert!(!cancel_modes(&mut state));
        assert!(
            state.selected_clips.contains(&clip),
            "Escape cleared the selection"
        );
    }
}
