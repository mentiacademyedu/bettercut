//! Keyboard shortcuts (§57).
//!
//! The keys are listed once, in [`SHORTCUTS`], which is also what the
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

use crate::panels;
use crate::state::UiState;

/// Every shortcut, as the Shortcuts window lists it: keys, then what they do.
///
/// One table, next to the handler, so the list a user reads and the keys that
/// actually work are edited in the same place.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Space", "Play / pause"),
    ("Left / Right", "Step one frame (Shift: ten)"),
    ("Up / Down", "Jump to the previous / next cut or marker"),
    ("Home / End", "Jump to the start / end"),
    ("S", "Split at the playhead"),
    ("M", "Add / remove a marker at the playhead"),
    (", and .", "Nudge the selection a frame (Shift: ten)"),
    (
        "[ and ]",
        "Trim the selection's start / end to the playhead",
    ),
    ("Delete", "Delete the selection"),
    ("Shift + Delete", "Delete and close the gap"),
    ("Ctrl + X / C / V", "Cut / copy / paste"),
    ("Ctrl + D", "Duplicate"),
    ("Ctrl + Z", "Undo"),
    ("Ctrl + Shift + Z", "Redo"),
    ("Ctrl + S", "Save"),
    ("N", "Snapping on / off"),
    ("? or F1", "This list"),
];

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

    let (keys, modifiers) = ctx.input(|i| {
        let pressed: Vec<egui::Key> = [
            egui::Key::Space,
            egui::Key::Z,
            egui::Key::S,
            egui::Key::C,
            egui::Key::V,
            egui::Key::X,
            egui::Key::D,
            egui::Key::N,
            egui::Key::M,
            egui::Key::Comma,
            egui::Key::Period,
            egui::Key::OpenBracket,
            egui::Key::CloseBracket,
            egui::Key::Delete,
            egui::Key::Backspace,
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::Home,
            egui::Key::End,
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
            egui::Key::F1,
            egui::Key::Questionmark,
        ]
        .into_iter()
        .filter(|k| i.key_pressed(*k))
        .collect();
        (pressed, i.modifiers)
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

            egui::Key::Z if modifiers.command && modifiers.shift => match editor.redo() {
                Ok(()) => state.needs_repaint = true,
                Err(err) => state.info(err.to_string()),
            },
            egui::Key::Z if modifiers.command => match editor.undo() {
                Ok(()) => state.needs_repaint = true,
                Err(err) => state.info(err.to_string()),
            },

            egui::Key::S if modifiers.command => panels::save_project(editor, state),
            egui::Key::S => split_at_playhead(editor, state),

            egui::Key::X if modifiers.command => cut_selection(editor, state),
            egui::Key::C if modifiers.command => copy_selection(editor, state),
            egui::Key::V if modifiers.command => paste_at_playhead(editor, state),
            egui::Key::D if modifiers.command => duplicate_selection(editor, state),

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

/// The Shortcuts window: every key in [`SHORTCUTS`], in one place.
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
            egui::Grid::new("shortcuts")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    for (keys, action) in SHORTCUTS {
                        ui.label(egui::RichText::new(*keys).monospace().strong());
                        ui.label(*action);
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
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
