//! The timeline's right-click menu (§58).
//!
//! Every item here is a second route to something [`crate::shortcuts`] already
//! does, and calls the very same function. That is deliberate: a menu is how a
//! user discovers what the keyboard can do, so the two must not drift, and each
//! item carries its shortcut as a hint.
//!
//! What the menu offers depends on what was clicked — a clip, a track header,
//! or empty canvas — because offering "Ripple Delete" over empty space would be
//! a dead entry, and an interface should explain itself rather than
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
            ContextTarget::Empty { at, track } => empty_menu(ui, editor, state, at, track),
        })
        .is_some();

    // Forget the target once the menu is gone, so a later left-click does not
    // reopen it against a stale clip — and keep a track name typed into it
    // rather than losing it with the menu.
    if !shown {
        state.context = None;
        commit_track_name(editor, state);
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
    if item(ui, "Ripple Trim Start to Playhead", "Q") {
        shortcuts::ripple_trim(editor, state, bettercut_editor_core::TrimEdge::Start);
    }
    if item(ui, "Ripple Trim End to Playhead", "W") {
        shortcuts::ripple_trim(editor, state, bettercut_editor_core::TrimEdge::End);
    }
    // A group is made from a selection of two or more, and broken up from any
    // one of its clips.
    let onto = selection_or(state, clip);
    if onto.len() > 1 && editor.group_of(clip).is_none() && item(ui, "Group Clips", "Ctrl+G") {
        shortcuts::group_selection(editor, state);
    }
    if editor.group_of(clip).is_some() && item(ui, "Ungroup", "Ctrl+Shift+G") {
        shortcuts::ungroup_selection(editor, state);
    }
    // Only the sides that have a clip to trade places with.
    for (label, keys, side) in [
        (
            "Swap with Previous Clip",
            "Alt+Left",
            bettercut_editor_core::Neighbour::Previous,
        ),
        (
            "Swap with Next Clip",
            "Alt+Right",
            bettercut_editor_core::Neighbour::Next,
        ),
    ] {
        if editor.neighbour_of(clip, side).is_some() && item(ui, label, keys) {
            shortcuts::swap_clip(editor, state, clip, side);
        }
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

    // A look is minutes of work, and the next twenty clips want the same
    // one. Beside Copy, because it is the same idea applied to the settings
    // rather than to the clip.
    if editor.video_clip(clip).is_some() && item(ui, "Copy Look", "Ctrl+Alt+C") {
        state.copied_look = editor.clip_look(clip);
        state.info("Look copied — paste it onto other clips");
    }
    // Stripping a clip back is the same edit as pasting, with a plain look
    // instead of a copied one — so it is one undo step and needs no machinery
    // of its own (§79).
    if editor.video_clip(clip).is_some() && item(ui, "Clear Look", "") {
        let onto = selection_or(state, clip);
        match editor.paste_look(&bettercut_editor_core::Editor::plain_look(), onto) {
            Ok(0) => state.error("Nothing there to clear"),
            Ok(1) => state.info("Look cleared"),
            Ok(n) => state.info(format!("Look cleared on {n} clips")),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }

    if let Some(look) = state.copied_look.clone() {
        let onto = selection_or(state, clip);
        if item(ui, "Paste Look", "Ctrl+Alt+V") {
            match editor.paste_look(&look, onto) {
                Ok(0) => state.error("Nothing there to take a look"),
                Ok(1) => state.info("Look pasted"),
                Ok(n) => state.info(format!("Look pasted onto {n} clips")),
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
        }
    }

    // Organising, not editing: beside Delete rather than among the looks.
    color_label_menu(ui, editor, state, clip);
    split_screen_menu(ui, editor, state, clip);
    pip_menu(ui, editor, state, clip);

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

    // §24: keys sit on exact ticks and the playhead is dragged in pixels, so
    // landing on one by hand is luck. Easing acts on the keys *under* the
    // playhead, which makes this the other half of that feature rather than a
    // convenience.
    if let Some(video) = editor.video_clip(clip)
        && !video.keyframes.is_empty()
    {
        let (back, forward) = (
            video.key_beside(editor.playhead(), false),
            video.key_beside(editor.playhead(), true),
        );
        for (label, target) in [("Previous Keyframe", back), ("Next Keyframe", forward)] {
            if ui
                .add_enabled(target.is_some(), egui::Button::new(label))
                .clicked()
                && let Some(at) = target
            {
                ui.close();
                editor.set_playhead(at);
                state.needs_repaint = true;
            }
        }
    }

    // §24: how the keys under the playhead ease. Linear keys are what a
    // keyframe gets when it is placed without a choice, and a whole animation
    // of them is what makes a move look mechanical.
    if editor
        .video_clip(clip)
        .is_some_and(|clip| !clip.keyframes.is_empty())
    {
        ui.menu_button("Keyframe Easing", |ui| {
            for easing in bettercut_editor_core::timeline::Interpolation::PRESETS {
                if ui.button(easing.label()).clicked() {
                    ui.close();
                    match editor.set_keyframe_easing(clip, easing) {
                        Ok(0) => state.error("No keyframe under the playhead"),
                        Ok(_) => state.info(format!("Keys here are now {}", easing.label())),
                        Err(err) => state.error(err.to_string()),
                    }
                    state.needs_repaint = true;
                }
            }
        });
    }

    // A mirror, on the clip rather than in the Inspector, because that is where
    // the hand already is when a shot turns out to face the wrong way. The same
    // command either way (§11: one intention, one history entry).
    if let Some(video) = editor.video_clip(clip) {
        let transform = video.transform;
        let mut flip = None;
        for axis in bettercut_editor_core::timeline::FlipAxis::ALL {
            let on = axis.is_set(&transform);
            if ui
                .button(if on {
                    format!("{} ✓", axis.label())
                } else {
                    axis.label().to_string()
                })
                .clicked()
            {
                ui.close();
                flip = Some((axis, !on));
            }
        }
        if let Some((axis, on)) = flip {
            match editor.set_clip_property(
                clip,
                bettercut_editor_core::ClipProperty::Flip { axis, on },
                false,
            ) {
                Ok(()) => state.needs_repaint = true,
                Err(err) => state.error(err.to_string()),
            }
        }
    }

    // Both of these belong to a picture: a held frame needs a frame to hold,
    // and a transition needs footage either side of the cut. Offered on a title
    // or an adjustment, one failed with an error after the click and the other
    // listed nothing that could be chosen — so they are only offered where they
    // can do something.
    if editor.video_clip(clip).is_some() {
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

        if editor.can_retime(clip) {
            let reversed = editor.is_reversed(clip);
            if ui
                .button(if reversed { "Play Forwards" } else { "Reverse" })
                .on_hover_text("Play the clip backwards, with its sound")
                .clicked()
            {
                ui.close();
                match editor.set_reversed(clip, !reversed) {
                    Ok(()) => state.info(if reversed {
                        "Playing forwards"
                    } else {
                        "Reversed"
                    }),
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
            ui.menu_button("Speed Ramp", |ui| {
                if crate::panels::speed_ramp_buttons(ui, editor, state, clip) {
                    ui.close();
                }
            })
            .response
            .on_hover_text("Speed up and slow down across the clip");
        }

        ui.separator();
        transition_menu(ui, editor, state, clip);
    }

    // A better take, or the real logo in place of the placeholder: the clip
    // keeps its cut, look and keys. Only files that could go in are listed.
    replace_menu(ui, editor, state, clip);

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
    // hand (Milestone 12).
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
            .button("Cut on Beats")
            .on_hover_text("Split this clip at every beat of its music")
            .clicked()
    {
        ui.close();
        cut_on_beats(editor, state, clip, sound);
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
/// What a menu item acts on: the whole selection, or the clip it was opened on.
///
/// A menu opened on a clip *inside* a selection acts on the selection — that is
/// what the count at the top of the menu is telling the user. Opened on a clip
/// outside one, it acts on that clip alone, because otherwise a right-click
/// would quietly edit clips somewhere else on the timeline.
/// Arrange the selected picture clips side by side, stacked or in a grid.
///
/// Offered once more than one clip is selected, with each layout enabled only
/// for the number of clips it takes — a disabled entry that says why is how
/// the user learns the rule, where a refusal after the click would not be.
fn split_screen_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, clip: ClipId) {
    use bettercut_editor_core::SplitLayout;

    let onto = selection_or(state, clip);
    let pictures = onto
        .iter()
        .filter(|id| editor.video_clip(**id).is_some())
        .count();
    if pictures < 2 {
        return;
    }
    ui.menu_button("Split Screen", |ui| {
        for layout in SplitLayout::ALL {
            let fits = layout.clips() == pictures;
            if ui
                .add_enabled(fits, egui::Button::new(layout.label()))
                .on_disabled_hover_text(format!(
                    "Takes {} picture clips; {pictures} are selected",
                    layout.clips()
                ))
                .clicked()
            {
                ui.close();
                match editor.apply_split_screen(&onto, layout) {
                    Ok(()) => {
                        state.info(format!("Arranged {pictures} clips: {}", layout.label()));
                        state.needs_repaint = true;
                    }
                    Err(err) => state.error(err.to_string()),
                }
            }
        }
    })
    .response
    .on_hover_text("Put the selected shots on screen together");
}

/// Shrink a picture into a corner: a size row, the four corners two by two,
/// and a way back to full frame.
fn pip_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, clip: ClipId) {
    use bettercut_editor_core::{PipCorner, PipSize};

    if editor.video_clip(clip).is_none() {
        return;
    }
    let current = editor.picture_in_picture_of(clip);
    ui.menu_button("Picture in Picture", |ui| {
        // The size that is on, when the clip is already inset — so moving it
        // to another corner keeps its size.
        let mut size = current.map_or(state.pip_size, |(_, size)| size);
        ui.horizontal(|ui| {
            for option in PipSize::ALL {
                if ui
                    .selectable_label(size == option, option.label())
                    .clicked()
                {
                    size = option;
                    state.pip_size = option;
                    if let Some((corner, _)) = current {
                        apply_pip(editor, state, clip, corner, option);
                    }
                }
            }
        });
        ui.separator();
        egui::Grid::new(("pip corners", clip)).show(ui, |ui| {
            for (index, corner) in PipCorner::ALL.into_iter().enumerate() {
                let on = current.is_some_and(|(at, _)| at == corner);
                if ui.selectable_label(on, corner.label()).clicked() {
                    ui.close();
                    apply_pip(editor, state, clip, corner, size);
                }
                if index % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        ui.separator();
        if ui
            .button("Full Frame")
            .on_hover_text("Fill the frame again: no crop, normal size, centred")
            .clicked()
        {
            ui.close();
            match editor.reset_to_full_frame(clip) {
                Ok(()) => state.info("Back to full frame"),
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
        }
    })
    .response
    .on_hover_text("Shrink this shot into a corner — put it on a track above the main shot");
}

fn apply_pip(
    editor: &mut Editor,
    state: &mut UiState,
    clip: ClipId,
    corner: bettercut_editor_core::PipCorner,
    size: bettercut_editor_core::PipSize,
) {
    match editor.apply_picture_in_picture(clip, corner, size) {
        Ok(()) => state.info(format!(
            "{} inset, {}",
            size.label(),
            corner.label().to_lowercase()
        )),
        Err(err) => state.error(err.to_string()),
    }
    state.needs_repaint = true;
}

/// Tag the clip — or the selection it is part of — with a colour.
fn color_label_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, clip: ClipId) {
    use bettercut_editor_core::timeline::ColorLabel;

    let current = editor.color_label(clip).unwrap_or_default();
    ui.menu_button("Colour Label", |ui| {
        for label in ColorLabel::ALL {
            let swatch = label.rgb().map_or(crate::theme::DISABLED, |[r, g, b]| {
                egui::Color32::from_rgb(r, g, b)
            });
            let text = egui::RichText::new(label.name()).color(swatch).strong();
            if ui.selectable_label(current == label, text).clicked() {
                ui.close();
                let onto = selection_or(state, clip);
                match editor.set_color_label(&onto, label) {
                    Ok(_) => state.needs_repaint = true,
                    Err(err) => state.error(err.to_string()),
                }
            }
        }
    })
    .response
    .on_hover_text("Tag clips with a colour to find your way round the edit");
}

fn selection_or(state: &UiState, clip: ClipId) -> Vec<ClipId> {
    if state.selected_clips.contains(&clip) {
        state.selected_clips.iter().copied().collect()
    } else {
        vec![clip]
    }
}

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

/// Split a clip at every beat of its music.
///
/// The two halves of this were already here and tested — where the beats are
/// (`beat_markers`) and cutting a clip at a list of instants
/// (`Editor::split_clip_at`) — so this is the joining, and the judgement about
/// how many cuts is too many.
///
/// The beats come from the *sound* and the cut is made on the clip the user
/// asked about, which carries its linked partner (§12): cutting a music video
/// on the beat means cutting the picture, not the song.
pub fn cut_on_beats(editor: &mut Editor, state: &mut UiState, clip: ClipId, sound: ClipId) {
    /// More cuts than this is not an edit anyone wants from one click.
    ///
    /// Three minutes at 120 bpm is 360 beats. Cutting there is legitimate, but
    /// a whole song of it is a thousand clips and an undo step nobody can see
    /// the end of — so the count is offered as a refusal rather than performed
    /// as a surprise.
    const MOST_CUTS: usize = 400;

    let Some(audio) = editor.audio_clip(sound).cloned() else {
        return;
    };
    let Some(waveform) = state.waveforms.get(audio.media_id) else {
        state.info("This clip's sound is still being analysed — try again in a moment");
        state.needs_repaint = true;
        return;
    };

    let Some((beats, bpm)) = bettercut_playback::beat_markers(&audio, &waveform) else {
        state.info("No steady beat found in this clip");
        state.needs_repaint = true;
        return;
    };
    if beats.len() > MOST_CUTS {
        state.info(format!(
            "{} beats here — too many to cut in one go. Trim the clip first.",
            beats.len()
        ));
        state.needs_repaint = true;
        return;
    }

    match editor.split_clip_at(clip, &beats) {
        Ok(0) => state.info("No beats fall inside this clip"),
        Ok(cuts) => state.info(format!("Cut into {} at about {bpm:.0} bpm", cuts + 1)),
        Err(err) => state.error(err.to_string()),
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
            // crossfade says why instead of failing after the click.
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

/// Apply a track name typed into the menu, if there is one waiting.
fn commit_track_name(editor: &mut Editor, state: &mut UiState) {
    let Some((track, name)) = state.track_name_draft.take() else {
        return;
    };
    match editor.rename_track(track, &name) {
        Ok(true) => state.info(format!("Track renamed to {}", name.trim())),
        Ok(false) => {}
        Err(err) => state.error(err.to_string()),
    }
    state.needs_repaint = true;
}

fn track_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, track: TrackId) {
    use bettercut_editor_core::timeline::TrackKind;

    let Some(sequence) = editor.active_sequence() else {
        return;
    };
    let (Some(name), Some(kind)) = (
        sequence.track_name(track).map(str::to_owned),
        sequence.track_kind(track),
    ) else {
        return;
    };
    let is_sound = kind == TrackKind::Audio;
    let enabled = editor.track_flag(track, TrackFlag::Enabled);
    let locked = editor.track_flag(track, TrackFlag::Locked);
    let mix = sequence
        .audio_tracks
        .iter()
        .find(|t| t.id == track)
        .map(|t| (t.gain, t.pan));

    // The name, editable in place: typed into a draft and applied when the
    // field is left, so a rename is one undo step rather than one a letter.
    let mut draft = match &state.track_name_draft {
        Some((id, text)) if *id == track => text.clone(),
        _ => name,
    };
    let field = ui.add(
        egui::TextEdit::singleline(&mut draft)
            .char_limit(Editor::MAX_TRACK_NAME)
            .desired_width(180.0)
            .hint_text("track name"),
    );
    if field.changed() {
        state.track_name_draft = Some((track, draft));
    }
    if field.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.track_name_draft = None;
        } else {
            commit_track_name(editor, state);
        }
    }
    ui.separator();

    // Sound tracks mute, the rest hide — one flag, two words, because
    // "Enabled" means nothing to someone looking for the mute button.
    let toggle = match (is_sound, enabled) {
        (false, true) => "Hide Track",
        (false, false) => "Show Track",
        (true, true) => "Mute Track",
        (true, false) => "Unmute Track",
    };
    if item(ui, toggle, "")
        && let Err(err) = editor.set_track_flag(track, TrackFlag::Enabled, !enabled)
    {
        state.error(err.to_string());
    }

    // §20a.4: the point of solo is that it comes off in one click. Muting
    // three lanes to hear the fourth means putting three back afterwards and
    // hoping you remember which were already off.
    let soloed = editor.track_flag(track, TrackFlag::Solo);
    if item(ui, if soloed { "Unsolo Track" } else { "Solo Track" }, "")
        && let Err(err) = editor.set_track_flag(track, TrackFlag::Solo, !soloed)
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

    if item(ui, "Duplicate Track", "") {
        match editor.duplicate_track(track) {
            Ok(copy) => {
                state.selected_track = Some(copy);
                state.info("Track duplicated, with its clips");
            }
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }
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
    track: Option<bettercut_editor_core::foundation::TrackId>,
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

    // The opposite, on one track: in this menu because a gap is exactly the
    // empty space that was right-clicked.
    if let Some(track) = track {
        gap_items(ui, editor, state, track, at);
    }

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

    if item(ui, "Select All", "Ctrl+A") {
        shortcuts::select_from(
            editor,
            state,
            bettercut_editor_core::foundation::TimelineTime::ZERO,
            None,
        );
    }
    if item(ui, "Select All After Here", "") {
        shortcuts::select_from(editor, state, at, None);
    }
    if let Some(track) = track
        && item(ui, "Select All After Here on This Track", "")
    {
        shortcuts::select_from(editor, state, at, Some(track));
    }
    if item(ui, "Deselect All", "") {
        state.clear_selection();
        state.needs_repaint = true;
    }
}

/// "Replace With", listing the project's files that could go in `clip`.
fn replace_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, clip: ClipId) {
    let candidates: Vec<(bettercut_editor_core::foundation::MediaId, String)> = editor
        .replacement_candidates(clip)
        .into_iter()
        .map(|asset| (asset.id, asset.file_name.clone()))
        .collect();
    if candidates.is_empty() {
        return;
    }
    ui.menu_button("Replace With", |ui| {
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for (media, name) in candidates {
                    if ui.button(&name).clicked() {
                        ui.close();
                        use bettercut_editor_core::replace::SoundChange;
                        match editor.replace_media(clip, media) {
                            Ok(change) => state.info(match change {
                                SoundChange::Removed => {
                                    format!("Now showing {name}; it has no sound, so the old sound was removed")
                                }
                                SoundChange::Added => format!("Now showing {name}, with its sound"),
                                SoundChange::Unlinked => {
                                    format!("Now playing {name}, untied from the picture")
                                }
                                SoundChange::Replaced | SoundChange::None => {
                                    format!("Replaced with {name}")
                                }
                            }),
                            Err(err) => state.error(err.to_string()),
                        }
                        state.needs_repaint = true;
                    }
                }
            });
    })
    .response
    .on_hover_text("Put another file in this clip, keeping its length, look, keys and speed");
}

/// "Close Gap" when the click was in one, and "Close All Gaps" when the track
/// has others. Neither shows on a track with no gaps, so the menu never offers
/// an edit that would only report there was nothing to do.
fn gap_items(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    track: bettercut_editor_core::foundation::TrackId,
    at: bettercut_editor_core::foundation::TimelineTime,
) {
    let in_gap = editor.gap_at(track, at).is_some();
    if in_gap
        && ui
            .button("Close Gap")
            .on_hover_text("Pull the clips after this gap left to fill it, with their linked sound")
            .clicked()
    {
        ui.close();
        match editor.close_gap(track, at) {
            Ok(moved) => state.info(format!("Gap closed; {moved} clip(s) moved")),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }
    // One gap, and it is the one clicked: "Close Gap" already says it all.
    let gaps = editor.gaps_on(track).len();
    if (gaps > 1 || (gaps == 1 && !in_gap))
        && ui
            .button(if gaps == 1 {
                "Close the Gap on This Track".to_owned()
            } else {
                format!("Close All {gaps} Gaps on This Track")
            })
            .on_hover_text("Butt every clip on this track up against the one before it")
            .clicked()
    {
        ui.close();
        match editor.close_all_gaps(track) {
            Ok(closed) if closed == gaps => state.info(format!("Closed {closed} gap(s)")),
            Ok(closed) => state.info(format!(
                "Closed {closed} of {gaps} gaps; the rest would push linked clips into others"
            )),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A right-click inside a selection acts on the selection — that is what
    /// the "n clips selected" line at the top of the menu is promising.
    #[test]
    fn a_menu_opened_inside_a_selection_acts_on_all_of_it() {
        let mut state = UiState::default();
        let (a, b) = (ClipId::new(), ClipId::new());
        state.selected_clips.insert(a);
        state.selected_clips.insert(b);

        let mut onto = selection_or(&state, a);
        onto.sort();
        let mut both = vec![a, b];
        both.sort();
        assert_eq!(onto, both);
    }

    /// And outside one it acts on the clip under the pointer alone. Otherwise a
    /// right-click would quietly edit clips elsewhere on the timeline, which
    /// the user cannot even see from where they are looking.
    #[test]
    fn a_menu_opened_outside_a_selection_acts_on_that_clip_alone() {
        let mut state = UiState::default();
        let (selected, other) = (ClipId::new(), ClipId::new());
        state.selected_clips.insert(selected);

        assert_eq!(selection_or(&state, other), vec![other]);
    }

    #[test]
    fn nothing_selected_acts_on_the_clip_under_the_pointer() {
        let state = UiState::default();
        let clip = ClipId::new();
        assert_eq!(selection_or(&state, clip), vec![clip]);
    }
}
