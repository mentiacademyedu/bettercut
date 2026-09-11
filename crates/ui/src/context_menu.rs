//! The timeline's right-click menu (§58, §41).
//!
//! Every item here is a second route to something [`crate::shortcuts`] already
//! does, and calls the very same function. That is deliberate: a menu is how a
//! user discovers what the keyboard can do, so the two must not drift, and each
//! item carries its shortcut as a hint.
//!
//! What the menu offers depends on what was clicked — a clip, a track header,
//! or empty canvas — because offering "Ripple Delete" over empty space would be
//! a dead entry, and §41 asks for an interface that explains itself rather than
//! one that lists everything and greys most of it out.

use bettercut_editor_core::foundation::{ClipId, TrackId};
use bettercut_editor_core::timeline::{MIN_TRANSITION, TransitionKind};
use bettercut_editor_core::{Editor, TrackFlag};

use crate::shortcuts;
use crate::state::{ContextTarget, UiState};

/// Draw the menu, if a right-click opened one.
///
/// Takes the timeline's own `Response`; egui anchors the popup at the pointer
/// and handles dismissal.
pub fn show(response: &egui::Response, editor: &mut Editor, state: &mut UiState) {
    // No target means the click never landed on the timeline.
    let Some(target) = state.context else {
        return;
    };

    let shown = egui::Popup::context_menu(response)
        .show(|ui| match target {
            ContextTarget::Clip { clip, .. } => clip_menu(ui, editor, state, clip),
            ContextTarget::TrackHeader { track } => track_menu(ui, editor, state, track),
            ContextTarget::Empty { at } => empty_menu(ui, editor, state, at),
        })
        .is_some();

    // Forget the target once the menu is gone, so a later left-click does not
    // reopen it against a stale clip.
    if !shown {
        state.context = None;
    }
}

/// One menu row: label on the left, shortcut greyed on the right.
fn item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    let clicked = ui
        .add(
            egui::Button::new(label)
                .shortcut_text(egui::RichText::new(shortcut).color(crate::theme::DISABLED)),
        )
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn clip_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, clip: ClipId) {
    let count = state.selected_clips.len();
    // Say how many will be affected, so a menu opened on one clip inside a
    // multi-selection cannot be mistaken for acting on just that clip.
    ui.label(
        egui::RichText::new(if count > 1 {
            format!("{count} clips selected")
        } else {
            "Clip".to_owned()
        })
        .small()
        .color(crate::theme::DISABLED),
    );
    ui.separator();

    if item(ui, "Split at Playhead", "S") {
        shortcuts::split_at_playhead(editor, state);
    }

    ui.separator();

    if item(ui, "Cut", "Ctrl+X") {
        shortcuts::cut_selection(editor, state);
    }
    if item(ui, "Copy", "Ctrl+C") {
        shortcuts::copy_selection(editor, state);
    }
    if item(ui, "Duplicate", "Ctrl+D") {
        shortcuts::duplicate_selection(editor, state);
    }

    ui.separator();

    if item(ui, "Delete", "Del") {
        shortcuts::delete_selection(editor, state);
    }
    if ui
        .add(
            egui::Button::new("Ripple Delete")
                .shortcut_text(egui::RichText::new("Shift+Del").color(crate::theme::DISABLED)),
        )
        .on_hover_text("Delete and close the gap, pulling later clips left")
        .clicked()
    {
        ui.close();
        shortcuts::ripple_delete_selection(editor, state);
    }

    ui.separator();
    transition_menu(ui, editor, state, clip);

    // §12: only offered when there is something to unlink. A disabled entry
    // on every title and every silent clip would be noise.
    if editor.link_of(clip).is_some()
        && ui
            .button("Unlink Audio")
            .on_hover_text("Let the picture and its sound move, trim and re-time independently")
            .clicked()
    {
        ui.close();
        match editor.unlink(clip) {
            Ok(()) => state.info("Picture and sound unlinked"),
            Err(err) => state.error(err.to_string()),
        }
    }
    ui.separator();

    // Jumping to a clip's edges is what makes trimming to a neighbour precise,
    // and there is no keyboard route to it yet.
    if item(ui, "Playhead to Clip Start", "")
        && let Some(payload) = editor.clip_payload(clip)
    {
        editor.set_playhead(payload.start());
        state.needs_repaint = true;
    }
}

