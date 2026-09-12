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
use bettercut_editor_core::timeline::{AnimatedParameter, MIN_TRANSITION, TransitionKind};
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
    if item(ui, "Trim Start to Playhead", "[") {
        shortcuts::trim_selection(editor, state, bettercut_editor_core::TrimEdge::Start);
    }
    if item(ui, "Trim End to Playhead", "]") {
        shortcuts::trim_selection(editor, state, bettercut_editor_core::TrimEdge::End);
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

    // A held frame, with room made for it so the sound keeps its place (§10).
    ui.menu_button("Freeze Frame", |ui| {
        for seconds in [1, 2, 3] {
            if ui.button(format!("{seconds} s")).clicked() {
                ui.close();
                let duration =
                    bettercut_editor_core::foundation::TimelineTime::from_seconds(seconds);
                match editor.freeze_frame(clip, duration) {
                    Ok(held) => {
                        state.select_only(held);
                        state.info(format!("Holding this frame for {seconds} s"));
                    }
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
        }
    })
    .response
    .on_hover_text("Hold the frame under the playhead, making room for it");

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
    // Beats, for any clip with sound: its own, or the sound linked to it.
    if let Some(sound) = sound_of(editor, clip)
        && ui
            .button("Mark Beats")
            .on_hover_text("Put a marker on every beat of this clip's music, to cut on")
            .clicked()
    {
        ui.close();
        mark_beats(editor, state, sound);
    }
    // Cuts, for picture: a file that already has cuts in it — a download, a
    // screen recording, last year's export — takes a scrub per cut to find by
    // hand (§45).
    if editor.video_clip(clip).is_some_and(|clip| !clip.frozen)
        && ui
            .button("Find Cuts…")
            .on_hover_text("Look through this clip for scene changes and offer to split it there")
            .clicked()
    {
        ui.close();
        state.scene_request = Some(clip);
    }
    if let Some(sound) = sound_of(editor, clip)
        && ui
            .button("Normalise Volume")
            .on_hover_text("Set this clip's level so its loudest moment sits just under full scale")
            .clicked()
    {
        ui.close();
        normalise_volume(editor, state, sound);
    }

    // Ducking, for a clip with sound that has something else playing over it.
    if let Some(sound) = sound_of(editor, clip) {
        let ducked = editor
            .audio_clip(sound)
            .is_some_and(|clip| clip.keyframes.is_animated(AnimatedParameter::Gain));
        if ducked {
            if ui
                .button("Clear Ducking")
                .on_hover_text("Put this clip's volume back where it was")
                .clicked()
            {
                ui.close();
                match editor.set_gain_envelope(sound, &[], false) {
                    Ok(_) => state.info("Volume envelope cleared"),
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
        } else if ui
            .button("Duck Under Voice")
            .on_hover_text("Dip this clip's volume wherever something else is speaking over it")
            .clicked()
        {
            ui.close();
            duck_under_voice(editor, state, sound);
        }
    }
    if let Some(sound) = sound_of(editor, clip)
        && ui
            .button("Time Captions")
            .on_hover_text("Put an empty caption on each phrase in this clip, ready to type into")
            .clicked()
    {
        ui.close();
        time_captions(editor, state, sound);
    }
    if let Some(sound) = sound_of(editor, clip)
        && ui
            .button("Remove Silences…")
            .on_hover_text("Find the pauses in this clip and offer to cut them out")
            .clicked()
    {
        ui.close();
        match crate::silence_dialog::SilenceDialog::open(editor, &mut state.waveforms, sound) {
            Some(dialog) => state.silence = Some(dialog),
            None => state.info("This clip's sound is still being analysed — try again in a moment"),
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

/// The sound clip behind `clip`: itself, or what it is linked to (§12).
fn sound_of(editor: &Editor, clip: ClipId) -> Option<ClipId> {
    editor
        .linked_with(clip)
        .into_iter()
        .find(|c| editor.audio_clip(*c).is_some())
}

/// Mark the beats of a sound clip, from the waveform the timeline already has.
pub fn mark_beats(editor: &mut Editor, state: &mut UiState, sound: ClipId) {
    let Some(clip) = editor.audio_clip(sound).cloned() else {
        return;
    };
    let Some(waveform) = state.waveforms.get(clip.media_id) else {
        state.info("This clip's sound is still being analysed — try again in a moment");
        return;
    };
    match bettercut_playback::beat_markers(&clip, &waveform) {
        Some((times, bpm)) => match editor.add_markers(&times) {
            Ok(0) => state.info("Its beats are already marked"),
            Ok(added) => state.info(format!("Marked {added} beats, about {bpm:.0} bpm")),
            Err(err) => state.error(err.to_string()),
        },
        None => state.info("No steady beat found in this clip"),
    }
    state.needs_repaint = true;
}

/// Set a clip's gain so its loudest moment lands just under full scale.
///
/// The most ordinary problem in an edit — "this one is too quiet" — and the
/// answer is already in the waveform. It writes the clip's **static** gain
/// rather than an envelope: normalising is one level for the whole clip, and
/// putting it in the envelope would fight whatever ducking is there.
pub fn normalise_volume(editor: &mut Editor, state: &mut UiState, sound: ClipId) {
    let Some(clip) = editor.audio_clip(sound).cloned() else {
        return;
    };
    let Some(waveform) = state.waveforms.get(clip.media_id) else {
        state.info("This clip's sound is still being analysed — try again in a moment");
        state.needs_repaint = true;
        return;
    };

    match bettercut_playback::normalise(&clip, &waveform) {
        Some(gain) => {
            // Replaces whatever the gain was rather than multiplying it: the
            // peak is measured from the *source*, so this is the level that
            // puts it at the target however the clip was set before.
            let db = 20.0 * gain.max(1e-6).log10();
            match editor.set_clip_property(
                sound,
                bettercut_editor_core::ClipProperty::Gain(gain),
                false,
            ) {
                Ok(()) => state.info(format!("Level set to {db:+.1} dB")),
                Err(err) => state.error(err.to_string()),
            }
        }
        None => state.info("This clip is already at a good level, or too quiet to raise"),
    }
    state.needs_repaint = true;
}

/// Dip a clip's volume wherever something else is talking over it (§20a.4).
///
/// The speech comes from every *other* audio clip that overlaps this one, which
/// is what "under the voice" means on a timeline: the music ducks under
/// whatever else is playing, wherever it is playing, however many clips that
/// is.
pub fn duck_under_voice(editor: &mut Editor, state: &mut UiState, music: ClipId) {
    let Some(clip) = editor.audio_clip(music).cloned() else {
        return;
    };
    let Some(sequence) = editor.active_sequence() else {
        return;
    };

    let mut speech = Vec::new();
    let mut waiting = false;
    for track in &sequence.audio_tracks {
        for other in track.clips() {
            // Itself, and anything not playing at the same time, are not what
            // it is ducking under.
            if other.id == music || !other.timeline.overlaps(clip.timeline) {
                continue;
            }
            let Some(waveform) = state.waveforms.get(other.media_id) else {
                waiting = true;
                continue;
            };
            speech.extend(bettercut_playback::speech_ranges(
                other,
                &waveform,
                bettercut_playback::SpeechSettings::default(),
            ));
        }
    }

    if speech.is_empty() {
        state.info(if waiting {
            "Still analysing the other clips — try again in a moment"
        } else {
            "Nothing is speaking over this clip"
        });
        state.needs_repaint = true;
        return;
    }

    let points = bettercut_playback::duck_envelope(
        &clip,
        &speech,
        bettercut_playback::DuckSettings::default(),
    );
    match editor.set_gain_envelope(music, &points, false) {
        Ok(0) => state.info("Nothing to duck under on this clip"),
        Ok(_) => state.info("Ducked under the voice — the dips are on the clip"),
        Err(err) => state.error(err.to_string()),
    }
    state.needs_repaint = true;
}

/// One empty caption per phrase (§28).
///
/// The half of automatic captions that needs no model: finding *when* someone
/// speaks is in the waveform, and it is the tedious half — typing a sentence
/// takes a moment, finding the instant it starts takes a scrub, a nudge and
/// another scrub, thirty times over.
pub fn time_captions(editor: &mut Editor, state: &mut UiState, sound: ClipId) {
    let Some(clip) = editor.audio_clip(sound).cloned() else {
        return;
    };
    let Some(waveform) = state.waveforms.get(clip.media_id) else {
        state.info("This clip's sound is still being analysed — try again in a moment");
        return;
    };

    let ranges = bettercut_playback::speech_ranges(
        &clip,
        &waveform,
        bettercut_playback::SpeechSettings::default(),
    );
    if ranges.is_empty() {
        state.info("No speech found in this clip");
        state.needs_repaint = true;
        return;
    }

    let segments: Vec<bettercut_editor_core::captions::CaptionSegment> = ranges
        .iter()
        .map(|range| {
            bettercut_editor_core::captions::CaptionSegment::new(range.start, range.end, "")
        })
        .collect();
    let count = segments.len();
    match editor.replace_captions(segments, format!("Time {count} Captions")) {
        Ok(n) => {
            // Straight into the list: the captions are empty, and typing into
            // them is the rest of the job.
            state.captions_open = true;
            state.info(format!("{n} empty captions — type the words in the list"));
        }
        Err(err) => state.error(err.to_string()),
    }
    state.needs_repaint = true;
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
    let (name, enabled, locked, is_video, mix) = {
        let video = sequence.video_tracks.iter().find(|t| t.id == track);
        let audio = sequence.audio_tracks.iter().find(|t| t.id == track);
        match (video, audio) {
            (Some(t), _) => (t.name.clone(), t.enabled, t.locked, true, None),
            (_, Some(t)) => (
                t.name.clone(),
                t.enabled,
                t.locked,
                false,
                Some((t.gain, t.pan)),
            ),
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

    // §20a.4's track stage: the whole lane louder or quieter, and where it
    // sits between the speakers. In the menu rather than on the header, where
    // two sliders per lane would crowd out the clips they are beside.
    if let Some((mut gain, mut pan)) = mix {
        ui.separator();
        let volume = ui.add(
            egui::Slider::new(&mut gain, 0.0..=2.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .text("volume"),
        );
        let balance = ui.add(
            egui::Slider::new(&mut pan, -1.0..=1.0)
                .custom_formatter(|v, _| pan_label(v as f32))
                .text("pan"),
        );
        if (volume.changed() || balance.changed())
            && let Err(err) =
                editor.set_track_mix(track, gain, pan, volume.dragged() || balance.dragged())
        {
            state.error(err.to_string());
        }
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

    // A gap across every track, so what follows keeps its sync (§10). In the
    // empty-canvas menu because "make room here" is about a place on the
    // timeline rather than about a clip.
    ui.menu_button("Insert Gap Here", |ui| {
        for seconds in [1, 2, 5] {
            if ui.button(format!("{seconds} s")).clicked() {
                ui.close();
                let duration =
                    bettercut_editor_core::foundation::TimelineTime::from_seconds(seconds);
                match editor.insert_time(at, duration) {
                    Ok(0) => state.info("Nothing after that point to move"),
                    Ok(moved) => state.info(format!("Opened {seconds} s; {moved} clip(s) moved")),
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
        }
    })
    .response
    .on_hover_text("Move everything from here on to the right, on every track");

    if item(ui, "Add Marker Here", "") {
        match editor.toggle_marker(at) {
            // Toggling on an existing mark removes it, which is what a second
            // "add" at the same place is taken to mean.
            Ok(added) => state.info(if added {
                "Marker added"
            } else {
                "Marker removed"
            }),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }
    if !editor.markers().is_empty() && item(ui, "Clear All Markers", "") {
        match editor.clear_markers() {
            Ok(()) => state.info("Markers cleared"),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
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

/// A pan position as people say it: "centre", "L40", "R100".
pub fn pan_label(pan: f32) -> String {
    let percent = (pan * 100.0).round() as i32;
    match percent {
        0 => "centre".to_owned(),
        p if p < 0 => format!("L{}", -p),
        p => format!("R{p}"),
    }
}
