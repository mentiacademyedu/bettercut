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
                .shortcut_text(egui::RichText::new(shortcut).color(crate::theme::disabled())),
        )
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn clip_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, clip: ClipId) {
    let count = state.selected_clips.len();

    // A name of the clip's own, typed right here: Enter keeps it, an empty
    // name gives the file's back.
    match state.clip_name_draft.as_mut() {
        Some((for_clip, draft)) if *for_clip == clip => {
            // With several selected, the name is a base and every one of
            // them is numbered after it in timeline order.
            let many = count > 1 && state.selected_clips.contains(&clip);
            let field = ui.add(
                egui::TextEdit::singleline(draft)
                    .hint_text(if many {
                        "Name, numbered 01, 02…"
                    } else {
                        "Clip name"
                    })
                    .desired_width(180.0),
            );
            field.request_focus();
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let typed = draft.clone();
                state.clip_name_draft = None;
                if many && !typed.trim().is_empty() {
                    let chosen: Vec<ClipId> = state.selected_clips.iter().copied().collect();
                    match editor.number_clips(&chosen, &typed) {
                        Ok(n) => {
                            state.needs_repaint = true;
                            state.info(format!("Named {n} clip(s) {} 01 onwards", typed.trim()));
                        }
                        Err(err) => state.error(err.to_string()),
                    }
                } else {
                    match editor.set_clip_name(clip, Some(&typed)) {
                        Ok(_) => state.needs_repaint = true,
                        Err(err) => state.error(err.to_string()),
                    }
                }
                ui.close();
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                state.clip_name_draft = None;
                ui.close();
            }
        }
        _ => {
            let label = if count > 1 && state.selected_clips.contains(&clip) {
                "Rename and Number…"
            } else {
                "Rename…"
            };
            if item(ui, label, "") {
                let current = editor.clip_name(clip).unwrap_or_default();
                state.clip_name_draft = Some((clip, current));
            }
        }
    }

    // A mark on the footage rather than on the edit: it travels with the
    // clip (`Sequence::clip_marks`).
    let marks = editor.clip_marks(clip).len();
    if item(ui, "Mark This Frame", "Shift+M") {
        match editor.toggle_clip_mark(clip) {
            Ok(true) => {
                state.needs_repaint = true;
                state.info("Marked — the mark travels with the clip");
            }
            Ok(false) => {
                state.needs_repaint = true;
                state.info("Mark removed");
            }
            Err(err) => state.error(err.to_string()),
        }
    }
    if marks > 0 && item(ui, "Clear Clip Marks", "") {
        match editor.clear_clip_marks(clip) {
            Ok(n) => {
                state.needs_repaint = true;
                state.info(format!("Took off {n} mark(s)"));
            }
            Err(err) => state.error(err.to_string()),
        }
    }

    // A lane up or down, keeping its time: over another shot for a
    // picture-in-picture, or onto a sound lane of its own.
    for (label, up) in [("Move Up a Lane", true), ("Move Down a Lane", false)] {
        if editor.neighbour_lane(clip, up).is_some() && item(ui, label, "") {
            match editor.move_to_neighbour_lane(clip, up) {
                Ok(()) => state.needs_repaint = true,
                Err(err) => state.error(err.to_string()),
            }
        }
    }

    // Solo: only this (and what is soloed with it) on screen or in the mix,
    // without muting a lane at a time. Its sound comes along (§12).
    let soloed = editor.clip_soloed(clip);
    if item(ui, if soloed { "Unsolo" } else { "Solo" }, "") {
        let onto = selection_or(state, clip);
        match editor.set_clip_solo(&onto, !soloed) {
            Ok(_) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }
    if editor.any_clip_soloed() && item(ui, "Clear Solos", "") {
        match editor.clear_clip_solos() {
            Ok(_) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }

    // Every clip painted this colour, wherever it is: the way a colour
    // label earns its keep on a long timeline.
    if let Some(label) = editor
        .color_label(clip)
        .filter(|l| *l != bettercut_editor_core::timeline::ColorLabel::None)
        && item(ui, &format!("Select All {}", label.name()), "")
    {
        let same = editor.clips_with_colour(label);
        state.clear_selection();
        state.selected_clips.extend(same);
        state.needs_repaint = true;
    }

    // A title read aloud, by a voice built into the computer, as a voiceover
    // starting where the title does.
    if let Some(title) = editor.text_clip(clip) {
        let (words, at) = (title.text.clone(), title.timeline.start);
        ui.menu_button("Read Aloud", |ui| {
            let mut voice = None;
            if ui.button("Default voice").clicked() {
                voice = Some(None);
            }
            match &state.speech_voices {
                None => {
                    state.speech_voices_wanted = true;
                    ui.label(
                        egui::RichText::new("Finding voices…")
                            .small()
                            .color(crate::theme::disabled()),
                    );
                }
                Some(voices) => {
                    for name in voices {
                        if ui.button(name.as_str()).clicked() {
                            voice = Some(Some(name.clone()));
                        }
                    }
                }
            }
            if let Some(voice) = voice {
                ui.close();
                state.speech_request = Some(crate::speech::SpeechRequest {
                    text: words.clone(),
                    voice,
                    rate: 0,
                    at,
                });
            }
        })
        .response
        .on_hover_text(
            "Have a computer voice read this title, added as a voiceover where the title starts",
        );
    }

    // Fold the selection into a sequence of its own, or open the one this
    // clip already is. Near the top: it changes what the rest of the menu is
    // even acting on.
    // A multicam clip: which camera is on screen, and cutting to another one
    // where the playhead is.
    if let Some(angles) = editor.angle_count(clip).filter(|count| *count > 1) {
        let current = editor.angle_of(clip);
        let at = editor.playhead();
        ui.menu_button("Angle", |ui| {
            for angle in 0..angles {
                if ui
                    .selectable_label(current == Some(angle), format!("Angle {}", angle + 1))
                    .on_hover_text("Show this camera from the playhead on")
                    .clicked()
                {
                    match editor.cut_to_angle(clip, at, angle) {
                        Ok(piece) => state.select_only(piece),
                        Err(err) => state.error(err.to_string()),
                    }
                    state.needs_repaint = true;
                    ui.close();
                }
            }
            if ui
                .selectable_label(current.is_none(), "All at once")
                .on_hover_text("Play every camera stacked, as an ordinary compound clip")
                .clicked()
            {
                if let Err(err) = editor.set_angle(clip, None) {
                    state.error(err.to_string());
                }
                state.needs_repaint = true;
                ui.close();
            }
        });
    }

    if editor.compound_of(clip).is_some() {
        if ui
            .button("Open Compound")
            .on_hover_text("Edit what is inside this compound clip")
            .clicked()
        {
            editor.open_compound(clip);
            state.needs_repaint = true;
            ui.close();
        }
        if ui
            .button("Break Apart")
            .on_hover_text("Put what is inside back on the timeline where this compound sits")
            .clicked()
        {
            ui.close();
            match editor.break_apart(clip) {
                Ok(n) => {
                    state.clear_selection();
                    state.needs_repaint = true;
                    state.info(format!("{n} clip(s) back on the timeline"));
                }
                Err(err) => state.error(err.to_string()),
            }
        }
    } else {
        let mut chosen: Vec<ClipId> = state.selected_clips.iter().copied().collect();
        if !chosen.contains(&clip) {
            chosen = vec![clip];
        }
        if chosen.len() > 1
            && ui
                .button("Make Multicam Clip")
                .on_hover_text(
                    "Fold the selected cameras into one clip you cut between. \
                     Line them up with Sync by Sound first.",
                )
                .clicked()
        {
            match editor.make_multicam(&chosen, "") {
                Ok(multicam) => {
                    state.select_only(multicam);
                    state.info("Folded into a multicam clip");
                }
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
            ui.close();
        }
        if ui
            .button("Make Compound Clip")
            .on_hover_text(
                "Fold the selection into a sequence of its own, played here as one \
                 clip. Open it later to change what is inside.",
            )
            .clicked()
        {
            match editor.make_compound(&chosen, "") {
                Ok(compound) => {
                    state.select_only(compound);
                    state.info("Folded into a compound clip");
                }
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
            ui.close();
        }
    }

    // Two clips picked out: line this one up with the other by what the two
    // of them heard. A second camera, or a recorder running beside one.
    let others: Vec<ClipId> = state
        .selected_clips
        .iter()
        .copied()
        .filter(|other| *other != clip)
        .collect();
    // And, with the same two, make this one follow something in the other:
    // the box this clip sits in is the patch that is tracked, so putting it
    // over a face and asking is the whole gesture.
    if let [footage] = others[..]
        && editor.video_clip(clip).is_some()
        && editor.video_clip(footage).is_some()
        && ui
            .button("Follow the Other Clip")
            .on_hover_text(
                "Track what is under this clip in the other one, and keyframe \
                 this clip to follow it",
            )
            .clicked()
    {
        crate::tracking::track_and_attach(editor, state, clip, footage);
        ui.close();
    }

    // Take the shake out of a hand-held shot, measured from the shot itself.
    if others.is_empty()
        && editor.video_clip(clip).is_some()
        && ui
            .button("Steady This Shot")
            .on_hover_text(
                "Measure how the picture moves and cancel the wobble. The shot \
                 is cropped in a little to hide the edges.",
            )
            .clicked()
    {
        crate::tracking::steady(editor, state, clip);
        ui.close();
    }

    if let [reference] = others[..]
        && ui
            .button("Sync by Sound")
            .on_hover_text("Move this clip so its sound lines up with the other selected clip")
            .clicked()
    {
        sync_by_sound(editor, state, clip, reference);
        ui.close();
    }

    // Where this clip's footage came from. The browser is where a file is
    // renamed, re-binned, relinked and replaced everywhere — all of which
    // start with finding it among the rest.
    if let Some(media) = editor.media_of_clip(clip) {
        let name = editor
            .project()
            .media_asset(media)
            .map(|asset| asset.display_name().to_owned());
        if let Some(name) = name
            && ui
                .button("Find in Media")
                .on_hover_text("Show the file this clip plays in the media browser")
                .clicked()
        {
            state.reveal_media(media, &name);
            ui.close();
        }

        // The clip filling the timeline: for trimming its ends by a frame.
        if ui
            .button("Zoom to Clip")
            .on_hover_text("Zoom the timeline in on this clip (Shift + Z zooms to the selection)")
            .clicked()
        {
            let lanes =
                (ui.ctx().content_rect().width() - crate::theme::TRACK_HEADER_WIDTH).max(200.0);
            state.zoom_to_clip(editor, clip, lanes);
            ui.close();
        }

        // The same, but at the frame being looked at and with the stretch this
        // clip plays marked on the file (`editor_core::match_frame`).
        if ui
            .button("Match Frame")
            .on_hover_text(
                "Open this clip's file at the frame under the playhead, with what it plays marked",
            )
            .clicked()
        {
            crate::shortcuts::match_frame(editor, state, Some(clip));
            ui.close();
        }
    }

    // Fitting a shot to a length by re-timing it rather than trimming it
    // (`editor_core::rate_stretch`).
    if editor.can_retime(clip) {
        let playhead = editor.playhead();
        let speed = editor.rate_stretch_speed(clip, playhead);
        if let Some(speed) = speed
            && item(
                ui,
                &format!("Stretch to Playhead ({:.2}\u{d7})", speed.as_f64()),
                "",
            )
        {
            match editor.rate_stretch(clip, playhead, false) {
                Ok(_) => state.info("Stretched to the playhead"),
                Err(err) => state.error(err.to_string()),
            }
        }
    }

    // §12's deliberate exception: the sound of one shot reaching past its
    // picture into the next, which is what a cut sounds like in every film
    // ever made (`editor_core::split_edit`).
    if editor.sound_of(clip).is_some() {
        ui.menu_button("Split Edit", |ui| {
            let offset = editor
                .split_edit_offset(clip, bettercut_editor_core::TrimEdge::End)
                .unwrap_or(bettercut_editor_core::foundation::TimelineTime::ZERO);
            ui.label(
                egui::RichText::new(if offset.ticks() == 0 {
                    "The cut after this shot is straight".to_owned()
                } else if offset.ticks() > 0 {
                    format!(
                        "Its sound runs {:.2} s past its picture",
                        seconds_of(offset)
                    )
                } else {
                    format!("Its sound stops {:.2} s early", -seconds_of(offset))
                })
                .small()
                .color(crate::theme::disabled()),
            );
            let mut rolled = None;
            for (label, sign) in [
                ("This sound over the next shot (L)", 1_i64),
                ("Next sound before its picture (J)", -1),
            ] {
                ui.menu_button(label, |ui| {
                    for millis in [250_i64, 500, 1000, 2000] {
                        if ui
                            .button(format!("{:.2} s", millis as f64 / 1000.0))
                            .clicked()
                        {
                            ui.close();
                            rolled = Some(sign * millis);
                        }
                    }
                });
            }
            if ui
                .button("Straighten This Cut")
                .on_hover_text("Put the sound edge back level with the picture")
                .clicked()
            {
                ui.close();
                if let Err(err) = editor.straighten_cut(clip, bettercut_editor_core::TrimEdge::End)
                {
                    state.error(err.to_string());
                }
            }
            if let Some(millis) = rolled
                && let Err(err) = editor.roll_sound_cut(
                    clip,
                    bettercut_editor_core::foundation::TimelineTime::from_millis(millis),
                )
            {
                state.error(err.to_string());
            }
        });
    }

    // Fade the selection in one go: pictures through their entrance and exit,
    // sounds through their volume.
    ui.menu_button("Fade", |ui| {
        use bettercut_editor_core::fade_selection::FadeEnds;
        let mut selected: Vec<ClipId> = state.selected_clips.iter().copied().collect();
        if !selected.contains(&clip) {
            selected = vec![clip];
        }
        let mut chosen = None;
        for (label, ends) in [
            ("In and Out", FadeEnds::Both),
            ("In", FadeEnds::In),
            ("Out", FadeEnds::Out),
        ] {
            ui.menu_button(label, |ui| {
                for millis in [500_i32, 1000, 2000] {
                    if ui
                        .button(format!("{:.1} s", f64::from(millis) / 1000.0))
                        .clicked()
                    {
                        ui.close();
                        chosen = Some((ends, millis));
                    }
                }
            });
        }
        if ui.button("Remove Fades").clicked() {
            ui.close();
            chosen = Some((FadeEnds::Neither, 0));
        }
        if let Some((ends, millis)) = chosen {
            match editor.fade_clips(
                &selected,
                ends,
                bettercut_editor_core::foundation::TimelineTime::from_millis(i64::from(millis)),
            ) {
                Ok(0) => state.info("Those clips already fade that way"),
                Ok(n) => {
                    state.info(format!("Faded {n} clip(s)"));
                    state.needs_repaint = true;
                }
                Err(err) => state.error(err.to_string()),
            }
        }
    })
    .response
    .on_hover_text("Fade the selected clips in, out or both — pictures and their sound together");

    // A montage in a new order: the selected shots, reshuffled in place.
    if count > 1 {
        let selected: Vec<ClipId> = state.selected_clips.iter().copied().collect();
        if ui
            .button("Shuffle Order")
            .on_hover_text(
                "Put the selected clips in a random order, keeping the stretch they cover",
            )
            .clicked()
        {
            ui.close();
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64);
            match editor.shuffle_clips(&selected, seed) {
                Ok(order) => {
                    state.info(format!("Shuffled {} clips", order.len()));
                    state.needs_repaint = true;
                }
                Err(err) => state.error(err.to_string()),
            }
        }
        if ui
            .button("Reverse Order")
            .on_hover_text("Put the selected clips last-first, keeping the stretch they cover")
            .clicked()
        {
            ui.close();
            match editor.reverse_clip_order(&selected) {
                Ok(order) => {
                    state.info(format!("Reversed {} clips", order.len()));
                    state.needs_repaint = true;
                }
                Err(err) => state.error(err.to_string()),
            }
        }
    }
    // Say how many will be affected, so a menu opened on one clip inside a
    // multi-selection cannot be mistaken for acting on just that clip.
    ui.label(
        egui::RichText::new(if count > 1 {
            format!("{count} clips selected")
        } else {
            "Clip".to_owned()
        })
        .small()
        .color(crate::theme::disabled()),
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
    // The range for exporting one scene or looping one shot, set in one go.
    if item(ui, "Mark In and Out Around", "") {
        let chosen = selection_or(state, clip);
        match editor.mark_clips(&chosen) {
            Ok(Some(range)) => {
                state.needs_repaint = true;
                state.info(format!(
                    "Marked {} to {}",
                    range.start.format_timecode(),
                    range.end.format_timecode()
                ));
            }
            Ok(None) => state.info("Those clips are no longer on the timeline"),
            Err(err) => state.error(err.to_string()),
        }
    }
    // The location card on every shot, the speaker on every answer.
    if editor.video_clip(clip).is_some()
        && ui
            .button("Title Each Clip")
            .on_hover_text(
                "Put a lower third over each selected picture, reading its name, to retype",
            )
            .clicked()
    {
        ui.close();
        let chosen = selection_or(state, clip);
        match editor.title_each_clip(&chosen) {
            Ok(0) => state.info("The title lane is already taken over those clips"),
            Ok(n) => {
                state.needs_repaint = true;
                state.info(format!(
                    "Added {n} title(s) — double-click one to retype it"
                ));
            }
            Err(err) => state.error(err.to_string()),
        }
    }
    // A copy right over the shot: where a glow, a blur layer or a
    // picture-in-picture starts.
    if editor.video_clip(clip).is_some() && item(ui, "Duplicate Onto Lane Above", "") {
        match editor.duplicate_onto_lane_above(clip) {
            Ok(copy) => {
                state.clear_selection();
                state.selected_clips.insert(copy);
                state.needs_repaint = true;
                state.info("Copied onto the lane above — the copy is selected");
            }
            Err(err) => state.error(err.to_string()),
        }
    }

    ui.menu_button("Look and Animation", |ui| {
        // A look is minutes of work, and the next twenty clips want the same
        // one. Beside Copy, because it is the same idea applied to the settings
        // rather than to the clip.
        // The copied look, in parts: the grade from the wide shot without its
        // framing (`editor_core::attributes`).
        if let Some(look) = state.copied_look.clone() {
            use bettercut_editor_core::attributes::{AttributeGroup, groups_in, only};
            let available = groups_in(&look);
            ui.menu_button("Paste Attributes", |ui| {
                ui.label(
                    egui::RichText::new("Tick what to paste, then Paste.")
                        .small()
                        .color(crate::theme::disabled()),
                );
                for group in AttributeGroup::ALL {
                    if !available.contains(&group) {
                        continue;
                    }
                    let (label, hint) = group.label();
                    let mut on = state.paste_groups.contains(&group);
                    if ui.checkbox(&mut on, label).on_hover_text(hint).changed() {
                        if on {
                            state.paste_groups.push(group);
                        } else {
                            state.paste_groups.retain(|g| *g != group);
                        }
                    }
                }
                ui.separator();
                let wanted = only(&look, &state.paste_groups);
                if ui
                    .add_enabled(!wanted.is_empty(), egui::Button::new("Paste"))
                    .on_disabled_hover_text("Tick at least one")
                    .clicked()
                {
                    ui.close();
                    let onto = selection_or(state, clip);
                    match editor.paste_look(&wanted, onto) {
                        Ok(0) => state.error("Nothing to paste onto"),
                        Ok(n) => {
                            state.needs_repaint = true;
                            state.info(format!("Pasted onto {n} clip(s)"));
                        }
                        Err(err) => state.error(err.to_string()),
                    }
                }
            });
        }

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

        // A move worked out on one shot, for the next ones: the keyframes alone.
        if let Some(animation) = editor.copy_animation(clip)
            && item(ui, "Copy Animation", "")
        {
            state.copied_animation = Some(animation);
            state.info("Animation copied: paste it onto other clips");
        }
        if let Some(animation) = state.copied_animation.clone()
            && editor.video_clip(clip).is_some()
            && item(ui, "Paste Animation", "")
        {
            let onto = selection_or(state, clip);
            match editor.paste_animation(&animation, onto) {
                Ok(0) => state.error("Nothing there to take the animation"),
                Ok(1) => state.info("Animation pasted"),
                Ok(n) => state.info(format!("Animation pasted onto {n} clips")),
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
        }
    })
    .response
    .on_hover_text("Copy, paste or clear a look, or a clip's animation");

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
                .shortcut_text(egui::RichText::new("Shift+Del").color(crate::theme::disabled())),
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
                    format!("{} ✔", axis.label())
                } else {
                    axis.label().to_string()
                })
                .clicked()
            {
                ui.close();
                flip = Some((axis, !on));
            }
        }
        // A quarter turn either way, for the selection when this clip is in
        // it — a phone shot filmed sideways is rarely the only one.
        for (label, keys, quarters) in [("Rotate Right", "R", 1), ("Rotate Left", "Shift+R", -1)] {
            if item(ui, label, keys) {
                if !state.selected_clips.contains(&clip) {
                    state.select_only(clip);
                }
                crate::shortcuts::rotate_selection(editor, state, quarters);
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
        // Censor: a pixelated, masked copy above, selected so its mask can be
        // dragged over the face or plate straight away.
        ui.menu_button("Censor Part of Shot", |ui| {
            for style in bettercut_editor_core::censor::CensorStyle::ALL {
                if ui.button(style.label()).clicked() {
                    ui.close();
                    match editor.censor_clip_with(clip, style) {
                        Ok(copy) => {
                            state.select_only(copy);
                            state.info(
                                "Censor added above the shot: place it with the Mask controls",
                            );
                        }
                        Err(err) => state.error(err.to_string()),
                    }
                    state.needs_repaint = true;
                }
            }
        })
        .response
        .on_hover_text(
            "Cover a face or a number plate. Move and size the oval with the Mask controls.",
        );

        // Colour match: this clip graded to look like the frame under the
        // playhead, worked out in the background by the desktop shell.
        if ui
            .button("Match Colour to Playhead")
            .on_hover_text(
                "Grade this clip to look like the frame under the playhead. Park the playhead on the shot to match first; this replaces the clip's colour settings, and can be undone.",
            )
            .clicked()
        {
            ui.close();
            state.colour_match_request = Some((clip, editor.playhead()));
        }
        // And the same solver aimed at a neutral frame: exposure and balance
        // put right without a reference shot.
        if ui
            .button("Auto Level")
            .on_hover_text(
                "Bring this clip to a neutral exposure and white balance, keeping its own colourfulness. Replaces the clip's colour settings; can be undone.",
            )
            .clicked()
        {
            ui.close();
            state.auto_level_request = Some(clip);
        }

        ui.menu_button("Freeze and Hold", |ui| {
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

            // The record-scratch moment: the frame freezes, the picture punches in
            // and a flash marks it.
            ui.menu_button("Freeze and Punch In", |ui| {
                for seconds in [1, 2, 3] {
                    if ui.button(format!("{seconds} s")).clicked() {
                        ui.close();
                        match editor.freeze_punch(
                            clip,
                            bettercut_editor_core::foundation::TimelineTime::from_seconds(seconds),
                        ) {
                            Ok(held) => {
                                state.select_only(held);
                                state.info("Frozen, punched in, with a flash");
                            }
                            Err(err) => state.error(err.to_string()),
                        }
                        state.needs_repaint = true;
                    }
                }
            })
            .response
            .on_hover_text("Freeze the frame under the playhead, zoom in on it fast and flash into it");

            // The opening image standing still before the shot moves: room for
            // a title over it.
            ui.menu_button("Hold First Frame", |ui| {
                for seconds in [1, 2, 3] {
                    if ui.button(format!("{seconds} s")).clicked() {
                        ui.close();
                        match editor.hold_first_frame(
                            clip,
                            bettercut_editor_core::foundation::TimelineTime::from_seconds(seconds),
                        ) {
                            Ok(held) => {
                                state.select_only(held);
                                state.info(format!("Holding the first frame for {seconds} s"));
                            }
                            Err(err) => state.error(err.to_string()),
                        }
                        state.needs_repaint = true;
                    }
                }
            })
            .response
            .on_hover_text("Stand still on the clip's first frame before it plays, moving it and what follows along");

            // The shot lingering on its last image, room made after it.
            ui.menu_button("Hold Last Frame", |ui| {
                for seconds in [1, 2, 3] {
                    if ui.button(format!("{seconds} s")).clicked() {
                        ui.close();
                        match editor.hold_last_frame(
                            clip,
                            bettercut_editor_core::foundation::TimelineTime::from_seconds(seconds),
                        ) {
                            Ok(held) => {
                                state.select_only(held);
                                state.info(format!("Holding the last frame for {seconds} s"));
                            }
                            Err(err) => state.error(err.to_string()),
                        }
                        state.needs_repaint = true;
                    }
                }
            })
            .response
            .on_hover_text("Linger on the clip's final frame after it ends, moving what follows along");

            // One long take in pieces: equal parts, or chunks of a set length.
            ui.menu_button("Split Into", |ui| {
                let mut result = None;
                for parts in [2, 3, 4, 5, 10] {
                    if ui.button(format!("{parts} Equal Parts")).clicked() {
                        ui.close();
                        result = Some(editor.split_into_parts(clip, parts));
                    }
                }
                ui.separator();
                for seconds in [1, 2, 5, 10, 30] {
                    if ui.button(format!("Every {seconds} s")).clicked() {
                        ui.close();
                        result = Some(editor.split_every(
                            clip,
                            bettercut_editor_core::foundation::TimelineTime::from_seconds(seconds),
                        ));
                    }
                }
                match result {
                    Some(Ok(cuts)) => {
                        state.clear_selection();
                        state.info(format!("Split into {} clips", cuts + 1));
                        state.needs_repaint = true;
                    }
                    Some(Err(err)) => state.error(err.to_string()),
                    None => {}
                }
            })
            .response
            .on_hover_text("Cut the clip into equal pieces, or into pieces of a set length");
        })
        .response
        .on_hover_text("Freeze a frame, hold the first or last, or split into pieces");

        if editor.can_retime(clip) {
            let reversed = editor.is_reversed(clip);
            if ui
                .button(if reversed { "Play Forwards" } else { "Reverse" })
                .on_hover_text("Play the clip backwards, with its sound")
                .clicked()
            {
                ui.close();
                // The whole selection, when there is one: each shot turned
                // the same way as the one clicked.
                let chosen = selection_or(state, clip);
                if chosen.len() > 1 {
                    match editor.set_clips_reversed(&chosen, !reversed) {
                        Ok(n) => state.info(format!(
                            "{n} clip(s) {}",
                            if reversed {
                                "playing forwards"
                            } else {
                                "reversed"
                            }
                        )),
                        Err(err) => state.error(err.to_string()),
                    }
                } else {
                    match editor.set_reversed(clip, !reversed) {
                        Ok(()) => state.info(if reversed {
                            "Playing forwards"
                        } else {
                            "Reversed"
                        }),
                        Err(err) => state.error(err.to_string()),
                    }
                }
                state.needs_repaint = true;
            }
            // Fit to fill: the speed at which the clip ends exactly at the
            // next clip, or at the playhead.
            let fill = editor.fill_end(clip);
            if ui
                .add_enabled(fill.is_some(), egui::Button::new("Fit to Fill"))
                .on_hover_text("Play the clip slower so it fills the gap after it")
                .on_disabled_hover_text("There is no gap after this clip to fill")
                .clicked()
            {
                ui.close();
                match editor.fit_to_fill(clip) {
                    Ok(speed) => state.info(format!("Fitted at {:.2}×", speed.as_f64())),
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
            let playhead = editor.playhead();
            let ends_there = editor
                .active_sequence()
                .and_then(|s| s.clip_span(clip))
                .is_some_and(|span| span.timeline.end == playhead);
            let reaches = !ends_there && editor.speed_to_end_at(clip, playhead).is_some();
            if ui
                .add_enabled(reaches, egui::Button::new("Fit to Playhead"))
                .on_hover_text("Speed the clip up or slow it down so it ends at the playhead")
                .on_disabled_hover_text(
                    "Put the playhead after the clip's start, away from its end",
                )
                .clicked()
            {
                ui.close();
                match editor.fit_to_end(clip, playhead) {
                    Ok(speed) => state.info(format!("Fitted at {:.2}×", speed.as_f64())),
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
            if ui
                .button("Boomerang")
                .on_hover_text("Follow the clip with itself playing backwards, making room for it")
                .clicked()
            {
                ui.close();
                match editor.boomerang(clip) {
                    Ok(back) => {
                        state.select_only(back);
                        state.info("Boomerang added");
                    }
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
            if ui
                .button("Rewind")
                .on_hover_text(
                    "Follow the clip with itself winding back fast, like a tape being rewound",
                )
                .clicked()
            {
                ui.close();
                match editor.rewind(clip) {
                    Ok(back) => {
                        state.select_only(back);
                        state.info("Rewind added");
                    }
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
            // One-click speeds. The sound is re-timed with the picture, and
            // later clips on the track move to make room or close up.
            ui.menu_button("Speed", |ui| {
                let now = editor.video_clip(clip).map(|c| c.speed);
                for speed in bettercut_editor_core::timeline::SPEED_PRESETS {
                    let label = bettercut_editor_core::timeline::speed_label(speed);
                    let current =
                        now.is_some_and(|s| s.num() * speed.den() == speed.num() * s.den());
                    if ui.selectable_label(current, &label).clicked() {
                        ui.close();
                        // Every selected shot at once: a montage at 2x is
                        // one pick, not one per clip.
                        let chosen = selection_or(state, clip);
                        if chosen.len() > 1 {
                            match editor.set_clips_speed(&chosen, speed) {
                                Ok(0) => state.info("None of those clips has motion to re-time"),
                                Ok(n) => state.info(format!("{n} clip(s) playing at {label}")),
                                Err(err) => state.error(err.to_string()),
                            }
                        } else {
                            match editor.set_clip_speed(clip, speed, false) {
                                Ok(()) => state.info(format!("Playing at {label}")),
                                Err(err) => state.error(err.to_string()),
                            }
                        }
                        state.needs_repaint = true;
                    }
                }
            })
            .response
            .on_hover_text("Play the clip slower or faster, with its sound");
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

    // The same shot, or the same bars of music, again and again.
    if editor.video_clip(clip).is_some() || editor.audio_clip(clip).is_some() {
        ui.menu_button("Loop", |ui| {
            for times in [2, 3, 4, 5, 10] {
                if ui.button(format!("Play {times} Times")).clicked() {
                    ui.close();
                    match editor.loop_clip(clip, times) {
                        Ok(_) => state.info(format!("Looped {times} times")),
                        Err(err) => state.error(err.to_string()),
                    }
                    state.needs_repaint = true;
                }
            }
        })
        .response
        .on_hover_text("Repeat the clip back to back, moving everything after it along");
    }

    // Cuts, for picture: a file that already has cuts in it — a download, a
    // screen recording, last year's export — takes a scrub per cut to find by
    // hand (Milestone 12).
    if editor.video_clip(clip).is_some_and(|clip| !clip.frozen)
        && ui
            .button("Find Cuts…")
            .on_hover_text(
                "Look through this clip for scene changes and black at its ends, and offer to split or trim",
            )
            .clicked()
    {
        ui.close();
        state.scene_request = Some(clip);
    }
    // The sound tools, together: a long list at the top level is one
    // nobody reads.
    ui.menu_button("Sound", |ui| {
        // Bars that jump with this sound, over the whole picture.
        if let Some(sound) = sound_of(editor, clip)
            && ui
                .button("Visualize This Sound")
                .on_hover_text(
                    "Bars across the picture that jump with this clip's sound, for as long as it plays",
                )
                .clicked()
        {
            ui.close();
            visualize(editor, state, sound);
        }

        // A song longer than the edit: end it with the pictures, faded out.
        if editor.audio_clip(clip).is_some()
            && editor.linked_with(clip).len() == 1
            && ui
                .button("Fit Music to Edit")
                .on_hover_text("Trim this sound to end where the pictures end, with a fade-out")
                .clicked()
        {
            ui.close();
            match editor.fit_music(clip) {
                Ok(end) => state.info(format!("Music now ends at {}", end.format_timecode())),
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
        }

        // Silence this clip's sound without moving or deleting anything.
        let has_sound = editor
            .linked_with(clip)
            .iter()
            .any(|c| editor.audio_clip(*c).is_some());
        if has_sound {
            let muted = editor.is_muted(clip);
            if ui
                .button(if muted { "Unmute Clip" } else { "Mute Clip" })
                .on_hover_text(
                    "Silence this clip's sound where it stands; it stays in place and in sync",
                )
                .clicked()
            {
                ui.close();
                match editor.set_muted(clip, !muted) {
                    Ok(_) => state.info(if muted { "Clip unmuted" } else { "Clip muted" }),
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
        }

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
        // Several sound clips at different levels — two people, two mics — made
        // as loud as each other (`loudness::even_out`).
        let sounds: Vec<ClipId> = selection_or(state, clip)
            .into_iter()
            .filter_map(|c| {
                if editor.audio_clip(c).is_some() {
                    Some(c)
                } else {
                    sound_of(editor, c)
                }
            })
            .collect();
        if sounds.len() > 1
            && ui
                .button("Even Out Loudness")
                .on_hover_text(
                    "Measure each selected sound and set its volume so they all sound as loud as each other",
                )
                .clicked()
        {
            ui.close();
            crate::loudness::even_out(editor, state, &sounds);
        }

        // The other way round from removing silences: what is loud is what is
        // kept (`playback::highlights`).
        if let Some(sound) = sound_of(editor, clip)
            && ui
                .button("Find the Good Bits…")
                .on_hover_text("Find the loudest moments of this clip and offer to keep only them")
                .clicked()
        {
            ui.close();
            match crate::highlight_dialog::HighlightDialog::open(editor, &mut state.waveforms, sound) {
                Some(dialog) => state.highlights = Some(dialog),
                None => state.info("This clip's sound is still being analysed — try again in a moment"),
            }
        }
        // Just the ends, no dialog: room tone before the first word and the
        // reach for the stop button after the last.
        if let Some(sound) = sound_of(editor, clip)
            && ui
                .button("Trim Silent Ends")
                .on_hover_text("Take the quiet off the start and end of this clip, picture and all")
                .clicked()
        {
            ui.close();
            let ends = editor.audio_clip(sound).cloned().and_then(|audio| {
                let waveform = state.waveforms.get(audio.media_id)?;
                Some(bettercut_playback::silent_ends(
                    &audio,
                    &waveform,
                    bettercut_playback::SilenceSettings::default(),
                ))
            });
            match ends {
                None => state.info("This clip's sound is still being analysed — try again in a moment"),
                Some((None, None)) => state.info("No silence at either end"),
                Some((from, to)) => {
                    let before = editor.clip_duration(sound);
                    match editor.trim_ends(sound, from, to, "Trim Silent Ends") {
                        Ok(true) => {
                            let after = editor.clip_duration(sound);
                            let cut = before
                                .zip(after)
                                .map_or(0.0, |(b, a)| (b - a).as_seconds_f64());
                            state.info(format!("Trimmed {cut:.1} s of silence"));
                        }
                        Ok(false) => state.info("No silence at either end"),
                        Err(err) => state.error(err.to_string()),
                    }
                    state.needs_repaint = true;
                }
            }
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
    })
    .response
    .on_hover_text("Beats, loudness, silences, captions and more for this clip's sound");
    ui.separator();

    // Jumping to a clip's edges is what makes trimming to a neighbour precise,
    // and there is no keyboard route to it yet.
    // The slideshow at three seconds a photo: every selected picture one
    // length, what follows moved to suit.
    let pictures: Vec<ClipId> = selection_or(state, clip)
        .into_iter()
        .filter(|c| editor.video_clip(*c).is_some() || editor.text_clip(*c).is_some())
        .collect();
    if !pictures.is_empty() {
        ui.menu_button("Set Length", |ui| {
            for tenths in [5_i64, 10, 20, 30, 50] {
                if ui.button(format!("{} s", tenths as f64 / 10.0)).clicked() {
                    ui.close();
                    let length =
                        bettercut_editor_core::foundation::TimelineTime::from_millis(tenths * 100);
                    match editor.set_clip_lengths(&pictures, length) {
                        Ok(0) => state.info("Already that long, or no more footage to show"),
                        Ok(n) => {
                            state.needs_repaint = true;
                            state.info(format!("{n} clip(s) set to {} s", tenths as f64 / 10.0));
                        }
                        Err(err) => state.error(err.to_string()),
                    }
                }
            }
        })
        .response
        .on_hover_text(
            "Make the selected pictures and titles this long, moving what follows on their lane",
        );
    }

    // The camera's scratch audio gone from under a shot, the shot kept.
    if editor.video_clip(clip).is_some()
        && sound_of(editor, clip).is_some_and(|s| s != clip)
        && ui
            .button("Remove Its Sound")
            .on_hover_text("Delete the sound recorded with this clip and keep the picture")
            .clicked()
    {
        ui.close();
        // The whole selection's camera audio, when several are picked.
        let chosen = selection_or(state, clip);
        match editor.remove_sound_of(&chosen) {
            Ok(0) => state.info("This clip has no sound of its own"),
            Ok(_) => {
                state.needs_repaint = true;
                state.info("Sound removed; the picture stays");
            }
            Err(err) => state.error(err.to_string()),
        }
    }

    // The gap after a shot filled with more of the shot, rather than
    // closed by moving everything after it.
    if (editor.video_clip(clip).is_some() || editor.audio_clip(clip).is_some())
        && ui
            .button("Extend to Next Clip")
            .on_hover_text("Pull this clip's end out to meet the next clip on its lane, as far as its footage goes")
            .clicked()
    {
        ui.close();
        match editor.extend_to_next(clip) {
            Ok(end) => {
                state.needs_repaint = true;
                state.info(format!("Now ends at {}", end.format_timecode()));
            }
            Err(err) => state.error(err.to_string()),
        }
    }
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
            let swatch = label.rgb().map_or(crate::theme::disabled(), |[r, g, b]| {
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

/// Put a visualizer over the sequence from a sound clip's waveform, keeping the
/// look of any visualizer already there.
pub fn visualize(editor: &mut Editor, state: &mut UiState, sound: ClipId) {
    let Some(clip) = editor.audio_clip(sound).cloned() else {
        return;
    };
    let Some(waveform) = state.waveforms.get(clip.media_id) else {
        state.info("This clip's sound is still being analysed — try again in a moment");
        return;
    };
    let Some((start, levels)) = bettercut_playback::visualizer_levels(&clip, &waveform) else {
        state.info("This clip is silent: nothing to visualize");
        return;
    };
    let visualizer = match editor.active_sequence().and_then(|s| s.visualizer.clone()) {
        Some(existing) => bettercut_editor_core::timeline::visualizer::Visualizer {
            levels,
            start,
            ..existing
        },
        None => bettercut_editor_core::timeline::visualizer::Visualizer::new(levels, start),
    };
    match editor.set_visualizer(Some(visualizer), false) {
        Ok(()) => {
            state.info("Visualizer added — change its look in the Inspector with nothing selected")
        }
        Err(err) => state.error(err.to_string()),
    }
    state.needs_repaint = true;
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

    // Every join on a sound lane softened at once: music and room tone cut
    // together sound like a mistake until they are crossfaded.
    if is_sound {
        ui.menu_button("Crossfade Every Cut", |ui| {
            for ms in [100_i64, 250, 500, 1_000] {
                if ui.button(format!("{ms} ms")).clicked() {
                    ui.close();
                    let length = bettercut_editor_core::foundation::TimelineTime::from_millis(ms);
                    match editor.crossfade_every_cut(track, length) {
                        Ok((0, 0)) => state.info("There are no joins on this lane to soften"),
                        Ok((done, 0)) => state.info(format!("Crossfaded {done} cut(s)")),
                        Ok((done, skipped)) => {
                            state.info(format!("Crossfaded {done} cut(s); {skipped} had no room"))
                        }
                        Err(err) => state.error(err.to_string()),
                    }
                    state.needs_repaint = true;
                }
            }
        })
        .response
        .on_hover_text("Soften every join between the clips on this lane");
    }

    let lock_label = if locked { "Unlock Track" } else { "Lock Track" };
    if item(ui, lock_label, "")
        && let Err(err) = editor.set_track_flag(track, TrackFlag::Locked, !locked)
    {
        state.error(err.to_string());
    }
    // Every lane at once: locked while the picture is signed off, or all
    // opened again to move on.
    for (label, value) in [("Lock All Lanes", true), ("Unlock All Lanes", false)] {
        if item(ui, label, "") {
            match editor.set_all_tracks_flag(TrackFlag::Locked, value) {
                Ok(0) => state.info(if value {
                    "Every lane is already locked"
                } else {
                    "No lane is locked"
                }),
                Ok(_) => state.needs_repaint = true,
                Err(err) => state.error(err.to_string()),
            }
        }
    }
    // Back to hearing and seeing everything, after checking one lane alone.
    for (label, flag, value, none) in [
        (
            "Turn All Lanes On",
            TrackFlag::Enabled,
            true,
            "Every lane is already on",
        ),
        (
            "Clear All Solos",
            TrackFlag::Solo,
            false,
            "No lane is soloed",
        ),
    ] {
        if item(ui, label, "") {
            match editor.set_all_tracks_flag(flag, value) {
                Ok(0) => state.info(none),
                Ok(_) => state.needs_repaint = true,
                Err(err) => state.error(err.to_string()),
            }
        }
    }

    // Mixing the lane down to one clip: fewer decoders, and thirty small
    // decisions that can no longer be knocked out of place by accident.
    if is_sound && editor.can_bounce(track) {
        if item(ui, "Bounce to One Clip", "") {
            state.bounce_request = Some(track);
            state.info("Mixing that track down…");
        }
        ui.label(
            egui::RichText::new("Replaces the clips on it with the mix; one undo puts them back")
                .small()
                .color(crate::theme::disabled()),
        );
    }

    // §10's two lane questions: where an import or a paste lands, and which
    // lanes move when a ripple edit moves this one. Both are invisible until
    // something turns up somewhere unexpected, so both say what they do.
    let targeted = editor.track_flag(track, TrackFlag::Targeted);
    let target_label = if targeted {
        "Stop Targeting Track"
    } else {
        "Target Track"
    };
    if item(ui, target_label, "")
        && let Err(err) = editor.set_target_track(track, !targeted)
    {
        state.error(err.to_string());
    }

    // §20a.4's volume line: the lane's level riding along the timeline, as
    // against a clip's own envelope. Started from here because a line with no
    // points has nothing on screen to click.
    if is_sound {
        let points = editor.track_volume(track).len();
        let at = editor.playhead();
        if item(ui, "Add Volume Point at Playhead", "")
            && let Err(err) = editor.add_track_volume_point(track, at)
        {
            state.error(err.to_string());
        }
        if points > 0 {
            if item(ui, "Clear Volume Automation", "") {
                match editor.clear_track_volume(track) {
                    Ok(gone) => state.info(format!("Took {gone} point(s) off the volume line")),
                    Err(err) => state.error(err.to_string()),
                }
            }
        } else {
            ui.label(
                egui::RichText::new("Drag the line to ride the lane; double-click it for a point")
                    .small()
                    .color(crate::theme::disabled()),
            );
        }
    }

    let synced = editor.track_flag(track, TrackFlag::SyncLock);
    let sync_label = if synced {
        "Unlock Sync"
    } else {
        "Sync Lock Track"
    };
    if item(ui, sync_label, "")
        && let Err(err) = editor.set_track_flag(track, TrackFlag::SyncLock, !synced)
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

    // This lane's height alone: a tall waveform to cut dialogue on while
    // the picture lanes stay out of the way. The S/M/L buttons set the rest.
    let own_height = state.lane_heights.get(&track).copied();
    ui.menu_button("Lane Height", |ui| {
        if ui
            .selectable_label(own_height.is_none(), "Same as the others")
            .clicked()
        {
            ui.close();
            state.set_lane_height(track, None);
        }
        for height in crate::state::LaneHeight::ALL {
            let (_, meaning) = height.label();
            if ui
                .selectable_label(own_height == Some(height), meaning)
                .clicked()
            {
                ui.close();
                state.set_lane_height(track, Some(height));
            }
        }
    });

    // The lane's colour: its head and its unlabelled clips, so a tall
    // timeline reads at a glance.
    let lane_colour = editor.track_colour(track);
    ui.menu_button("Lane Colour", |ui| {
        use bettercut_editor_core::timeline::ColorLabel;
        for label in ColorLabel::ALL {
            let swatch = label.rgb().map_or(crate::theme::disabled(), |[r, g, b]| {
                egui::Color32::from_rgb(r, g, b)
            });
            let text = egui::RichText::new(label.name()).color(swatch).strong();
            if ui.selectable_label(lane_colour == label, text).clicked() {
                ui.close();
                match editor.set_track_colour(track, label) {
                    Ok(()) => state.needs_repaint = true,
                    Err(err) => state.error(err.to_string()),
                }
            }
        }
    })
    .response
    .on_hover_text("Tint this lane's head, and its clips that have no colour of their own");

    // The lane's own tone: the clip's three controls, over everything on it.
    // For the room a whole lane was recorded in, rather than each clip.
    if mix.is_some() {
        use bettercut_editor_core::timeline::{EQ_HIGH_CUT_MIN, EQ_LOW_CUT_MAX, EQ_PRESENCE_MAX};
        let mut eq = editor.track_eq(track);
        ui.label(
            egui::RichText::new("Lane EQ")
                .small()
                .color(crate::theme::disabled()),
        );
        let low = ui
            .add(
                egui::Slider::new(&mut eq.low_cut, 0.0..=EQ_LOW_CUT_MAX)
                    .custom_formatter(|v, _| {
                        if v < 1.0 {
                            "off".to_owned()
                        } else {
                            format!("{v:.0} Hz")
                        }
                    })
                    .text("low cut"),
            )
            .on_hover_text("Take out rumble below here, on every clip of the lane");
        let high = ui
            .add(
                egui::Slider::new(&mut eq.high_cut, 0.0..=20_000.0)
                    .logarithmic(true)
                    .custom_formatter(|v, _| {
                        if v < f64::from(EQ_HIGH_CUT_MIN) {
                            "off".to_owned()
                        } else {
                            format!("{:.1} kHz", v / 1000.0)
                        }
                    })
                    .text("high cut"),
            )
            .on_hover_text("Soften hiss and harshness above here, on every clip of the lane");
        let presence = ui
            .add(
                egui::Slider::new(&mut eq.presence, -EQ_PRESENCE_MAX..=EQ_PRESENCE_MAX)
                    .custom_formatter(|v, _| format!("{v:+.1} dB"))
                    .text("presence"),
            )
            .on_hover_text("Bring the lane forward, or set it back");
        if (low.changed() || high.changed() || presence.changed())
            && let Err(err) = editor.set_track_eq(
                track,
                eq,
                low.dragged() || high.dragged() || presence.dragged(),
            )
        {
            state.error(err.to_string());
        }
    }

    ui.separator();

    // The same transition between every shot on the lane: a montage in one go.
    let picture_lane = editor
        .active_sequence()
        .is_some_and(|s| s.video_track(track).is_some());
    if picture_lane {
        ui.menu_button("Transition on Every Cut", |ui| {
            for kind in TransitionKind::ALL {
                if ui.button(kind.label()).on_hover_text(kind.description()).clicked() {
                    ui.close();
                    match editor.transition_every_cut(track, kind) {
                        Ok((0, 0)) => state.info("There are no cuts on this track"),
                        Ok((0, skipped)) => state.error(format!(
                            "None of the {skipped} cut(s) has spare footage for that; try Fade through black"
                        )),
                        Ok((applied, 0)) => state.info(format!("{} on {applied} cut(s)", kind.label())),
                        Ok((applied, skipped)) => state.info(format!(
                            "{} on {applied} cut(s); {skipped} had no spare footage",
                            kind.label()
                        )),
                        Err(err) => state.error(err.to_string()),
                    }
                    state.needs_repaint = true;
                }
            }
            // A different one at each cut, in turn: the montage look.
            if ui
                .button("Mixed")
                .on_hover_text("A different transition at each cut: crossfade, slide, zoom, push, wipe, flash")
                .clicked()
            {
                ui.close();
                match editor.mixed_transition_every_cut(track) {
                    Ok((0, 0)) => state.info("There are no cuts on this track"),
                    Ok((0, skipped)) => state.error(format!(
                        "None of the {skipped} cut(s) has spare footage for a transition"
                    )),
                    Ok((applied, 0)) => state.info(format!("Mixed transitions on {applied} cut(s)")),
                    Ok((applied, skipped)) => state.info(format!(
                        "Mixed transitions on {applied} cut(s); {skipped} had no spare footage"
                    )),
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
            ui.separator();
            // Every transition already there, one length: dissolves added
            // over a week of cutting come out all slightly different.
            ui.menu_button("Make All This Long", |ui| {
                for ms in [250_i64, 500, 1_000, 2_000] {
                    if ui.button(format!("{} s", ms as f64 / 1000.0)).clicked() {
                        ui.close();
                        let length =
                            bettercut_editor_core::foundation::TimelineTime::from_millis(ms);
                        match editor.retime_every_transition(track, length) {
                            Ok(0) => state.info("Every transition is already that long, or has no room"),
                            Ok(n) => state.info(format!("{n} transition(s) made that long")),
                            Err(err) => state.error(err.to_string()),
                        }
                        state.needs_repaint = true;
                    }
                }
            });
            if ui.button("Remove All").clicked() {
                ui.close();
                match editor.remove_every_transition(track) {
                    Ok(n) => state.info(format!("Removed {n} transition(s)")),
                    Err(err) => state.error(err.to_string()),
                }
                state.needs_repaint = true;
            }
        });
    }

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

    // What is drawn over what: a picture lane moved up covers the one it
    // passes. The lane keeps its name, flags and volume line.
    for (label, up) in [("Move Lane Up", true), ("Move Lane Down", false)] {
        if ui.button(label).clicked() {
            ui.close();
            match editor.move_lane(track, up) {
                Ok(true) => state.needs_repaint = true,
                Ok(false) => state.info("That lane is already at the end"),
                Err(err) => state.error(err.to_string()),
            }
        }
    }
    // Two sparse lanes made one, and the empty ones gone: the timeline
    // after an edit that grew more lanes than it needed.
    if editor.lane_below(track).is_some()
        && ui
            .button("Merge Into Lane Below")
            .on_hover_text("Move every clip on this lane onto the one below, at the same times, and remove this lane")
            .clicked()
    {
        ui.close();
        match editor.merge_lane_down(track) {
            Ok(n) => {
                state.clear_selection();
                state.selected_track = None;
                state.needs_repaint = true;
                state.info(format!("Merged {n} clip(s) into the lane below"));
            }
            Err(err) => state.error(err.to_string()),
        }
    }
    if ui
        .button("Remove Empty Lanes")
        .on_hover_text("Remove every lane with nothing on it, keeping one of each kind")
        .clicked()
    {
        ui.close();
        match editor.remove_empty_lanes() {
            Ok(0) => state.info("No empty lanes to remove"),
            Ok(n) => {
                state.selected_track = None;
                state.needs_repaint = true;
                state.info(format!("Removed {n} empty lane(s)"));
            }
            Err(err) => state.error(err.to_string()),
        }
    }

    ui.separator();

    // Removing a track takes its clips with it. That is undoable — the whole
    // track is kept in the undo payload — but it is still the one destructive
    // entry here, so it is coloured and sits alone at the bottom.
    if ui
        .add(egui::Button::new(
            egui::RichText::new("Remove Track").color(crate::theme::error_text()),
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
    if editor.clipboard_len() > 0 && item(ui, "Paste Insert", "Ctrl+Shift+V") {
        shortcuts::paste_insert(editor, state);
    }

    ui.separator();

    if item(ui, "Move Playhead Here", "") {
        editor.set_playhead(at);
        state.needs_repaint = true;
    }
    // Something new, where the click was: the playhead moves there and the
    // thing is placed the way its own button places it.
    ui.menu_button("Add Here", |ui| {
        use bettercut_editor_core::media::{Generated, GeneratedSound};
        let mut added: Option<Result<ClipId, bettercut_editor_core::EditorError>> = None;
        if ui.button("Title").clicked() {
            editor.set_playhead(at);
            added = Some(editor.add_text("Title"));
        }
        if ui.button("Colour Background").clicked() {
            editor.set_playhead(at);
            added = Some(editor.add_colour_clip(Generated::Colour {
                top: [40, 60, 150],
                bottom: [10, 10, 40],
            }));
        }
        for (label, sound, seconds) in [
            ("Whoosh", GeneratedSound::Whoosh, 1),
            ("Click", GeneratedSound::Click, 1),
            ("Riser", GeneratedSound::Riser, 3),
        ] {
            if ui.button(label).clicked() {
                editor.set_playhead(at);
                added = Some(editor.add_generated_sound(
                    sound,
                    bettercut_editor_core::foundation::TimelineTime::from_seconds(seconds),
                ));
            }
        }
        if let Some(result) = added {
            ui.close();
            match result {
                Ok(clip) => {
                    state.select_only(clip);
                    state.needs_repaint = true;
                }
                Err(err) => state.error(err.to_string()),
            }
        }
    })
    .response
    .on_hover_text("A title, a colour background or a sound effect, right here");
    if item(ui, "Split at Playhead", "S") {
        shortcuts::split_at_playhead(editor, state);
    }

    if !editor.markers().is_empty() && item(ui, "Split at Every Marker", "") {
        match editor.split_at_markers(None) {
            Ok(n) => state.info(format!("{n} cut(s) made at the markers")),
            Err(err) => state.error(err.to_string()),
        }
        state.clear_selection();
        state.needs_repaint = true;
    }

    // The marked stretch, on every lane at once: gone with its gap left, or
    // gone with the edit closed up behind it.
    if editor
        .active_sequence()
        .is_some_and(|s| s.marked_range().is_some())
    {
        ui.separator();
        if item(ui, "Lift Marked Range", "") {
            match editor.lift_marked() {
                Ok(n) => state.info(format!("Lifted {n} clip part(s); the gap is left")),
                Err(err) => state.error(err.to_string()),
            }
            state.clear_selection();
            state.needs_repaint = true;
        }
        if item(ui, "Extract Marked Range", "") {
            match editor.extract_marked() {
                Ok(n) => state.info(format!("Extracted {n} clip part(s) and closed the gap")),
                Err(err) => state.error(err.to_string()),
            }
            state.clear_selection();
            state.needs_repaint = true;
        }
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
    // A grid over the edit: a chapter every minute of a talk, a cut point
    // every two seconds of a montage.
    ui.menu_button("Add Markers Every", |ui| {
        for (label, ms) in [
            ("1 s", 1_000_i64),
            ("2 s", 2_000),
            ("5 s", 5_000),
            ("10 s", 10_000),
            ("30 s", 30_000),
            ("1 min", 60_000),
            ("5 min", 300_000),
        ] {
            if ui.button(label).clicked() {
                ui.close();
                let interval = bettercut_editor_core::foundation::TimelineTime::from_millis(ms);
                match editor.add_markers_every(interval) {
                    Ok(0) => state.info("No room for a marker that far apart"),
                    Ok(n) => {
                        state.needs_repaint = true;
                        state.info(format!("Added {n} marker(s)"));
                    }
                    Err(err) => state.error(err.to_string()),
                }
            }
        }
    })
    .response
    .on_hover_text("A marker at every interval, over the marked range or the whole edit");
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
    if let Some(track) = track
        && item(ui, "Select All on This Track", "")
    {
        shortcuts::select_from(
            editor,
            state,
            bettercut_editor_core::foundation::TimelineTime::ZERO,
            Some(track),
        );
    }
    // Pick the shots to keep, invert, delete the rest.
    if !state.selected_clips.is_empty() && item(ui, "Invert Selection", "") {
        shortcuts::invert_selection(editor, state);
    }
    if state.selected_clips.len() > 1 && item(ui, "New Sequence from Selection", "") {
        let chosen: Vec<ClipId> = state.selected_clips.iter().copied().collect();
        match editor.sequence_from_clips(&chosen, "") {
            Ok(_) => {
                state.needs_repaint = true;
                state.info("A new sequence of the selected clips is in the tabs above");
            }
            Err(err) => state.error(err.to_string()),
        }
    }
    // After a rough assembly: every lane butted up at once.
    if ui
        .button("Close Gaps on Every Lane")
        .on_hover_text("Pull every clip left against the one before it, on every unlocked lane")
        .clicked()
    {
        ui.close();
        match editor.close_gaps_everywhere() {
            Ok(n) => state.info(format!("Closed {n} gap(s)")),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }
    // The one-frame leftovers of fast cutting: a flash of the wrong shot,
    // a blink of black.
    if ui
        .button("Clean Up Slivers")
        .on_hover_text(
            "Remove clips and close gaps shorter than two frames, on every unlocked lane",
        )
        .clicked()
    {
        ui.close();
        match editor.clean_up_slivers(2) {
            Ok((0, 0)) => state.info("No slivers — nothing shorter than two frames"),
            Ok((clips, gaps)) => {
                state.clear_selection();
                state.needs_repaint = true;
                state.info(format!(
                    "Removed {clips} sliver clip(s) and closed {gaps} gap(s)"
                ));
            }
            Err(err) => state.error(err.to_string()),
        }
    }
    if item(ui, "Select Under Playhead", "") {
        let under = editor.clips_under_playhead();
        state.clear_selection();
        state.selected_clips.extend(under);
        state.needs_repaint = true;
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
        .map(|asset| (asset.id, asset.display_name().to_owned()))
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

/// Line `clip` up with `reference` by their recordings, and say what happened.
///
/// The listening is [`bettercut_cache::align`]; everything here is about
/// having the peaks to hand and telling the user when the answer is not one to
/// trust. A match nobody believes is worse than no match: it moves a clip to
/// somewhere arbitrary and looks like a bug.
fn sync_by_sound(editor: &mut Editor, state: &mut UiState, clip: ClipId, reference: ClipId) {
    let (Some(clip_media), Some(reference_media)) =
        (editor.media_of_clip(clip), editor.media_of_clip(reference))
    else {
        state.error("Both clips need sound to line up by");
        return;
    };
    let (Some(other), Some(against)) = (
        state.waveforms.get(clip_media),
        state.waveforms.get(reference_media),
    ) else {
        state.info("Still reading the sound — try again in a moment");
        return;
    };
    let Some(found) = bettercut_cache::align(&against, &other) else {
        state.error("Not enough sound in these two to line them up");
        return;
    };
    if !found.is_convincing() {
        state.error("Could not find the same sound in both clips");
        return;
    }
    match editor.sync_to_sound(clip, reference, found.seconds) {
        Ok(start) => {
            state.info(format!("Lined up at {}", start.format_timecode()));
            state.needs_repaint = true;
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// A span in seconds, for the words a menu says about it.
fn seconds_of(span: bettercut_editor_core::foundation::TimelineTime) -> f64 {
    span.ticks() as f64 / bettercut_editor_core::foundation::TICKS_PER_SECOND as f64
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