/// Transitions on the cut at the end of this clip (§25).
///
/// On the clip rather than on the cut because that is where the model keeps it,
/// and because a cut is not something you can right-click: it is a boundary one
/// pixel wide.
fn transition_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, clip: ClipId) {
    let existing = editor.video_clip(clip).and_then(|c| c.transition_out);

    ui.menu_button("Transition", |ui| {
        for kind in TransitionKind::ALL {
            // Ask what the cut can take before offering it, so an unavailable
            // crossfade says why instead of failing after the click (§41).
            let room = editor.transition_room(clip, kind);
            let available = room.is_some_and(|r| r >= MIN_TRANSITION);
            let chosen = existing.is_some_and(|t| t.kind == kind);

            let button = egui::Button::new(if chosen {
                format!("✔  {}", kind.label())
            } else {
                format!("     {}", kind.label())
            });
            let response = ui
                .add_enabled(available, button)
                .on_hover_text(if available {
                    kind.description()
                } else if room.is_none() {
                    "There is no clip straight after this one to fade into."
                } else {
                    "Not enough spare footage either side of the cut."
                });

            if response.clicked() {
                ui.close();
                if let Err(err) = editor.set_transition(clip, kind) {
                    state.error(err.to_string());
                }
            }
        }

        ui.separator();
        if ui
            .add_enabled(existing.is_some(), egui::Button::new("     Remove"))
            .clicked()
        {
            ui.close();
            if let Err(err) = editor.remove_transition(clip) {
                state.error(err.to_string());
            }
        }
    });
}

fn track_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, track: TrackId) {
    let Some(sequence) = editor.active_sequence() else {
        return;
    };
    let (name, enabled, locked, is_video) = {
        let video = sequence.video_tracks.iter().find(|t| t.id == track);
        let audio = sequence.audio_tracks.iter().find(|t| t.id == track);
        match (video, audio) {
            (Some(t), _) => (t.name.clone(), t.enabled, t.locked, true),
            (_, Some(t)) => (t.name.clone(), t.enabled, t.locked, false),
            _ => return,
        }
    };

    ui.label(
        egui::RichText::new(name)
            .small()
            .color(crate::theme::DISABLED),
    );
    ui.separator();

    // Video tracks hide, audio tracks mute — one flag, two words, because
    // "Enabled" means nothing to someone looking for the mute button.
    let toggle = match (is_video, enabled) {
        (true, true) => "Hide Track",
        (true, false) => "Show Track",
        (false, true) => "Mute Track",
        (false, false) => "Unmute Track",
    };
    if item(ui, toggle, "")
        && let Err(err) = editor.set_track_flag(track, TrackFlag::Enabled, !enabled)
    {
        state.error(err.to_string());
    }

    let lock_label = if locked { "Unlock Track" } else { "Lock Track" };
    if item(ui, lock_label, "")
        && let Err(err) = editor.set_track_flag(track, TrackFlag::Locked, !locked)
    {
        state.error(err.to_string());
    }

    ui.separator();

    if item(ui, "Add Video Track", "") {
        let name = format!(
            "V{}",
            editor
                .active_sequence()
                .map_or(1, |s| s.video_tracks.len() + 1)
        );
        if let Err(err) = editor.add_video_track(name) {
            state.error(err.to_string());
        }
    }
    if item(ui, "Add Audio Track", "") {
        let name = format!(
            "A{}",
            editor
                .active_sequence()
                .map_or(1, |s| s.audio_tracks.len() + 1)
        );
        if let Err(err) = editor.add_audio_track(name) {
            state.error(err.to_string());
        }
    }

    ui.separator();

    // Removing a track takes its clips with it. That is undoable — the whole
    // track is kept in the undo payload — but it is still the one destructive
    // entry here, so it is coloured and sits alone at the bottom.
    if ui
        .add(egui::Button::new(
            egui::RichText::new("Remove Track").color(crate::theme::ERROR_TEXT),
        ))
        .on_hover_text("Removes the track and every clip on it. Undoable.")
        .clicked()
    {
        ui.close();
        let Some(sequence) = editor.active_sequence().map(|s| s.id) else {
            return;
        };
        match editor.dispatch(bettercut_editor_core::Command::RemoveTrack { sequence, track }) {
            Ok(()) => {
                state.clear_selection();
                state.selected_track = None;
                state.info("Track removed");
            }
            Err(err) => state.error(err.to_string()),
        }
    }
}

fn empty_menu(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    at: bettercut_editor_core::foundation::TimelineTime,
) {
    if item(ui, "Paste at Playhead", "Ctrl+V") {
        shortcuts::paste_at_playhead(editor, state);
    }

    ui.separator();

    if item(ui, "Move Playhead Here", "") {
        editor.set_playhead(at);
        state.needs_repaint = true;
    }
    if item(ui, "Split at Playhead", "S") {
        shortcuts::split_at_playhead(editor, state);
    }

    ui.separator();

    if item(ui, "Select All", "") {
        select_all(editor, state);
    }
    if item(ui, "Deselect All", "") {
        state.clear_selection();
        state.needs_repaint = true;
    }
}

fn select_all(editor: &Editor, state: &mut UiState) {
    let Some(sequence) = editor.active_sequence() else {
        return;
    };
    state.selected_clips = sequence
        .video_tracks
        .iter()
        .flat_map(|t| t.clips().iter().map(|c| c.id))
        .chain(
            sequence
                .audio_tracks
                .iter()
                .flat_map(|t| t.clips().iter().map(|c| c.id)),
        )
        .collect();
    state.needs_repaint = true;
}
