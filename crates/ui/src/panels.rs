//! The §58 panels: toolbar, media browser, preview, inspector, status bar.
//!
//! Every control here either reads through `editor.project()` or dispatches a
//! command. None of them mutate project data (§54).

use bettercut_editor_core::foundation::{FrameRate, MediaId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::project_format::PerformanceMode;
use bettercut_editor_core::timeline::{AnimatedParameter, ColorAdjust, Resolution, VideoClip};
use bettercut_editor_core::{Editor, Movement, SettingChange, TrackFlag};

use crate::state::UiState;
use crate::theme;

/// Toolbar: the actions a user reaches for constantly, and nothing else (§58).
pub fn toolbar(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    preview: Option<&mut crate::Preview>,
) {
    ui.horizontal(|ui| {
        // Transport first: it is the control reached for most often.
        if let Some(preview) = preview {
            let playing = preview.is_playing();
            if ui
                .button(if playing { "Pause" } else { "Play" })
                .on_hover_text("Space")
                .clicked()
            {
                preview.set_playing(editor, !playing);
                state.needs_repaint = true;
            }
            ui.separator();
        }

        if ui.button("New").on_hover_text("New project").clicked() {
            new_project(editor, state);
        }
        // A menu rather than a button: the project wanted is usually one
        // opened yesterday, and a file dialog is a long way round to it.
        ui.menu_button("Open…", |ui| {
            if ui
                .button("Browse…")
                .on_hover_text("Choose a project file")
                .clicked()
            {
                ui.close();
                open_project(editor, state);
            }
            ui.separator();
            if state.recent.is_empty() {
                ui.label(
                    egui::RichText::new("No recent projects yet")
                        .small()
                        .color(theme::disabled()),
                );
                return;
            }
            let mut chosen = None;
            for path in state.recent.paths() {
                let (name, folder) = crate::recent::menu_label(path);
                let there = path.exists();
                let text = if there {
                    egui::RichText::new(name)
                } else {
                    egui::RichText::new(format!("{name} (not found)")).color(theme::disabled())
                };
                if ui
                    .button(text)
                    .on_hover_text(if there {
                        folder
                    } else {
                        format!("{folder}\nMoved, renamed or on a drive that is not connected")
                    })
                    .clicked()
                {
                    chosen = Some(path.clone());
                }
            }
            ui.separator();
            if ui.button("Clear list").clicked() {
                ui.close();
                state.recent.clear();
            }
            if let Some(path) = chosen {
                ui.close();
                open_project_at(editor, state, &path);
            }
        })
        .response
        .on_hover_text("Open a project, or one you worked on recently");
        if ui.button("Save").on_hover_text(crate::keys::keys("Ctrl+S")).clicked() {
            save_project(editor, state);
        }
        ui.menu_button("Save…", |ui| {
            if ui
                .button("Save As…")
                .on_hover_text(crate::keys::keys(
                    "Save under a new name and keep working in that file (Ctrl+Shift+S)",
                ))
                .clicked()
            {
                ui.close();
                save_project_as(editor, state);
            }
            if ui
                .button("Save a Copy…")
                .on_hover_text("Write a copy to keep, and carry on in this file")
                .clicked()
            {
                ui.close();
                save_copy(editor, state);
            }
            // Everything in one folder, for handing the edit to somebody else
            // or putting it on another drive.
            let (files, bytes) = editor.collect_size();
            if ui
                .add_enabled(files > 0, egui::Button::new("Collect Files…"))
                .on_hover_text(format!(
                    "Copy this project and the {files} file(s) it uses — about {} — into \
                     one folder. Nothing is moved or deleted.",
                    readable_bytes(bytes)
                ))
                .on_disabled_hover_text("This project uses no files yet")
                .clicked()
            {
                ui.close();
                collect_files(editor, state);
            }
            // Earlier saves, kept beside the project as it is saved.
            let versions = editor.versions();
            ui.add_enabled_ui(!versions.is_empty(), |ui| {
                ui.menu_button("Go Back to a Version", |ui| {
                    ui.label(
                        egui::RichText::new("Each save keeps the one before. Going back keeps the current project too.")
                            .small()
                            .color(theme::disabled()),
                    );
                    let mut chosen = None;
                    for version in &versions {
                        if ui.button(&version.label).clicked() {
                            chosen = Some(version.path.clone());
                            ui.close();
                        }
                    }
                    if let Some(path) = chosen {
                        match editor.restore_version(&path) {
                            Ok(()) => {
                                reset_state(state);
                                state.info("Went back to an earlier version. Save to keep it");
                            }
                            Err(err) => state.error(err.to_string()),
                        }
                    }
                })
                .response
                .on_disabled_hover_text("Earlier versions appear here once the project has been saved more than once");

                // What is *in* a version, before deciding to go back to it: a
                // list of dates says when each save happened and nothing about
                // what it holds (`editor_core::compare`).
                ui.menu_button("What Changed Since…", |ui| {
                    ui.label(
                        egui::RichText::new("Compare this edit with an earlier save. Nothing is opened or changed.")
                            .small()
                            .color(theme::disabled()),
                    );
                    let mut asked = None;
                    for version in &versions {
                        if ui.button(&version.label).clicked() {
                            asked = Some((version.path.clone(), version.label.clone()));
                            ui.close();
                        }
                    }
                    if let Some((path, label)) = asked {
                        match editor.changes_since(&path) {
                            Ok(changes) => state.version_changes.show_changes(label, changes),
                            Err(err) => state.error(err.to_string()),
                        }
                    }
                })
                .response
                .on_disabled_hover_text("Earlier versions appear here once the project has been saved more than once");
            });
        });
        if ui
            .add(theme::primary_button("Export…"))
            .on_hover_text("Render the timeline to a video file")
            .clicked()
        {
            state.export_dialog.open(editor);
        }

        ui.separator();

        // Plain words and ASCII, not arrow glyphs. ↶ and ↷ *are* bundled — but
        // only in Hack, the monospace font, and a button draws with the
        // proportional family, so they arrive as empty boxes. ＋ and － are in
        // no bundled font at all. egui never falls back to the system's fonts.
        //
        // `tests/glyphs.rs` checks this per family for every symbol the
        // interface uses, rather than leaving it to a comment; it found the
        // keyframe diamonds failing the same way.
        let undo_label = editor
            .undo_label()
            .map_or_else(|| "Nothing to undo".to_owned(), |l| format!("Undo {l}"));
        if ui
            .add_enabled(editor.can_undo(), egui::Button::new("Undo"))
            .on_hover_text(undo_label)
            .clicked()
            && let Err(err) = editor.undo()
        {
            state.error(err.to_string());
        }

        let redo_label = editor
            .redo_label()
            .map_or_else(|| "Nothing to redo".to_owned(), |l| format!("Redo {l}"));
        if ui
            .add_enabled(editor.can_redo(), egui::Button::new("Redo"))
            .on_hover_text(redo_label)
            .clicked()
            && let Err(err) = editor.redo()
        {
            state.error(err.to_string());
        }

        ui.separator();

        // §26: next to the transport rather than buried in a menu. Adding a
        // title is one of the two or three things anyone does in a short-form
        // editor, and the playhead is already where they want it.
        // The same file again: the fifth version of a cut that is nearly
        // right should not be five trips through a window.
        if ui
            .add_enabled(
                state.export_dialog.has_exported(),
                egui::Button::new("Quick Export"),
            )
            .on_hover_text("Write the same file as last time, to the same folder, with these settings")
            .on_disabled_hover_text("Export once first, and this repeats it")
            .clicked()
        {
            state.export_dialog.quick_export(editor);
        }

        // Everything that puts something new on the timeline, one menu.
        ui.menu_button("Add", |ui| {
            ui.menu_button("Add Shape", |ui| {
                for kind in bettercut_editor_core::text::ShapeKind::ALL {
                    if ui.button(kind.label()).clicked() {
                        ui.close();
                        match editor.add_shape(kind) {
                            Ok(clip) => {
                                state.select_only(clip);
                                state.info(format!("{} added at the playhead", kind.label()));
                            }
                            Err(err) => state.error(err.to_string()),
                        }
                    }
                }
            })
            .response
            .on_hover_text("A box, an ellipse, an arrow, a star or a speech bubble on the title lane, placed like a title");
            // Who is talking: a name, a role and a bar, sliding in low on the left.
            ui.menu_button("Lower Third", |ui| {
                ui.set_min_width(240.0);
                let (name, role, accent) = &mut state.lower_third;
                ui.add(egui::TextEdit::singleline(name).hint_text("Name"));
                ui.add(egui::TextEdit::singleline(role).hint_text("Role (optional)"));
                ui.horizontal(|ui| {
                    ui.label("bar");
                    ui.color_edit_button_srgb(accent);
                });
                let ready = !name.trim().is_empty();
                if ui
                    .add_enabled(ready, egui::Button::new("Add at the Playhead"))
                    .on_disabled_hover_text("Type a name first")
                    .clicked()
                {
                    let (name, role, accent) = state.lower_third.clone();
                    ui.close();
                    match editor.add_lower_third(&name, &role, accent) {
                        Ok(parts) => {
                            state.clear_selection();
                            for part in parts {
                                state.selected_clips.insert(part);
                            }
                            state.info("Lower third added");
                            state.needs_repaint = true;
                        }
                        Err(err) => state.error(err.to_string()),
                    }
                }
            })
            .response
            .on_hover_text("A name and a role with a coloured bar, low on the left, sliding in");
            ui.menu_button("Add Timer", |ui| {
                for direction in bettercut_editor_core::timeline::CountDirection::ALL {
                    if ui.button(direction.label()).clicked() {
                        ui.close();
                        match editor.add_counter(direction) {
                            Ok(clip) => {
                                state.select_only(clip);
                                state.info(format!("{} added", direction.label()));
                            }
                            Err(err) => state.error(err.to_string()),
                        }
                    }
                }
            })
            .response
            .on_hover_text(
                "A countdown or a stopwatch on the title lane — trim it to set how long it runs",
            );
            if ui
                .button("Add Text")
                .on_hover_text("Put a title at the playhead")
                .clicked()
            {
                match editor.add_text("Text") {
                    Ok(clip) => {
                        // Selected straight away, so the inspector is already
                        // showing the box to type in — the next thing they want.
                        state.selected_clips.clear();
                        state.selected_clips.insert(clip);
                        state.inspector_tab = InspectorTab::Video;
                        state.needs_repaint = true;
                    }
                    Err(err) => state.error(err.to_string()),
                }
            }

            // Beside Add Text, because it is the other thing laid over the footage.
            if ui
                .button("Add Adjustment")
                .on_hover_text(
                    "Grade a stretch of the edit: everything beneath it, for as long \
                     as it runs. Titles on top are left alone.",
                )
                .clicked()
            {
                match editor.add_adjustment() {
                    Ok(clip) => {
                        // Selected straight away, so the Inspector is already
                        // showing its controls: an adjustment does nothing until
                        // it is told what to do.
                        state.selected_clips.clear();
                        state.selected_clips.insert(clip);
                        state.needs_repaint = true;
                    }
                    Err(err) => state.error(err.to_string()),
                }
            }

            // A background to go under titles and footage: a flat colour or a
            // gradient, on the lowest picture lane with room at the playhead.
            ui.menu_button("Add Colour", |ui| {
                for (name, colour) in COLOUR_CLIP_PRESETS {
                    if ui.button(name).clicked() {
                        match editor.add_colour_clip(colour) {
                            Ok(clip) => {
                                state.selected_clips.clear();
                                state.selected_clips.insert(clip);
                                state.inspector_tab = InspectorTab::Video;
                                state.needs_repaint = true;
                            }
                            Err(err) => state.error(err.to_string()),
                        }
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(
                "A solid colour or gradient clip at the playhead, under the footage. \
                 Change its colours in the Inspector.",
            );

            // Sound with no recording behind it: what a delivery is lined up
            // against, and a gap somebody chose (`editor_core::tone`).
            ui.menu_button("Add Sound", |ui| {
                use bettercut_editor_core::media::GeneratedSound;
                let beep = "Sync Beep";
                let mut made: Option<(GeneratedSound, bettercut_editor_core::foundation::TimelineTime)> =
                    None;
                let mut beeped = false;
                for (name, sound, seconds, hint) in [
                    (
                        "1 kHz Tone · 30 s",
                        GeneratedSound::LINE_UP,
                        30_i64,
                        "A steady 1 kHz at -18 dBFS: what whoever receives the file lines their \
                         meters up against",
                    ),
                    (
                        "Silence · 2 s",
                        GeneratedSound::Silence,
                        2,
                        "Silence you put there, which reads differently from a gap nobody noticed \
                         — trim, move and fade it like any clip",
                    ),
                    (
                        "Whoosh",
                        GeneratedSound::Whoosh,
                        1,
                        "A breath of air, dark to bright, under a fast move or a title flying in",
                    ),
                    (
                        "Click",
                        GeneratedSound::Click,
                        1,
                        "A short tick: a button, a cut, a beat",
                    ),
                    (
                        "Riser · 2 s",
                        GeneratedSound::Riser,
                        2,
                        "A tone climbing in pitch and level: the run-up to a drop or a reveal",
                    ),
                ] {
                    if ui.button(name).on_hover_text(hint).clicked() {
                        made = Some((
                            sound,
                            bettercut_editor_core::foundation::TimelineTime::from_seconds(seconds),
                        ));
                        ui.close();
                    }
                }
                if ui
                    .button(beep)
                    .on_hover_text(
                        "One frame of 1 kHz at the playhead: the frame it is on is the frame the \
                         picture mark lines up with",
                    )
                    .clicked()
                {
                    beeped = true;
                    ui.close();
                }

                let placed = if beeped {
                    Some(editor.add_sync_beep())
                } else {
                    made.map(|(sound, length)| editor.add_generated_sound(sound, length))
                };
                match placed {
                    Some(Ok(clip)) => {
                        state.selected_clips.clear();
                        state.selected_clips.insert(clip);
                        state.inspector_tab = InspectorTab::Audio;
                        state.needs_repaint = true;
                    }
                    Some(Err(err)) => state.error(err.to_string()),
                    None => {}
                }
            })
            .response
            .on_hover_text(
                "A line-up tone, a sync beep or a stretch of silence at the playhead, on the first \
                 sound lane with room for it",
            );

            // A leader at the head of the edit: black, the numbers, a beep a
            // second. One undo step, made of ordinary clips (`editor_core::count_in`).
            ui.menu_button("Count-In", |ui| {
                use bettercut_editor_core::count_in::{DEFAULT_COUNT_IN_SECONDS, MAX_COUNT_IN_SECONDS};
                ui.label(
                    egui::RichText::new(
                        "Black, a countdown and a beep on every second, put in front of the whole \
                         edit. Everything moves later by that much.",
                    )
                    .small()
                    .color(theme::disabled()),
                );
                let mut chosen = None;
                for seconds in [3_i64, DEFAULT_COUNT_IN_SECONDS, MAX_COUNT_IN_SECONDS] {
                    if ui.button(format!("{seconds} seconds")).clicked() {
                        chosen = Some(seconds);
                        ui.close();
                    }
                }
                if let Some(seconds) = chosen {
                    match editor.add_count_in(seconds) {
                        Ok(count_in) => {
                            state.selected_clips.clear();
                            state.selected_clips.insert(count_in.countdown);
                            state.needs_repaint = true;
                            state.info(format!("Put a {seconds}-second count-in at the head"));
                        }
                        Err(err) => state.error(err.to_string()),
                    }
                }
            })
            .response
            .on_hover_text("A count-in leader at the head of the edit: black, numbers, a beep a second");

            // A symbol on the title lane: a heart, a tick, a lightning bolt. Four
            // to a row, so the grid reads as a palette rather than as a list.
            ui.menu_button("Sticker", |ui| {
                use bettercut_editor_core::stickers::STICKERS;
                let mut chosen: Option<&str> = None;
                egui::Grid::new("stickers").spacing([2.0, 2.0]).show(ui, |ui| {
                    for (index, (sticker, name)) in STICKERS.iter().enumerate() {
                        if ui
                            .add(egui::Button::new(egui::RichText::new(*sticker).size(22.0)))
                            .on_hover_text(*name)
                            .clicked()
                        {
                            chosen = Some(sticker);
                        }
                        if index % 4 == 3 {
                            ui.end_row();
                        }
                    }
                });
                if let Some(sticker) = chosen {
                    match editor.add_sticker(sticker) {
                        Ok(clip) => {
                            state.select_only(clip);
                            state.inspector_tab = InspectorTab::Video;
                            state.needs_repaint = true;
                        }
                        Err(err) => state.error(err.to_string()),
                    }
                    ui.close();
                }
            })
            .response
            .on_hover_text("Put a symbol on the title lane at the playhead");

            // §31: a whole edit from a few clips. Beside the other things that put
            // something on the timeline.
            if ui
                .button("Templates")
                .on_hover_text("Start from a ready-made edit and fill in your clips")
                .clicked()
            {
                state.template_dialog.open();
            }
        })
        .response
        .on_hover_text("Text, lower thirds, shapes, timers, stickers, colours, sounds and templates");

        // One slot rather than two buttons: importing subtitles is something
        // done once per project, and the toolbar is already the busiest strip
        // in the window.
        ui.menu_button("Captions", |ui| {
            if ui.button("Import…").clicked() {
                ui.close();
                import_captions(editor, state);
            }
            if ui.button("Export…").clicked() {
                ui.close();
                export_captions(editor, state);
            }
        })
        .response
        .on_hover_text("Subtitles, as .srt or .vtt");

        ui.separator();

        // The timeline's own controls, one menu: zoom, lane heights,
        // snapping, magnetic, split and new tracks.
        ui.menu_button("Timeline", |ui| {
            ui.label("Zoom");
            if ui
                .add_enabled(state.can_zoom_out(), egui::Button::new("-"))
                .on_hover_text("Zoom out (Ctrl + scroll)")
                .clicked()
            {
                state.zoom_out();
            }
            if ui
                .add_enabled(state.can_zoom_in(), egui::Button::new("+"))
                .on_hover_text("Zoom in (Ctrl + scroll)")
                .clicked()
            {
                state.zoom_in();
            }
            if ui.button("Fit").on_hover_text("Zoom to fit").clicked() {
                let duration = editor
                    .active_sequence()
                    .map_or(TimelineTime::ZERO, |s| s.duration());
                let lanes = (ui.ctx().content_rect().width() - theme::TRACK_HEADER_WIDTH).max(200.0);
            state.zoom_to_fit(duration, lanes);
            }
            let selection = state.selection_range(editor);
            if ui
                .add_enabled(selection.is_some(), egui::Button::new("Selection"))
                .on_hover_text("Zoom in on the selected clips")
                .on_disabled_hover_text("Select clips to zoom in on them")
                .clicked()
                && let Some(range) = selection
            {
                // The lanes, not the whole window: the track names take the left.
                let lanes = (ui.ctx().content_rect().width() - theme::TRACK_HEADER_WIDTH).max(200.0);
                state.zoom_to_range(range, lanes);
            }

            ui.separator();
            ui.label("Tracks");
            for height in crate::state::LaneHeight::ALL {
                let (letter, meaning) = height.label();
                if ui
                    .selectable_label(state.lane_height == height, letter)
                    .on_hover_text(meaning)
                    .clicked()
                {
                    state.lane_height = height;
                    state.needs_repaint = true;
                }
            }

            ui.separator();

            ui.checkbox(&mut state.snapping, "Snap")
                .on_hover_text("Snap edits to clip edges and the playhead (N). Hold Alt to bypass.");

            // A magnetic main track: the first picture lane stays packed, so a
            // delete or a drag closes up behind it.
            let mut magnetic = editor.is_magnetic();
            if ui
                .checkbox(&mut magnetic, "Magnetic")
                .on_hover_text("Keep the main track's clips together from the start: deleting or moving one closes the gap")
                .changed()
            {
                match editor.dispatch(bettercut_editor_core::Command::ChangeSetting {
                    change: bettercut_editor_core::SettingChange::MagneticTimeline(magnetic),
                }) {
                    Ok(()) => {
                        if magnetic {
                            // Turning it on packs the track now, as part of the
                            // same step, so it is magnetic from the first moment.
                            crate::shortcuts::close_up_if_magnetic(editor, state);
                        }
                    }
                    Err(err) => state.error(err.to_string()),
                }
            }

            if ui
                .button("Split")
                .on_hover_text("Split at the playhead (S)")
                .clicked()
            {
                let selected: Vec<_> = state.selected_clips.iter().copied().collect();
                match editor.split_at_playhead(&selected) {
                    Ok(0) => state.info("Nothing under the playhead to split"),
                    Ok(n) => {
                        state.clear_selection();
                        state.info(format!("Split {n} clip(s)"));
                    }
                    Err(err) => state.error(err.to_string()),
                }
            }

            ui.separator();

            if ui.button("+ Video track").clicked() {
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
            if ui.button("+ Audio track").clicked() {
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
        })
        .response
        .on_hover_text("Zoom, track height, snapping, magnetic, split, and new tracks");
        ui.separator();
        // Find clips by file name, title words or note; Enter goes to the next
        // one after the playhead, and round again from the start.
        let field = ui.add(
            egui::TextEdit::singleline(&mut state.find_query)
                .desired_width(130.0)
                .hint_text("Find clips"),
        );
        let found = editor.find_clips(&state.find_query);
        if !state.find_query.trim().is_empty() {
            ui.label(
                egui::RichText::new(match found.len() {
                    0 => "none".to_owned(),
                    n => format!("{n} found"),
                })
                .small()
                .color(theme::disabled()),
            );
        }
        if field.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && !found.is_empty()
        {
            let playhead = editor.playhead();
            let next = found
                .iter()
                .find(|f| f.start > playhead)
                .unwrap_or(&found[0]);
            editor.set_playhead(next.start);
            state.select_only(next.clip);
            let lanes = (ui.ctx().content_rect().width() - theme::TRACK_HEADER_WIDTH).max(200.0);
            state.follow_playhead(next.start, lanes);
            state.info(format!("Found {}", next.label));
            state.needs_repaint = true;
            // Keep the box ready for the next Enter.
            field.request_focus();
        }


        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The windows, one menu: ten buttons in a row read as clutter.
            ui.menu_button("Windows", |ui| {
                // The last thing on the right, where people look for help.
                if ui
                    .button("Captions")
                    .on_hover_text("Every caption in one list, editable in place")
                    .clicked()
                {
                    state.captions_open = !state.captions_open;
                }
                if ui
                    .button("Markers")
                    .on_hover_text("Every marker in one list: jump to one, name it, delete it")
                    .clicked()
                {
                    state.markers_open = !state.markers_open;
                }
                if ui
                    .button("Notes")
                    .on_hover_text("Notes about the edit, saved with the project: to-dos, what the client asked for")
                    .clicked()
                {
                    state.notes_open = !state.notes_open;
                }
                if ui
                    .button("Trim")
                    .on_hover_text("Both sides of the cut nearest the playhead, a frame at a time")
                    .clicked()
                {
                    state.trim.open = !state.trim.open;
                }
                if ui
                    .button("Storyboard")
                    .on_hover_text("The cut as a row of cards: drag one to move that shot")
                    .clicked()
                {
                    state.storyboard_open = !state.storyboard_open;
                }
                if ui
                    .button("Sound")
                    .on_hover_text(
                        "The selected sound clip drawn tall: zoom in, read the level, place the playhead exactly",
                    )
                    .clicked()
                {
                    state.waveform_view.open = !state.waveform_view.open;
                }
                if ui
                    .button("Scopes")
                    .on_hover_text("Histogram and waveform of the frame under the playhead")
                    .clicked()
                {
                    state.scopes.open = !state.scopes.open;
                }
                if ui
                    .button("History")
                    .on_hover_text(crate::keys::keys(
                        "Every step of the edit; click one to go back to it (Ctrl+H)",
                    ))
                    .clicked()
                {
                    state.history_open = !state.history_open;
                }
                if ui
                    .button("Shortcuts")
                    .on_hover_text("Every keyboard shortcut (? or F1)")
                    .clicked()
                {
                    state.shortcuts_open = !state.shortcuts_open;
                }
                if ui
                    .button("AI Assistant")
                    .on_hover_text("Connect Claude or another AI assistant to edit with you")
                    .clicked()
                {
                    state.assistant_open = !state.assistant_open;
                }
            })
            .response
            .on_hover_text("Captions, markers, notes, scopes, history and the other windows");
            // The palette, findable without knowing its key.
            if ui
                .button("Actions")
                .on_hover_text(crate::keys::keys("Find any action by typing its name (Ctrl+K)"))
                .clicked()
            {
                state.palette_open = !state.palette_open;
                state.palette_query.clear();
                state.palette_pick = 0;
            }
            timecode_readout(ui, editor, state);
        });
    });
}

/// The playhead's timecode, and a box to type one into when it is clicked:
/// Enter goes there (`crate::timecode_entry`), Escape leaves it.
fn timecode_readout(ui: &mut egui::Ui, editor: &Editor, state: &mut UiState) {
    let shown = editor.display_time(editor.playhead()).format_timecode();
    let Some(draft) = state.timecode_draft.as_mut() else {
        let readout = ui
            .add(
                egui::Label::new(egui::RichText::new(&shown).monospace().size(15.0))
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(
                "Click to type a timecode: 1:02:03, 2:03.5, 120f, or +10 / -1:00 to move. \
                 Right-click to copy it",
            );
        // For a note to the client or a comment on a review: the time,
        // exactly as shown, on the clipboard.
        readout.context_menu(|ui| {
            if ui.button("Copy Timecode").clicked() {
                ui.ctx().copy_text(shown.clone());
                state.info(format!("Copied {shown}"));
                ui.close();
            }
        });
        if readout.clicked() {
            state.timecode_draft = Some(shown);
            state.needs_repaint = true;
        }
        return;
    };
    let field = ui.add(
        egui::TextEdit::singleline(draft)
            .font(egui::TextStyle::Monospace)
            .desired_width(120.0),
    );
    if state.timecode_draft.as_ref().is_some_and(|d| d == &shown) && !field.has_focus() {
        // Just opened: take the keyboard, with the whole time selected so
        // typing replaces it.
        field.request_focus();
    }
    let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if entered {
        let rate = editor
            .active_sequence()
            .map_or(FrameRate::PAL_25, |s| s.frame_rate);
        let typed = state.timecode_draft.take().unwrap_or_default();
        match crate::timecode_entry::parse(&typed, rate) {
            Some(jump) => {
                state.jump_to = Some(crate::timecode_entry::resolve(
                    jump,
                    editor.playhead(),
                    editor.start_timecode(),
                ));
            }
            None => state.error(format!(
                "\"{}\" is not a timecode — try 1:02:03, 45, 120f or +10",
                typed.trim()
            )),
        }
        state.needs_repaint = true;
    } else if escaped || field.lost_focus() {
        state.timecode_draft = None;
        state.needs_repaint = true;
    }
}

/// The sequences, as tabs above the timeline: click one to work on it, "+" for
/// a new one, right-click a tab to rename, duplicate or delete it.
pub fn sequence_tabs(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    let sequences = editor.sequence_list();
    let active = editor.active_sequence().map(|s| s.id);
    let only_one = sequences.len() == 1;
    let mut switch_to = None;
    let mut duplicate = None;
    let mut reshaped = None;
    let mut save_template = None;
    let mut remove = None;
    let mut menu_open = false;

    ui.horizontal_wrapped(|ui| {
        for (id, name) in &sequences {
            let tab = ui
                .selectable_label(active == Some(*id), name)
                .on_hover_text("Right-click to rename, duplicate or delete");
            if tab.clicked() {
                switch_to = Some(*id);
            }
            tab.context_menu(|ui| {
                menu_open = true;
                let mut draft = match &state.sequence_name_draft {
                    Some((draft_id, text)) if draft_id == id => text.clone(),
                    _ => name.clone(),
                };
                let field = ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .desired_width(160.0)
                        .char_limit(Editor::MAX_TRACK_NAME)
                        .hint_text("sequence name"),
                );
                if field.changed() {
                    state.sequence_name_draft = Some((*id, draft));
                }
                ui.separator();
                if ui.button("Duplicate").clicked() {
                    duplicate = Some(*id);
                    ui.close();
                }
                // This edit's timing, framing and titles as a template, its
                // footage left as slots for the next video.
                if ui
                    .button("Save as Template")
                    .on_hover_text("Keep this edit's timing, transitions and titles as a template to fill with other clips")
                    .clicked()
                {
                    ui.close();
                    save_template = Some((*id, name.clone()));
                }
                // Another shape of the same edit: the vertical cut of a
                // landscape video, with every shot cropped to fill it.
                ui.menu_button("Copy as Shape", |ui| {
                    let size = editor.project().sequence(*id).map(|s| s.resolution);
                    for shape in bettercut_editor_core::reshape::SHAPES {
                        if size.is_some_and(|size| shape.matches(size)) {
                            continue;
                        }
                        if ui.button(shape.label).on_hover_text(shape.hint).clicked() {
                            reshaped = Some((*id, shape));
                            ui.close();
                        }
                    }
                });
                if ui
                    .add_enabled(
                        !only_one,
                        egui::Button::new(egui::RichText::new("Delete").color(theme::error_text())),
                    )
                    .on_disabled_hover_text("A project needs at least one sequence")
                    .on_hover_text("Undoable")
                    .clicked()
                {
                    remove = Some(*id);
                    ui.close();
                }
            });
        }
        if ui
            .button("+")
            .on_hover_text("A new, empty sequence in the same format")
            .clicked()
        {
            match editor.add_sequence() {
                Ok(_) => {
                    state.clear_selection();
                    state.info("New sequence");
                }
                Err(err) => state.error(err.to_string()),
            }
        }
    });

    // The name typed into a tab's menu, once no menu is open to type into.
    if !menu_open && let Some((id, name)) = state.sequence_name_draft.take() {
        match editor.rename_sequence(id, &name) {
            Ok(true) => state.info(format!("Sequence renamed to {}", name.trim())),
            Ok(false) => {}
            Err(err) => state.error(err.to_string()),
        }
    }
    if let Some(id) = switch_to
        && editor.switch_sequence(id)
    {
        state.clear_selection();
        state.needs_repaint = true;
    }
    if let Some(id) = duplicate {
        match editor.duplicate_sequence(id) {
            Ok(_) => {
                state.clear_selection();
                state.info("Sequence duplicated");
            }
            Err(err) => state.error(err.to_string()),
        }
    }
    if let Some((id, name)) = save_template {
        if editor.switch_sequence(id) {
            state.clear_selection();
        }
        let folder = bettercut_editor_core::templates::library::user_dir();
        match editor.save_as_template(&name, &folder) {
            Ok(saved) => state.info(if saved.left_out > 0 {
                format!(
                    "Saved \"{name}\" as a template with {} slot(s); {} shape(s) or timer(s) left out",
                    saved.slots, saved.left_out
                )
            } else {
                format!("Saved \"{name}\" as a template with {} slot(s)", saved.slots)
            }),
            Err(err) => state.error(err.to_string()),
        }
    }
    if let Some((id, shape)) = reshaped {
        match editor.copy_sequence_as(id, shape) {
            Ok(_) => {
                state.clear_selection();
                state.info(format!("{} copy made", shape.label));
            }
            Err(err) => state.error(err.to_string()),
        }
    }
    if let Some(id) = remove {
        match editor.remove_sequence(id) {
            Ok(()) => {
                state.clear_selection();
                state.info("Sequence deleted");
            }
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// Apply the note being typed, if there is one.
pub fn commit_note(editor: &mut Editor, state: &mut UiState) {
    let Some((clip, text)) = state.note_draft.take() else {
        return;
    };
    match editor.set_clip_note(clip, &text) {
        Ok(true) => state.info(if text.trim().is_empty() {
            "Note removed"
        } else {
            "Note saved"
        }),
        Ok(false) => {}
        Err(err) => state.error(err.to_string()),
    }
    state.needs_repaint = true;
}

/// A note on the selected clip: typed into a draft and applied when the field
/// is left, so a note is one undo step rather than one a letter.
fn note_field(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    let mut text = match &state.note_draft {
        Some((id, draft)) if *id == clip => draft.clone(),
        _ => editor.clip_note(clip).unwrap_or("").to_owned(),
    };
    let field = ui.add(
        egui::TextEdit::multiline(&mut text)
            .desired_rows(1)
            .desired_width(f32::INFINITY)
            .char_limit(Editor::MAX_CLIP_NOTE)
            .hint_text("Add a note to this clip"),
    );
    if field.changed() {
        state.note_draft = Some((clip, text));
    }
    if field.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.note_draft = None;
        } else {
            commit_note(editor, state);
        }
    }
    ui.add_space(4.0);
}

/// Media browser (§58). Import lands here; placing on the timeline is one click.
pub fn media_browser(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    // One line: the panel's name, and Import — with the rarer ways in
    // behind More.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Media").strong().color(theme::ruler_text()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("More", |ui| {
                if ui
                    .button("Import Sequence…")
                    .on_hover_text(
                        "Numbered stills (frame_0001.png, frame_0002.png…) as one clip at the sequence's frame rate. Pick any frame of the run.",
                    )
                    .clicked()
                {
                    import_image_sequence(editor, state);
                }

                // Development builds only. It creates an asset pointing at a file that does
                // not exist, which is useful for laying out a timeline while working on the
                // editor and baffling to anyone else: the clip renders nothing and the
                // library lists it as missing. A release build is what people are shown.
                if cfg!(debug_assertions)
                    && ui
                        .button("Add placeholder clip")
                        .on_hover_text(
                            "Development builds only: adds a 5-second synthetic asset with no \
                             file behind it.",
                        )
                        .clicked()
                {
                    add_placeholder_clip(editor, state);
                }

            })
            .response
            .on_hover_text("Import a numbered image sequence");
            if ui
                .button("Import…")
                .on_hover_text("Add a file to the project's media library")
                .clicked()
            {
                import_media(editor, state);
            }
        });
    });
    ui.separator();

    if editor.project().media.is_empty() {
        ui.label(
            egui::RichText::new(
                "No media yet. Drag files onto the window, or press Import. Dropped on the timeline, they are added to it too.",
            )
            .color(theme::disabled()),
        );
        return;
    }

    // Finding one file among dozens: words typed match anywhere in the name,
    // and the kind narrows it further. Unused files can go in one step.
    let search = ui.add(
        egui::TextEdit::singleline(&mut state.media_search)
            .hint_text("Search files")
            .desired_width(f32::INFINITY),
    );
    if search.changed() {
        // Looking for something else: the mark has served its purpose.
        state.media_reveal = None;
    }
    ui.horizontal_wrapped(|ui| {
        for filter in crate::state::MediaFilter::ALL {
            if ui
                .selectable_label(state.media_kind == filter, filter.label())
                .clicked()
            {
                state.media_kind = filter;
            }
        }
        ui.separator();
        // The takes worth keeping: the first pass through a shoot is choosing,
        // not editing, and this is the half of it that shows the choice.
        let starred = state.media_stars > 0;
        if ui
            .selectable_label(starred, if starred { "Starred" } else { "All takes" })
            .on_hover_text("Show only the files with at least three stars")
            .clicked()
        {
            state.media_stars = if starred { 0 } else { 3 };
            state.needs_repaint = true;
        }
        // What is left to look at: the takes nothing in the edit uses yet.
        if ui
            .selectable_label(state.media_unused_only, "Not used yet")
            .on_hover_text("Show only the files no clip in the project uses")
            .clicked()
        {
            state.media_unused_only = !state.media_unused_only;
            state.needs_repaint = true;
        }
    });
    // How the files are ordered, and how much of each one is shown. Both are
    // one row: a long import is read by narrowing, then ordering, then
    // switching to the compact list — in that order, left to right.
    ui.horizontal_wrapped(|ui| {
        // How the files are ordered, one menu: the row of choices it
        // replaces was read once and then only in the way.
        ui.menu_button(format!("Sort: {}", state.media_sort.label()), |ui| {
            for sort in crate::state::MediaSort::ALL {
                if ui
                    .selectable_label(state.media_sort == sort, sort.label())
                    .clicked()
                {
                    // Pressing the sort already chosen turns it around, which is
                    // how every file list behaves.
                    if state.media_sort == sort {
                        state.media_sort_reversed = !state.media_sort_reversed;
                    } else {
                        state.media_sort = sort;
                        state.media_sort_reversed = false;
                    }
                    state.needs_repaint = true;
                }
            }
            if ui
                .button(if state.media_sort_reversed {
                    "Reversed"
                } else {
                    "In order"
                })
                .on_hover_text("Turn the order around")
                .clicked()
            {
                state.media_sort_reversed = !state.media_sort_reversed;
                state.needs_repaint = true;
            }
        })
        .response
        .on_hover_text("Order the files by when they were added, name, length or kind");
        ui.separator();
        let view = state.media_view;
        if ui
            .button(view.label())
            .on_hover_text("Switch between cards with thumbnails and a compact list")
            .clicked()
        {
            state.media_view = match view {
                crate::state::MediaView::Cards => crate::state::MediaView::List,
                crate::state::MediaView::List => crate::state::MediaView::Cards,
            };
            state.needs_repaint = true;
        }
    });

    // Bins, once there are any: every file, the unfiled ones, or one bin.
    let bins = editor.bins();
    if bins.is_empty() {
        state.media_bin = None;
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Bins").small().color(theme::disabled()));
            if ui
                .selectable_label(state.media_bin.is_none(), "All")
                .clicked()
            {
                state.media_bin = None;
            }
            for bin in &bins {
                let shown = state.media_bin.as_ref() == Some(&Some(bin.clone()));
                if ui.selectable_label(shown, bin).clicked() {
                    state.media_bin = Some(Some(bin.clone()));
                }
            }
            if ui
                .selectable_label(state.media_bin == Some(None), "Unfiled")
                .clicked()
            {
                state.media_bin = Some(None);
            }
        });
    }
    let unused = editor.unused_media().len();
    if unused > 0
        && ui
            .button(format!("Remove {unused} unused"))
            .on_hover_text(
                "Take every file no clip uses out of the project, in one step. \
                 The files themselves are not deleted.",
            )
            .clicked()
    {
        match editor.remove_unused_media() {
            Ok(removed) => {
                state.info(format!("Removed {removed} unused file(s) from the project"));
                state.needs_repaint = true;
            }
            Err(err) => state.error(err.to_string()),
        }
    }
    let total = editor.project().media.iter().filter(|m| !m.baked).count();
    let unused_ids = if state.media_unused_only {
        editor.unused_media()
    } else {
        Vec::new()
    };

    // Collect first so the list can be drawn while dispatching commands below.
    let mut assets: Vec<(MediaId, String, bool, bool, MediaTime, bool, MediaKind)> = editor
        .project()
        .media
        .iter()
        .filter(|m| {
            // A baked stretch of the edit is media the program wrote, not a
            // file the user brought in (`editor_core::render_in_place`).
            !m.baked
                && state.media_kind.accepts(m.kind)
                && m.rating >= state.media_stars
                && (!state.media_unused_only || unused_ids.contains(&m.id))
                && state
                    .media_bin
                    .as_ref()
                    .is_none_or(|wanted| &m.bin == wanted)
                && (crate::state::media_name_matches(m.display_name(), &state.media_search)
                    || crate::state::media_name_matches(&m.file_name, &state.media_search))
        })
        .map(|m| {
            (
                m.id,
                m.display_name().to_owned(),
                m.missing,
                // A photo has no duration and needs none.
                !m.is_still() && m.duration.is_zero(),
                m.duration,
                m.is_still(),
                m.kind,
            )
        })
        .collect();

    // Sorting is over the filtered list, so what is on screen is what is
    // ordered. "Added" is the order they arrived in, which is what the library
    // already held — so the default costs nothing.
    match state.media_sort {
        crate::state::MediaSort::Added => {}
        crate::state::MediaSort::Name => {
            assets.sort_by_key(|a| a.1.to_lowercase());
        }
        crate::state::MediaSort::Duration => assets.sort_by_key(|a| a.4.ticks()),
        crate::state::MediaSort::Kind => assets.sort_by_key(|a| match a.6 {
            MediaKind::Video => 0,
            MediaKind::Audio => 1,
            MediaKind::Image => 2,
        }),
    }
    if state.media_sort_reversed {
        assets.reverse();
    }

    // §33's auto slideshow, offered only when there is something to make one
    // from. A button that is always there and usually refuses teaches people to
    // ignore it.
    let photos: Vec<MediaId> = assets
        .iter()
        .filter(|(_, _, missing, _, _, still, _)| *still && !*missing)
        .map(|(id, ..)| *id)
        .collect();
    if photos.len() >= 2 {
        slideshow_menu(ui, editor, state, &photos);
        ui.separator();
    }

    let mut place: Option<MediaId> = None;
    // A file to put at the playhead, and how (`editor_core::three_point`).
    let mut drop_at: Option<(MediaId, bettercut_editor_core::three_point::DropKind)> = None;
    let mut relink: Option<MediaId> = None;
    let mut reveal: Option<MediaId> = None;
    let mut show_uses: Option<MediaId> = None;
    let mut deinterlace: Option<(MediaId, bool)> = None;
    let mut live_video: Option<std::path::PathBuf> = None;
    let mut remove: Option<MediaId> = None;
    let mut rename: Option<(MediaId, String)> = None;
    let mut refile: Option<(MediaId, String)> = None;
    let mut replace_all: Option<(MediaId, MediaId)> = None;
    // Every file by name, for "use another file everywhere this one is".
    let every_file: Vec<(MediaId, String)> = editor
        .project()
        .media
        .iter()
        .map(|m| (m.id, m.display_name().to_owned()))
        .collect();
    let mut renaming_open = false;

    // One banner rather than one button per broken asset: media usually moves a
    // folder at a time, so the folder scan is the action that actually fixes
    // the project (§66 "locate folder").
    let missing_count = assets.iter().filter(|a| a.2).count();
    if missing_count > 0 {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(format!("{missing_count} file(s) missing"))
                    .color(theme::error_text()),
            );
            if ui
                .button("Locate folder…")
                .on_hover_text(
                    "Pick the folder they moved to. Files are matched by name and \
                     confirmed by size, so a different file of the same name is \
                     left alone.",
                )
                .clicked()
            {
                relink_folder(editor, state);
            }
            if ui
                .button("Re-check")
                .on_hover_text("Look again — useful after reconnecting a drive")
                .clicked()
            {
                let still = editor.refresh_missing_media();
                state.info(if still == 0 {
                    "All media found".to_owned()
                } else {
                    format!("{still} file(s) still missing")
                });
            }
        });
        ui.separator();
    }

    // Nothing imported at all: say what to do. A blank panel is the one
    // screen a person cannot argue with, and this is the first one they see.
    if total == 0 {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("No files yet")
                .strong()
                .color(theme::disabled()),
        );
        ui.label(
            egui::RichText::new(
                "Drop a video, a photo or a song onto the window, or press Import… above. \
                 Then drag it onto the timeline, or press + beside it.",
            )
            .small()
            .color(theme::disabled()),
        );
        return;
    }

    if assets.len() < total {
        ui.label(
            egui::RichText::new(if assets.is_empty() {
                "No file matches".to_owned()
            } else {
                format!("{} of {total} files", assets.len())
            })
            .small()
            .color(theme::disabled()),
        );
    }

    if state.media_view == crate::state::MediaView::List {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (id, name, missing, no_duration, duration, still, kind) in &assets {
                let row = ui.horizontal(|ui| {
                    let can_place = !*no_duration && !*missing;
                    if ui
                        .add_enabled(can_place, egui::Button::new("+").small())
                        .on_hover_text("Add to timeline")
                        .on_disabled_hover_text(
                            "This file has no duration to place, or it is missing from disk.",
                        )
                        .clicked()
                    {
                        place = Some(*id);
                    }
                    ui.label(
                        egui::RichText::new(match kind {
                            MediaKind::Video => "Video",
                            MediaKind::Audio => "Sound",
                            MediaKind::Image => "Photo",
                        })
                        .small()
                        .color(theme::disabled()),
                    );
                    if *missing {
                        ui.label(egui::RichText::new("missing").color(theme::error_text()));
                    }
                    let length = if *still || *no_duration {
                        String::new()
                    } else {
                        TimelineTime::from_ticks(duration.ticks()).format_timecode()
                    };
                    if !length.is_empty() {
                        ui.label(
                            egui::RichText::new(length)
                                .small()
                                .monospace()
                                .color(theme::disabled()),
                        );
                    }
                    ui.label(name)
                        .on_hover_text("Switch to Cards for the thumbnail, renaming and the rest");
                });
                mark_revealed(ui, state, *id, row.response.rect);
            }
        });
        if let Some((id, kind)) = drop_at {
            drop_at_playhead(editor, state, id, kind);
        }
        if let Some(id) = place {
            place_on_timeline(editor, state, id);
        }
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, name, missing, no_duration, duration, still, _) in &assets {
            let card = ui.group(|ui| {
                match editor.project().media_asset(*id).and_then(|a| a.generated) {
                    Some(colour) => colour_swatch(ui, colour),
                    None => thumbnail(ui, state, *id, *missing, *duration),
                }

                ui.horizontal(|ui| {
                    let about = editor
                        .project()
                        .media_asset(*id)
                        .map(crate::file_details::details)
                        .unwrap_or_default();
                    let label = ui
                        .label(egui::RichText::new(name).strong())
                        .on_hover_text(format!(
                            "{about}\n\nRight-click to rename it in the project"
                        ));
                    // A name of its own, typed in a menu and applied when the
                    // menu closes, as a track's is.
                    label.context_menu(|ui| {
                        renaming_open = true;
                        let mut draft = match &state.media_name_draft {
                            Some((draft_id, text)) if draft_id == id => text.clone(),
                            _ => name.clone(),
                        };
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut draft)
                                .desired_width(180.0)
                                .char_limit(Editor::MAX_MEDIA_NAME)
                                .hint_text("name in this project"),
                        );
                        if field.changed() {
                            state.media_name_draft = Some((*id, draft.clone()));
                        }
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            rename = Some((*id, draft));
                            ui.close();
                        }
                        if ui
                            .button("Use the file name")
                            .on_hover_text("Forget the given name")
                            .clicked()
                        {
                            rename = Some((*id, String::new()));
                            ui.close();
                        }
                        ui.separator();
                        ui.menu_button("Move to Bin", |ui| {
                            for bin in &bins {
                                if ui.button(bin).clicked() {
                                    refile = Some((*id, bin.clone()));
                                    ui.close();
                                }
                            }
                            ui.horizontal(|ui| {
                                let field = ui.add(
                                    egui::TextEdit::singleline(&mut state.new_bin_draft)
                                        .desired_width(120.0)
                                        .char_limit(Editor::MAX_BIN_NAME)
                                        .hint_text("new bin"),
                                );
                                let enter = field.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if (ui.button("Add").clicked() || enter)
                                    && !state.new_bin_draft.trim().is_empty()
                                {
                                    refile = Some((*id, std::mem::take(&mut state.new_bin_draft)));
                                    ui.close();
                                }
                            });
                            if ui.button("Take out of its bin").clicked() {
                                refile = Some((*id, String::new()));
                                ui.close();
                            }
                        });
                        if editor.media_is_used(*id)
                            && ui
                                .button("Show in Timeline")
                                .on_hover_text("Select every clip that uses this file")
                                .clicked()
                        {
                            show_uses = Some(*id);
                            ui.close();
                        }
                        if editor.media_is_used(*id) {
                            ui.menu_button("Replace Everywhere With", |ui| {
                                for (other, other_name) in &every_file {
                                    if other != id && ui.button(other_name).clicked() {
                                        replace_all = Some((*id, *other));
                                        ui.close();
                                    }
                                }
                            });
                        }
                    });
                    if *missing {
                        ui.label(egui::RichText::new("missing").color(theme::error_text()));
                    }
                });

                stars_row(ui, editor, state, *id);

                let live = editor
                    .project()
                    .media_asset(*id)
                    .and_then(|a| a.live_video.clone());
                if *still {
                    ui.label(
                        egui::RichText::new(if live.is_some() { "Live Photo" } else { "Photo" })
                            .small()
                            .color(theme::disabled()),
                    );
                } else if *no_duration {
                    // A file whose container never declared a duration.
                    ui.label(
                        egui::RichText::new("no duration")
                            .small()
                            .color(theme::disabled()),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(
                            TimelineTime::from_ticks(duration.ticks()).format_timecode(),
                        )
                        .small()
                        .monospace()
                        .color(theme::disabled()),
                    );
                }

                // §66: a missing file needs a way back, right where the problem
                // is visible. Without it the project is simply broken.
                if *missing && ui.button("Locate…").clicked() {
                    relink = Some(*id);
                }
                // Interlaced originals: a checkbox on the file, because the
                // combing is the file's, not any one clip's.
                if !*still
                    && let Some(asset) = editor.project().media_asset(*id)
                {
                    let mut woven = asset.deinterlace;
                    if ui
                        .checkbox(&mut woven, "Deinterlace")
                        .on_hover_text(
                            "For camcorder and broadcast tapes whose frames show combed lines on movement: weave the two fields into one frame as it is read. Undoable.",
                        )
                        .changed()
                    {
                        deinterlace = Some((*id, woven));
                    }
                }

                ui.horizontal(|ui| {
                    let can_place = !*no_duration && !*missing;
                    if ui
                        .add_enabled(can_place, egui::Button::new("Add to timeline"))
                        .on_disabled_hover_text(
                            "This file has no duration to place, or it is missing \
                             from disk.",
                        )
                        .clicked()
                    {
                        place = Some(*id);
                    }

                    // One action on show, the one most reached for; the rest
                    // wait behind More so a long list of cards stays quiet.
                    ui.menu_button("More", |ui| {
                        // A Live Photo's moment, as video: the file that came
                        // with it, imported beside the photo.
                        if let Some(video) = &live
                            && ui
                                .button("Use the Live Photo Video")
                                .on_hover_text("Import the few seconds of video taken with this photo")
                                .clicked()
                        {
                            live_video = Some(video.clone());
                        }
                        // Three-point editing: the marked part, at the playhead,
                        // either replacing what is there or pushing it along
                        // (`editor_core::three_point`).
                        use bettercut_editor_core::three_point::DropKind;
                        if ui
                            .add_enabled(can_place, egui::Button::new("Overwrite"))
                            .on_hover_text(
                                "Put it at the playhead over whatever is there; nothing else moves",
                            )
                            .clicked()
                        {
                            drop_at = Some((*id, DropKind::Overwrite));
                        }
                        if ui
                            .add_enabled(can_place, egui::Button::new("Insert"))
                            .on_hover_text(
                                "Put it at the playhead and push everything from there along",
                            )
                            .clicked()
                        {
                            drop_at = Some((*id, DropKind::Insert));
                        }

                        if !*missing
                            && ui
                                .button("Show in Folder")
                                .on_hover_text(
                                    "Open the folder this file is in, with the file selected",
                                )
                                .clicked()
                        {
                            reveal = Some(*id);
                        }
                        ui.separator();

                        // Removing an asset a clip still uses would leave cuts
                        // pointing at nothing, so it is refused — and the button
                        // says why rather than failing after the click. §2 also
                        // applies, and the hover text says so: this takes the file
                        // out of the *project*, never off the disk.
                        let in_use = editor.media_is_used(*id);
                        if ui
                            .add_enabled(!in_use, egui::Button::new("Remove"))
                            .on_hover_text(
                                "Take this file out of the project. The file itself is \
                                 not deleted.",
                            )
                            .on_disabled_hover_text(
                                "A clip on the timeline uses this file. Delete those \
                                 clips first.",
                            )
                            .clicked()
                        {
                            remove = Some(*id);
                        }
                    });
                });
            });
            mark_revealed(ui, state, *id, card.response.rect);
        }
    });

    // A typed name is applied once its menu has closed, however it closed.
    if rename.is_none()
        && !renaming_open
        && let Some((id, text)) = state.media_name_draft.take()
    {
        rename = Some((id, text));
    }
    if let Some((id, text)) = rename {
        state.media_name_draft = None;
        match editor.rename_media(id, &text) {
            Ok(true) => state.needs_repaint = true,
            Ok(false) => {}
            Err(err) => state.error(err.to_string()),
        }
    }

    if let Some((old, new)) = replace_all {
        match editor.replace_media_everywhere(old, new) {
            Ok((0, 0)) => state.info("Nothing uses that file in this sequence"),
            Ok((changed, 0)) => state.info(format!("Replaced in {changed} clip(s)")),
            Ok((changed, refused)) => state.info(format!(
                "Replaced in {changed} clip(s); {refused} could not take the new file (too short, or missing a picture or sound)"
            )),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }

    if let Some((id, bin)) = refile {
        match editor.set_media_bin(id, &bin) {
            Ok(true) => state.needs_repaint = true,
            Ok(false) => {}
            Err(err) => state.error(err.to_string()),
        }
    }

    if let Some(id) = place {
        place_on_timeline(editor, state, id);
    }
    if let Some(id) = show_uses {
        let uses = editor.clips_using(id);
        if let Some((_, first)) = uses.first() {
            let lanes = (ui.ctx().content_rect().width() - theme::TRACK_HEADER_WIDTH).max(200.0);
            state.scroll_to_reveal(*first, lanes);
        }
        state.clear_selection();
        state
            .selected_clips
            .extend(uses.iter().map(|(clip, _)| *clip));
        state.info(match uses.len() {
            0 => "No clip uses this file".to_owned(),
            1 => "1 clip uses this file — selected".to_owned(),
            n => format!("{n} clips use this file — all selected"),
        });
        state.needs_repaint = true;
    }
    if let Some(video) = live_video {
        import_paths(editor, state, &[video]);
    }
    if let Some(id) = reveal
        && let Some(path) = editor.project().media_asset(id).map(|a| a.path.clone())
        && let Err(err) = crate::reveal::show_in_folder(&path)
    {
        state.error(err);
    }
    if let Some((id, woven)) = deinterlace {
        match editor.set_media_deinterlace(id, woven) {
            Ok(true) => {
                state.media_reopen.push(id);
                state.needs_repaint = true;
            }
            Ok(false) => {}
            Err(err) => state.error(err.to_string()),
        }
    }
    if let Some(id) = relink {
        relink_one(editor, state, id);
    }
    if let Some(id) = remove {
        let name = editor
            .project()
            .media_asset(id)
            .map_or_else(|| "the file".to_owned(), |m| m.file_name.clone());
        match editor.remove_media(id) {
            Ok(()) => {
                state.info(format!("Removed {name} from the project"));
                state.needs_repaint = true;
            }
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// §66 "locate file": point one asset at a file the user picks.
fn relink_one(editor: &mut Editor, state: &mut UiState, media: MediaId) {
    let name = editor
        .project()
        .media_asset(media)
        .map(|m| m.file_name.clone())
        .unwrap_or_default();

    let Some(path) = rfd::FileDialog::new()
        .set_title(format!("Locate {name}"))
        .pick_file()
    else {
        return;
    };

    // Warn rather than refuse. The user explicitly chose this file, and they
    // may know something we do not — a re-encode, a trimmed master. §66's size
    // check exists to stop *automatic* matching, not to overrule a person.
    let mismatch = editor
        .project()
        .media_asset(media)
        .is_some_and(|m| !m.matches_relink_candidate(&path));

    match editor.relink_media(media, &path) {
        Ok(()) => {
            state.needs_repaint = true;
            if mismatch {
                state.error(format!(
                    "Relinked to {} — it does not match the original's name and size",
                    path.display()
                ));
            } else {
                state.info(format!("Relinked {name}"));
            }
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// §66 "locate folder": relink everything that moved together.
fn relink_folder(editor: &mut Editor, state: &mut UiState) {
    let Some(folder) = rfd::FileDialog::new()
        .set_title("Locate the folder your media moved to")
        .pick_folder()
    else {
        return;
    };

    match editor.relink_from_folder(&folder) {
        Ok(0) => state.error(format!(
            "Nothing in {} matched the missing files by name and size",
            folder.display()
        )),
        Ok(n) => {
            state.needs_repaint = true;
            state.info(format!("Relinked {n} file(s)"));
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Preview (§58, §4.1).
///
/// The compositor's output texture is painted straight into this rect. §4.1's
/// whole argument for egui + wgpu is that this needs no copy and no overlay
/// window — and that panels may overlap it, which a native child window could
/// never allow.
/// The picture on the whole screen, with the playhead's time and how to get
/// back shown small in a corner while the pointer moves.
pub fn fullscreen_preview(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    preview: Option<&crate::Preview>,
) {
    let full = ui.max_rect();
    let Some(sequence) = editor.active_sequence() else {
        return;
    };
    let aspect = sequence.resolution.aspect_ratio().max(0.01);
    // The largest box of the sequence's shape that fits the screen.
    let size = if full.width() / full.height() > aspect {
        egui::vec2(full.height() * aspect, full.height())
    } else {
        egui::vec2(full.width(), full.width() / aspect)
    };
    let rect = egui::Rect::from_center_size(full.center(), size);
    let response = ui.allocate_rect(full, egui::Sense::click());
    if let Some(preview) = preview.filter(|p| p.has_content()) {
        ui.painter().image(
            preview.texture_id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    if response.double_clicked() {
        state.fullscreen = false;
    }
    // A moving pointer brings the hint back; still, the picture is clean.
    let moving = ui.input(|i| i.pointer.delta().length() > 0.0 || i.pointer.any_down());
    let shown_id = ui.id().with("fullscreen hint");
    let now = ui.input(|i| i.time);
    if moving {
        ui.data_mut(|d| d.insert_temp(shown_id, now));
    }
    let last: f64 = ui.data(|d| d.get_temp(shown_id)).unwrap_or(now);
    if now - last < 2.5 {
        ui.painter().text(
            full.left_bottom() + egui::vec2(16.0, -16.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "{}   ·   Space play   ·   Esc or F to leave full screen",
                editor.display_time(editor.playhead()).format_timecode()
            ),
            egui::FontId::proportional(14.0),
            egui::Color32::from_white_alpha(200),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
    }
}

pub fn preview(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    preview: Option<&crate::Preview>,
) {
    let available = ui.available_size();
    // Click-and-drag, not hover: the picture is a control now. Framing a shot
    // is done by looking at it, so the handles belong on it rather than only in
    // the Inspector's number fields.
    let (rect, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0, theme::background());

    let Some(sequence) = editor.active_sequence() else {
        return;
    };
    let output_aspect = sequence.resolution.aspect_ratio().max(0.01);
    let resolution = (sequence.resolution.width, sequence.resolution.height);

    // Ctrl + scroll steps the zoom; the middle button pans a zoomed picture.
    if response.hovered() {
        // One step per wheel notch, read from the raw events: the smoothed
        // scroll would step several times for one turn of the wheel.
        let notches: Vec<bool> = ui.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel {
                        delta, modifiers, ..
                    } if modifiers.command && delta.y != 0.0 => Some(delta.y > 0.0),
                    _ => None,
                })
                .collect()
        });
        for inwards in notches {
            state.preview_zoom = state.preview_zoom.step(inwards);
            state.needs_repaint = true;
        }
    }
    if response.dragged_by(egui::PointerButton::Middle)
        && state.preview_zoom != crate::state::PreviewZoom::Fit
    {
        state.preview_pan += response.drag_delta();
        state.needs_repaint = true;
    }

    // Where the picture goes: letterboxed to fit, or at a set scale and pan.
    // Everything drawn over it — guides, handles, the eyedropper — works from
    // this one rect, so they all follow the zoom.
    let (canvas, pan) = crate::preview_overlay::preview_canvas(
        rect,
        resolution,
        state.preview_zoom,
        state.preview_pan,
        1.0 / ui.ctx().pixels_per_point(),
    );
    state.preview_pan = pan;

    painter.rect_filled(canvas, 4, egui::Color32::BLACK);
    painter.rect_stroke(
        canvas,
        4,
        egui::Stroke::new(1.0, theme::grid_line()),
        egui::StrokeKind::Outside,
    );

    let has_content = preview.is_some_and(crate::Preview::has_content);

    // Dragging the divider of a side-by-side comparison. Before the picture is
    // drawn, so the line lands where the pointer left it rather than a frame
    // behind.
    if state.compare_split
        && response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        state.compare_split_at =
            ((pos.x - canvas.left()) / canvas.width().max(1.0)).clamp(0.05, 0.95);
        state.needs_repaint = true;
    }

    // The composited frame, painted directly. One line, no copy (§4.1).
    if let Some(preview) = preview
        && has_content
    {
        let split = state
            .compare_split
            .then(|| preview.snapshot_id())
            .flatten()
            .map(|id| (id, state.compare_split_at.clamp(0.05, 0.95)));
        match split {
            None => {
                painter.image(
                    preview.texture_id(),
                    canvas,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            // Graded to the left of the divider, the original to the right,
            // each drawn from its own part of its own texture so the two
            // halves line up exactly.
            Some((original, at)) => {
                let edge = canvas.left() + canvas.width() * at;
                painter.image(
                    preview.texture_id(),
                    egui::Rect::from_min_max(canvas.min, egui::pos2(edge, canvas.bottom())),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(at, 1.0)),
                    egui::Color32::WHITE,
                );
                painter.image(
                    original,
                    egui::Rect::from_min_max(egui::pos2(edge, canvas.top()), canvas.max),
                    egui::Rect::from_min_max(egui::pos2(at, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                painter.line_segment(
                    [
                        egui::pos2(edge, canvas.top()),
                        egui::pos2(edge, canvas.bottom()),
                    ],
                    egui::Stroke::new(2.0, theme::selection()),
                );
                painter.text(
                    egui::pos2(edge - 6.0, canvas.top() + 4.0),
                    egui::Align2::RIGHT_TOP,
                    "after",
                    egui::FontId::proportional(12.0),
                    theme::clip_text(),
                );
                painter.text(
                    egui::pos2(edge + 6.0, canvas.top() + 4.0),
                    egui::Align2::LEFT_TOP,
                    "before",
                    egui::FontId::proportional(12.0),
                    theme::clip_text(),
                );
            }
        }
    } else {
        painter.text(
            canvas.center(),
            egui::Align2::CENTER_CENTER,
            format!(
                "{}×{} · {} fps",
                sequence.resolution.width, sequence.resolution.height, sequence.frame_rate
            ),
            egui::FontId::proportional(14.0),
            theme::disabled(),
        );
        painter.text(
            egui::Pos2::new(canvas.center().x, canvas.center().y + 22.0),
            egui::Align2::CENTER_CENTER,
            "Move the playhead over a clip to see it here",
            egui::FontId::proportional(11.0),
            theme::disabled(),
        );
    }

    // Guides over the picture, under the handles: they help place things, and
    // must never get in the way of grabbing one.
    crate::preview_overlay::draw_guides(&painter, state.preview_guide, canvas);
    if state.preview_timecode {
        let text = editor.display_time(editor.playhead()).format_timecode();
        let galley =
            painter.layout_no_wrap(text, egui::FontId::monospace(14.0), egui::Color32::WHITE);
        let at = canvas.left_bottom() + egui::vec2(8.0, -8.0 - galley.size().y);
        let back = egui::Rect::from_min_size(at, galley.size()).expand(4.0);
        painter.rect_filled(back, 3, egui::Color32::from_black_alpha(170));
        painter.galley(at, galley, egui::Color32::WHITE);
    }

    // The eyedropper, before the handles: while it is armed the picture is
    // a colour chart, not a thing to drag, and letting a transform handle take
    // the click first would move the clip instead of keying it.
    if let Some(clip) = state.picking_key {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        painter.rect_stroke(
            canvas,
            4,
            egui::Stroke::new(2.0, theme::selection()),
            egui::StrokeKind::Outside,
        );
        if response.clicked() {
            pick_key_colour(editor, state, preview, &response, canvas, clip);
        }
        return;
    }

    transform_handles(
        ui,
        &painter,
        editor,
        state,
        preview,
        &response,
        canvas,
        output_aspect,
    );
}

/// Take the colour under the pointer and make it `clip`'s green screen.
///
/// Keeps whatever tolerance and softness were already dialled in: the usual
/// reason to re-pick is that the first point was slightly off, and resetting
/// the other two would throw away the part that was working.
fn pick_key_colour(
    editor: &mut Editor,
    state: &mut UiState,
    preview: Option<&crate::Preview>,
    response: &egui::Response,
    canvas: egui::Rect,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    state.picking_key = None;
    state.needs_repaint = true;

    let Some(resolution) = editor.active_sequence().map(|s| s.resolution) else {
        return;
    };
    let Some(at) = response.interact_pointer_pos() else {
        return;
    };
    let Some((x, y)) = crate::preview_overlay::frame_pixel_at(at, canvas, resolution) else {
        state.error("That is outside the picture — point at the screen itself");
        return;
    };
    let Some(colour) = preview.and_then(|preview| preview.read_pixel(x, y)) else {
        state.error("Could not read that pixel");
        return;
    };

    let existing = editor.video_clip(clip).and_then(|clip| clip.chroma_key);
    let key = bettercut_editor_core::timeline::ChromaKey {
        color: colour,
        ..existing.unwrap_or_default()
    };
    let property = bettercut_editor_core::ClipProperty::ChromaKey(Some(key));
    if let Err(err) = editor.set_clip_value(clip, property, false) {
        state.error(err.to_string());
    }
}

/// The move-and-scale box over the selected clip.
///
/// Drawn and driven here, but every decision it makes lives in
/// [`crate::preview_overlay`], where it can be tested without a window.
#[allow(clippy::too_many_arguments)]
fn transform_handles(
    ui: &egui::Ui,
    painter: &egui::Painter,
    editor: &mut Editor,
    state: &mut UiState,
    preview: Option<&crate::Preview>,
    response: &egui::Response,
    canvas: egui::Rect,
    output_aspect: f32,
) {
    use crate::preview_overlay as overlay;

    // Everything visible right now, topmost first. §22 makes track order the
    // compositing order with index 0 at the bottom, so reversing puts the clip
    // a click would actually hit first in the list.
    let visible = visible_boxes(
        editor,
        &|clip| preview.and_then(|p| p.source_size(clip)),
        canvas,
        output_aspect,
    );
    if visible.is_empty() {
        state.preview_drag = None;
        return;
    }

    // The handles belong to a single selected clip: two selections give two
    // boxes and no single answer to a drag.
    let selected = (state.selected_clips.len() == 1)
        .then(|| state.selected_clips.iter().copied().next())
        .flatten()
        .and_then(|id| visible.iter().find(|shown| shown.clip == id).copied());

    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let on_corner = selected.zip(pointer).and_then(|(shown, at)| {
        overlay::corner_at(shown.box_on_canvas, shown.transform.rotation_degrees, at)
    });
    let on_rotate = selected.zip(pointer).is_some_and(|(shown, at)| {
        overlay::on_rotate_handle(shown.box_on_canvas, shown.transform.rotation_degrees, at)
    });

    // §45's corner pin, for the selected clip while pin mode is on. Like crop
    // mode, it takes the corners over: a drag meant to pin that scaled instead
    // would be a nasty surprise.
    let pin_mode = selected
        .filter(|_| state.corner_pin_mode)
        .and_then(|shown| {
            editor
                .video_clip(shown.clip)
                .map(|clip| (shown, clip.corner_pin))
        });
    let on_pinned_corner = pin_mode.zip(pointer).and_then(|((shown, pin), at)| {
        overlay::pinned_corner_at(shown.box_on_canvas, canvas, pin, at)
    });

    // Crop mode, for the clip it was armed on and only while that clip is the
    // one selected (see `UiState::cropping`). While it is on, the crop edges
    // are the only handles: the corners and the rotate handle sit where a crop
    // edge's hand would go, and a drag meant to crop that scaled instead would
    // be a nasty surprise.
    let crop_mode = selected
        .filter(|shown| state.cropping == Some(shown.clip))
        .and_then(|shown| editor.video_clip(shown.clip).map(|clip| (shown, clip.crop)));
    let on_crop_edge = crop_mode.zip(pointer).and_then(|((shown, _), at)| {
        overlay::crop_edge_at(shown.box_on_canvas, shown.transform.rotation_degrees, at)
    });

    // The cursor says what is under it before it is pressed.
    if let Some(edge) = on_crop_edge {
        ui.ctx().set_cursor_icon(match edge {
            overlay::CropEdge::Left | overlay::CropEdge::Right => {
                egui::CursorIcon::ResizeHorizontal
            }
            overlay::CropEdge::Top | overlay::CropEdge::Bottom => egui::CursorIcon::ResizeVertical,
        });
    } else if crop_mode.is_some() {
        // Nothing else is live in crop mode, so nothing else is offered.
    } else if on_rotate {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    } else if on_corner.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    } else if pointer.is_some_and(|at| {
        visible
            .iter()
            .any(|s| overlay::contains(s.box_on_canvas, s.transform.rotation_degrees, at))
    }) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // A click picks a clip, so the picture is a place to select from and not
    // only a place to drag what the timeline already chose. Clicking the empty
    // canvas around it clears the selection, which is the only other thing that
    // click could reasonably mean.
    if response.clicked()
        && let Some(at) = pointer
    {
        match topmost_at(&visible, at) {
            Some(shown) => state.select_only(shown.clip),
            None => state.clear_selection(),
        }
    }

    // Decide the gesture on the **press**, not on `drag_started`.
    //
    // egui only calls a movement a drag once it has passed a threshold, and by
    // the time it says so the pointer has already left the handle: `corner_at`
    // then found nothing and every corner drag silently became a move. That is
    // the whole of "only one corner resizes".
    //
    // `is_pointer_button_down_on` is true from the press until release, so the
    // frame where it is true and nothing is recorded yet is the press itself —
    // and on that frame the pointer is exactly where the user put it.
    if response.is_pointer_button_down_on()
        && state.preview_drag.is_none()
        && let Some(at) = pointer
    {
        // A mask's handle sits inside the box, so it is checked before anything
        // that would otherwise claim the press and move the whole clip.
        // A key of the motion path: the smallest target on the canvas, so it
        // is checked first — under it there is always the picture, which
        // would otherwise take the press and move the whole clip.
        // The pivot handle, first of all: smaller than anything else here.
        let on_pivot = selected.and_then(|shown| {
            editor.video_clip(shown.clip)?;
            overlay::on_pivot(shown.pivot, at).then_some((shown, overlay::Gesture::Pivot))
        });
        let on_path = selected.and_then(|shown| {
            let clip = editor.video_clip(shown.clip)?;
            let keys = crate::motion_path::path_keys(clip);
            let index = crate::motion_path::key_at(&keys, canvas, at)?;
            Some((
                shown,
                overlay::Gesture::PathKey {
                    time: keys[index].time,
                    from: keys[index].position,
                    grab: at,
                },
            ))
        });
        let on_mask = selected.and_then(|shown| {
            let mask = editor.video_clip(shown.clip)?.mask?;
            let degrees = shown.transform.rotation_degrees;

            // The size handle first: it sits on the shape's edge, and on a
            // small mask the two handles can come within reach of each other.
            // Resizing is the more specific intent, so it wins the press.
            if let Some(edge) = overlay::mask_size_handle(mask, shown.box_on_canvas, degrees)
                && overlay::on_mask_handle(edge, at)
            {
                return Some((
                    shown,
                    overlay::Gesture::MaskResize {
                        from: mask.size,
                        grab: at,
                    },
                ));
            }

            let centre = overlay::clip_point_on_canvas(mask.center, shown.box_on_canvas, degrees);
            overlay::on_mask_handle(centre, at).then_some((
                shown,
                overlay::Gesture::MaskMove {
                    from: mask.center,
                    grab: at,
                },
            ))
        });

        let grabbed = match (selected, on_corner) {
            // In crop mode a press takes a crop edge or nothing. Captured
            // whole, for the reason `Gesture::Crop` gives.
            _ if crop_mode.is_some() => crop_mode.zip(on_crop_edge).map(|((shown, crop), edge)| {
                (
                    shown,
                    overlay::Gesture::Crop {
                        edge,
                        from: crop,
                        picture: shown.box_on_canvas,
                        degrees: shown.transform.rotation_degrees,
                    },
                )
            }),
            // A pinned corner, while pin mode is on: it sits where the scale
            // handle would, and pin mode is what says which one the press
            // means.
            _ if on_pinned_corner.is_some() => {
                pin_mode
                    .zip(on_pinned_corner)
                    .map(|((shown, pin), corner)| {
                        let index = match corner {
                            overlay::Corner::TopLeft => 0,
                            overlay::Corner::TopRight => 1,
                            overlay::Corner::BottomRight => 2,
                            overlay::Corner::BottomLeft => 3,
                        };
                        // Where that corner would sit with no pin at all: what
                        // the drag's offset is measured from.
                        let unpinned = overlay::pinned_corners(
                            shown.box_on_canvas,
                            canvas,
                            bettercut_editor_core::timeline::CornerPin::NONE,
                        )[index];
                        (
                            shown,
                            overlay::Gesture::Pin {
                                corner,
                                from: pin,
                                origin: unpinned,
                            },
                        )
                    })
            }
            _ if on_pivot.is_some() => on_pivot,
            _ if on_path.is_some() => on_path,
            _ if on_mask.is_some() => on_mask,
            // The rotate handle first: it sits outside the box, so nothing
            // else could claim that press, but checked ahead of the corners
            // in case a very small clip brings the two within reach.
            (Some(shown), _) if on_rotate => Some((
                shown,
                overlay::Gesture::Rotate {
                    from: shown.transform.rotation_degrees,
                    grab_angle: overlay::angle_of(shown.box_on_canvas.center(), at),
                },
            )),
            // A corner only counts for the clip that is already selected;
            // otherwise the corners of an unselected clip would be invisible
            // hotspots.
            (Some(shown), Some(corner)) => Some((
                shown,
                overlay::Gesture::Scale {
                    corner,
                    from: shown.transform.scale,
                    grab_distance: shown.box_on_canvas.center().distance(at),
                },
            )),
            _ => topmost_at(&visible, at).map(|shown| {
                (
                    shown,
                    overlay::Gesture::Move {
                        from: shown.transform.position,
                        grab: at,
                    },
                )
            }),
        };

        match grabbed {
            Some((shown, gesture)) => {
                state.select_only(shown.clip);
                state.preview_drag = Some(overlay::PreviewDrag {
                    clip: shown.clip,
                    gesture,
                    started: false,
                });
            }
            // Outside every picture: not a transform gesture.
            None => state.preview_drag = None,
        }
    }

    // Whatever is selected now — possibly picked a moment ago by the press
    // above — is what gets the box.
    let Some(shown) = (state.selected_clips.len() == 1)
        .then(|| state.selected_clips.iter().copied().next())
        .flatten()
        .and_then(|id| visible.iter().find(|shown| shown.clip == id).copied())
    else {
        return;
    };

    let dragging = state
        .preview_drag
        .is_some_and(|drag| drag.clip == shown.clip && response.dragged());

    let cropping_this = state.cropping == Some(shown.clip);
    if cropping_this && let Some(crop) = editor.video_clip(shown.clip).map(|clip| clip.crop) {
        overlay::draw_crop(
            painter,
            shown.box_on_canvas,
            crop,
            shown.transform.rotation_degrees,
            dragging,
        );
    } else if state.corner_pin_mode
        && let Some(pin) = editor.video_clip(shown.clip).map(|clip| clip.corner_pin)
    {
        overlay::draw_pin(
            painter,
            overlay::pinned_corners(shown.box_on_canvas, canvas, pin),
        );
    } else {
        overlay::draw(
            painter,
            shown.box_on_canvas,
            shown.transform.rotation_degrees,
            dragging,
        );
    }

    // A mask's centre and its edge, where it can be taken hold of — not in crop
    // mode, where they would be handles that do nothing.
    if editor.video_clip(shown.clip).is_some() {
        overlay::draw_pivot(painter, shown.pivot, dragging);
    }

    // The motion path, for a keyed position: the line the picture travels,
    // with a handle at every key (`crate::motion_path`).
    if let Some(clip) = editor.video_clip(shown.clip) {
        let keys = crate::motion_path::path_keys(clip);
        if !keys.is_empty() {
            let curve =
                crate::motion_path::path_curve(clip, &keys, crate::motion_path::STEPS_PER_SEGMENT);
            let now = clip.source_time_at(editor.playhead());
            let current = keys.iter().position(|key| key.time == now);
            crate::motion_path::draw(painter, &keys, &curve, canvas, current);
        }
    }

    if !cropping_this && let Some(mask) = editor.video_clip(shown.clip).and_then(|clip| clip.mask) {
        let degrees = shown.transform.rotation_degrees;
        overlay::draw_mask_handle(
            painter,
            overlay::clip_point_on_canvas(mask.center, shown.box_on_canvas, degrees),
            dragging,
        );
        if let Some(at) = overlay::mask_size_handle(mask, shown.box_on_canvas, degrees) {
            overlay::draw_mask_handle(painter, at, dragging);
        }
    }

    if dragging
        && let Some(at) = pointer
        && let Some(drag) = &mut state.preview_drag
    {
        // A mask drag is the one `property_for` cannot answer: moving a mask
        // means writing a whole `Mask` back, which needs the mask being moved.
        let continuing = drag.started;
        // Said after the drag's borrow ends, or the message and the drag would
        // both want the state at once.
        let mut path_error = None;
        let property = match drag.gesture {
            overlay::Gesture::Pivot => {
                let uv = overlay::canvas_point_in_clip(
                    at,
                    shown.box_on_canvas,
                    shown.transform.rotation_degrees,
                );
                let anchor = bettercut_editor_core::timeline::Vec2::new(uv[0], uv[1]);
                if let Err(err) = editor.set_pivot_keeping_place(shown.clip, anchor, continuing) {
                    path_error = Some(err.to_string());
                }
                None
            }
            overlay::Gesture::PathKey { time, from, grab } => {
                let moved = overlay::moved_position(from, grab, at, canvas);
                if let Err(err) = editor.move_position_key(shown.clip, time, moved, continuing) {
                    path_error = Some(err.to_string());
                }
                None
            }
            overlay::Gesture::MaskResize { from, grab } => editor
                .video_clip(shown.clip)
                .and_then(|clip| clip.mask)
                .map(|mask| {
                    let size = overlay::mask_resized(
                        from,
                        grab,
                        at,
                        mask.rotation_degrees,
                        shown.box_on_canvas,
                        shown.transform.rotation_degrees,
                    );
                    bettercut_editor_core::ClipProperty::Mask(Some(
                        bettercut_editor_core::timeline::Mask { size, ..mask },
                    ))
                }),
            overlay::Gesture::MaskMove { from, grab } => editor
                .video_clip(shown.clip)
                .and_then(|clip| clip.mask)
                .map(|mask| {
                    let centre = overlay::mask_moved(
                        from,
                        grab,
                        at,
                        shown.box_on_canvas,
                        shown.transform.rotation_degrees,
                    );
                    bettercut_editor_core::ClipProperty::Mask(Some(
                        bettercut_editor_core::timeline::Mask {
                            center: centre,
                            ..mask
                        },
                    ))
                }),
            // A move snaps to the middle and the edges, and shows the line
            // it caught; Alt moves freely.
            overlay::Gesture::Move { from, grab } => {
                let moved = overlay::moved_position(from, grab, at, canvas);
                let free = ui.input(|i| i.modifiers.alt);
                let position = if free {
                    moved
                } else {
                    let half = [
                        shown.box_on_canvas.width() / canvas.width().max(1.0) / 2.0,
                        shown.box_on_canvas.height() / canvas.height().max(1.0) / 2.0,
                    ];
                    let snapped = overlay::snap_position(moved, half, canvas);
                    overlay::draw_snap_guides(painter, snapped, canvas);
                    snapped.position
                };
                Some(bettercut_editor_core::ClipProperty::Position {
                    x: position.x,
                    y: position.y,
                })
            }
            gesture => overlay::property_for(gesture, canvas, shown.box_on_canvas, at),
        };
        drag.started = true;
        // §11: `continuing` after the first frame, so the whole drag is one
        // undo step — and §54, so this goes through a command rather than
        // touching the project.
        if let Some(property) = property
            && let Err(err) = editor.set_clip_value(shown.clip, property, continuing)
        {
            state.error(err.to_string());
        }
        state.needs_repaint = true;
        if let Some(err) = path_error {
            state.error(err);
        }
    }

    if response.drag_stopped() {
        state.preview_drag = None;
    }
}

/// A clip on screen right now, and where its picture is.
#[derive(Clone, Copy)]
struct ShownClip {
    clip: bettercut_editor_core::foundation::ClipId,
    /// Where the clip's pivot sits on the canvas: the point it turns about.
    pivot: egui::Pos2,
    /// The animated transform at this instant, so a keyed clip is grabbed where
    /// it actually appears rather than where its sliders read (§24).
    transform: bettercut_editor_core::timeline::Transform,
    box_on_canvas: egui::Rect,
}

/// Every visible layer at the playhead, topmost first.
///
/// `source_size` answers how big a layer's picture actually was in the last
/// composited frame, which only §26's titles need: how wide "Hello" comes out
/// is not something the model can answer, and the rasterizer is the only thing
/// that knows. Taken as a function rather than as the preview itself so the
/// title path can be exercised without a GPU.
fn visible_boxes(
    editor: &Editor,
    source_size: &dyn Fn(bettercut_editor_core::foundation::ClipId) -> Option<(u32, u32)>,
    canvas: egui::Rect,
    output_aspect: f32,
) -> Vec<ShownClip> {
    use crate::preview_overlay as overlay;

    let playhead = editor.playhead();
    let project = editor.project();
    let Some(sequence) = editor.active_sequence() else {
        return Vec::new();
    };

    let mut shown = Vec::new();

    // §26's titles composite over every video track, so they are first in a
    // list ordered topmost-first — a click lands on the title, not on the shot
    // behind it.
    for track in sequence.text_tracks.iter().rev() {
        if !track.enabled {
            continue;
        }
        let Some(clip) = track.clip_at(playhead) else {
            continue;
        };
        if clip.is_blank() || !clip.enabled {
            continue;
        }
        // No handles until it has been drawn once. There is nothing to draw
        // them around before that, and the next frame has the answer.
        let Some((width, height)) = source_size(clip.id) else {
            continue;
        };
        if height == 0 {
            continue;
        }

        // The transform kept on `ShownClip` is the clip's *own*, not the
        // composited one: it is what a gesture starts from and what the command
        // writes back. Seeded with the corrected scale instead, the first drag
        // of a corner would collapse the title to a fraction of itself.
        shown.push(ShownClip {
            clip: clip.id,
            pivot: egui::Pos2::ZERO,
            transform: clip.transform,
            box_on_canvas: overlay::to_canvas(
                overlay::generated_layer_box(
                    clip.transform,
                    width,
                    height,
                    sequence.resolution.width,
                    sequence.resolution.height,
                ),
                canvas,
            ),
        });
    }
    // Reversed: §22 puts index 0 at the bottom of the stack, and a click should
    // find what is drawn over everything else.
    for track in sequence.video_tracks.iter().rev() {
        if !track.enabled {
            continue;
        }
        let Some(clip) = track.clip_at(playhead) else {
            continue;
        };
        let look = clip.look_at(clip.source_time_at(playhead));
        let transform = look.transform;
        // The shape the renderer fits — after §22's crop, not before it. Asked
        // of the same `Crop::applied_to` the renderer uses, so the handles land
        // on the picture as drawn rather than on the shape it was before
        // cropping.
        let source_aspect = project
            .media_asset(clip.media_id)
            .filter(|asset| asset.height > 0)
            .map_or(output_aspect, |asset| {
                asset.width as f32 / asset.height as f32
            });

        let pivot = crate::motion_path::on_canvas(transform.position, canvas);
        let box_on_canvas = overlay::to_canvas(
            overlay::clip_box(source_aspect, output_aspect, look.crop, transform),
            canvas,
        );
        shown.push(ShownClip {
            clip: clip.id,
            pivot,
            transform,
            box_on_canvas: overlay::pivot_shifted(box_on_canvas, pivot, transform.rotation_degrees),
        });
    }
    shown
}

/// The frontmost clip whose picture covers `at`.
fn topmost_at(visible: &[ShownClip], at: egui::Pos2) -> Option<ShownClip> {
    // Against the picture as it is drawn, turned — not its upright box, which
    // would grab a turned clip by its empty corners and miss its tips.
    visible
        .iter()
        .find(|shown| {
            crate::preview_overlay::contains(
                shown.box_on_canvas,
                shown.transform.rotation_degrees,
                at,
            )
        })
        .copied()
}

/// Inspector (§58): what is selected, and the track switches.
pub fn inspector(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    ui.label(
        egui::RichText::new("Inspector")
            .strong()
            .color(theme::ruler_text()),
    );
    ui.add_space(4.0);

    // Scrollable, because this panel grows: sequence, selection, every track,
    // the proxy settings and the System diagnostics. Without it the lower
    // sections are simply unreachable — and the shorter the window, the more is
    // lost, which hits the §52.1 reference machine hardest (1080p at 125%
    // scaling leaves ~864 usable pixels).
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| inspector_body(ui, editor, state));
}

fn inspector_body(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    // Copy what the header needs, so the immutable borrow ends before the
    // format controls — which dispatch commands — need `editor` mutably.
    let Some((name, ticks, duration, clips, resolution, rate)) =
        editor.active_sequence().map(|s| {
            (
                s.name.clone(),
                s.ticks_per_frame(),
                s.duration(),
                s.clip_count(),
                s.resolution,
                s.frame_rate,
            )
        })
    else {
        ui.label("No sequence");
        return;
    };

    // The sequence's facts and format, folded away while a clip is
    // selected: the clip is what is being worked on, and it comes first.
    egui::CollapsingHeader::new(format!(
        "Sequence · {}×{} · {}",
        resolution.width,
        resolution.height,
        duration.format_timecode()
    ))
    .id_salt("inspector-sequence")
    .default_open(state.selected_clips.is_empty())
    .show(ui, |ui| {
        ui.monospace(format!("name      {name}"));
        ui.monospace(format!("frame     {ticks} ticks"));
        ui.monospace(format!("duration  {}", duration.format_timecode()));
        ui.monospace(format!("clips     {clips}"));

        sequence_format(ui, editor, state, resolution, rate);
    });

    if editor.active_sequence().is_none() {
        return;
    }

    ui.separator();
    theme::section(ui, "Selection");

    let selected: Vec<_> = state.selected_clips.iter().copied().collect();
    // A note typed for a clip that is no longer the one selected is kept.
    if state
        .note_draft
        .as_ref()
        .is_some_and(|(clip, _)| selected.as_slice() != [*clip])
    {
        commit_note(editor, state);
    }
    if let [clip] = selected.as_slice() {
        note_field(ui, editor, state, *clip);
    }
    let Some(sequence) = editor.active_sequence() else {
        return;
    };
    // Gather what the controls need, so the immutable borrow of the project
    // ends before any of them dispatches a command.
    let playhead = editor.playhead();
    let single = (selected.len() == 1).then(|| selected[0]).map(|id| {
        let video = sequence
            .video_tracks
            .iter()
            .find_map(|t| t.get(id).map(|c| VideoLook::of(c, playhead)));
        let audio = sequence
            .audio_tracks
            .iter()
            .find_map(|t| t.get(id).map(|c| (c.timeline, c.gain, c.media_id)));
        (id, video, audio)
    });

    // §26: a title takes different controls from a media clip — words and a
    // font, not a source range and a grade — so it gets its own panel rather
    // than a tab strip full of things it does not have.
    let text_selected = selected.len() == 1 && sequence.text_clip(selected[0]).is_some();
    // An adjustment has a grade and nothing else — no media, no transform, no
    // sound — so it gets its own panel too.
    let adjustment_selected =
        selected.len() == 1 && sequence.adjustment_clip(selected[0]).is_some();

    match (selected.len(), single) {
        (1, _) if text_selected => text_properties(ui, editor, state, selected[0]),
        (1, _) if adjustment_selected => adjustment_properties(ui, editor, state, selected[0]),
        // Nothing selected is not "nothing to adjust". The same controls now
        // apply to the finished video, which is the thing on screen when no
        // clip is picked out — and selecting a clip narrows them to it.
        (0, _) => {
            // Plain, not the keyframe blue: that colour means "animated"
            // everywhere else, and spending it on a heading would dilute the
            // one signal the Inspector has.
            ui.label(egui::RichText::new("Whole video").strong());
            ui.label(
                egui::RichText::new(
                    "These apply to everything at once. Click a clip — on the \
                     timeline or in the picture — to adjust just that one.",
                )
                .small()
                .color(theme::disabled()),
            );

            ui.add_space(6.0);
            // Audio too: the whole video has a volume (§20a.4's master gain).
            inspector_tabs(ui, state, true, true);
            ui.add_space(4.0);

            master_properties(ui, editor, state);
        }
        (1, Some((id, video, audio))) => {
            // The sound the Audio tab adjusts: this clip's own, or — for a
            // picture placed from a file with sound — the clip linked to it
            // (§12). Selecting the video and finding "no sound" when the sound
            // is right there beneath it is a dead end.
            let sound = audio.map(|a| (id, a.1)).or_else(|| {
                editor
                    .linked_with(id)
                    .into_iter()
                    .find_map(|c| editor.audio_clip(c).map(|a| (c, a.gain)))
            });
            let media_id = video.map(|v| v.media_id).or_else(|| audio.map(|a| a.2));
            let name = media_id
                .and_then(|m| editor.project().media_asset(m))
                .map_or_else(|| "(missing)".to_owned(), |m| m.display_name().to_owned());
            let range = video.map(|v| v.timeline).or_else(|| audio.map(|a| a.0));

            ui.monospace(format!("media     {name}"));
            if let Some(range) = range {
                ui.monospace(format!("start     {}", range.start.format_timecode()));
                ui.monospace(format!("duration  {}", range.duration().format_timecode()));
            }

            // Above the tabs, because it acts on the whole clip. Inside one it
            // would look like it acted on that tab, which is the confusion the
            // Animation tab's button used to cause.
            if video.is_some() {
                ui.add_space(2.0);
                if ui
                    .button("Reset clip")
                    .on_hover_text(
                        "Put every control on every tab back to its default, and \
                         remove all keyframes.",
                    )
                    .clicked()
                {
                    reset_video_properties(editor, state, id);
                }
            }

            ui.add_space(6.0);
            inspector_tabs(ui, state, video.is_some(), sound.is_some());
            ui.add_space(4.0);

            match state.inspector_tab {
                InspectorTab::Video => {
                    if let Some(look) = video {
                        clip_fill(ui, editor, state, id);
                        clip_video_properties(ui, editor, state, id, look);
                    } else {
                        unavailable(ui, "This clip has no picture.");
                    }
                }
                InspectorTab::Effects => {
                    if video.is_some() {
                        clip_effect_properties(ui, editor, state, id);
                    } else {
                        unavailable(ui, "This clip has no picture to put effects on.");
                    }
                }
                InspectorTab::Colours => {
                    if let Some(look) = video {
                        clip_colour_properties(ui, editor, state, id, look);
                    } else {
                        unavailable(ui, "This clip has no picture to grade.");
                    }
                }
                InspectorTab::Audio => {
                    if let Some((sound_id, gain)) = sound {
                        if sound_id != id {
                            ui.label(
                                egui::RichText::new("The sound that came with this clip.")
                                    .small()
                                    .color(theme::disabled()),
                            );
                        }
                        clip_audio_properties(ui, editor, state, sound_id, gain);
                    } else {
                        unavailable(ui, "This clip has no sound.");
                    }
                }
                InspectorTab::Speed => {
                    if video.is_some() && !editor.can_retime(id) {
                        // A hold and a photo are one picture: there is nothing
                        // to play faster, and offering the slider would let it
                        // shrink the clip to the frame it holds.
                        unavailable(ui, "A held frame and a photo have no motion to re-time.");
                    } else if video.is_some() {
                        clip_speed(ui, editor, state, id);
                    } else {
                        unavailable(ui, "Only picture can be re-timed so far.");
                    }
                }
                InspectorTab::Animation => {
                    if let Some(look) = video {
                        clip_animation(ui, editor, state, id, &look);
                    } else {
                        unavailable(ui, "Only picture can be animated so far.");
                    }
                }
            }
        }
        (n, _) => {
            ui.label(format!("{n} clips selected"));
            ui.label(
                egui::RichText::new("Select one clip to adjust its properties.")
                    .small()
                    .color(theme::disabled()),
            );
        }
    }

    let Some(sequence) = editor.active_sequence() else {
        return;
    };

    ui.separator();
    ui.label(egui::RichText::new("Tracks").strong());

    // Read the switch states, then dispatch after the borrow ends.
    let tracks: Vec<(TrackId, String, bool, bool, bool)> = sequence
        .video_tracks
        .iter()
        .map(|t| (t.id, t.name.clone(), t.enabled, t.locked, t.sync_lock))
        .chain(
            sequence
                .audio_tracks
                .iter()
                .map(|t| (t.id, t.name.clone(), t.enabled, t.locked, t.sync_lock)),
        )
        .collect();

    let mut toggles: Vec<(TrackId, TrackFlag, bool)> = Vec::new();

    for (id, name, enabled, locked, synced) in &tracks {
        ui.horizontal_wrapped(|ui| {
            ui.label(name);
            let mut visible = *enabled;
            if ui
                .checkbox(&mut visible, "on")
                .on_hover_text("Visible (video) or unmuted (audio)")
                .changed()
            {
                toggles.push((*id, TrackFlag::Enabled, visible));
            }
            let mut lock = *locked;
            if ui.checkbox(&mut lock, "lock").changed() {
                toggles.push((*id, TrackFlag::Locked, lock));
            }
            // §10's sync lock: the one track flag with no button on the
            // timeline header, so this is where it is turned on.
            let mut sync = *synced;
            if ui
                .checkbox(&mut sync, "sync")
                .on_hover_text("Ripple edits on other tracks move this one's clips too")
                .changed()
            {
                toggles.push((*id, TrackFlag::SyncLock, sync));
            }
        });
    }

    for (id, flag, value) in toggles {
        if let Err(err) = editor.set_track_flag(id, flag, value) {
            state.error(err.to_string());
        }
    }

    ui.separator();
    // Open by default, unlike "System" below: this is a setting a user changes,
    // not a diagnostic they go looking for.
    egui::CollapsingHeader::new("Proxies")
        .default_open(true)
        .show(ui, |ui| {
            // §13: "Allow the user to disable automatic proxies." Reached through a
            // command like every other project change (§54), so it survives a crash
            // (§38.2) and can be undone.
            let settings = &editor.project().settings;
            let mut automatic = settings.auto_generate_proxies;
            let mut mode = settings.performance_mode;
            let mut change = None;

            if ui
                .checkbox(&mut automatic, "Generate automatically")
                .on_hover_text(
                    "Heavy footage — 4K, HEVC, 10-bit, high frame rate — is copied to a \
                 smaller, easier format for editing.\nYour original files are never \
                 modified, and export always uses them.",
                )
                .changed()
            {
                change = Some(SettingChange::AutoGenerateProxies(automatic));
            }

            ui.horizontal_wrapped(|ui| {
                ui.label("Quality");
                egui::ComboBox::from_id_salt("performance_mode")
                    .selected_text(match mode {
                        PerformanceMode::Performance => "Smoothest",
                        PerformanceMode::Balanced => "Balanced",
                        PerformanceMode::Quality => "Sharpest",
                    })
                    .show_ui(ui, |ui| {
                        // Named for what the user gets, not for the resolution:
                        // the interface should explain itself.
                        let options = [
                            (PerformanceMode::Performance, "Smoothest", "360p proxies"),
                            (PerformanceMode::Balanced, "Balanced", "540p proxies"),
                            (PerformanceMode::Quality, "Sharpest", "720p proxies"),
                        ];
                        for (value, label, hint) in options {
                            if ui
                                .selectable_value(&mut mode, value, label)
                                .on_hover_text(hint)
                                .changed()
                            {
                                change = Some(SettingChange::PerformanceMode(value));
                            }
                        }
                    });
            });

            ui.label(
                egui::RichText::new("Changing the quality regenerates proxies in the background.")
                    .small()
                    .color(theme::disabled()),
            );

            if let Some(change) = change
                && let Err(err) =
                    editor.dispatch(bettercut_editor_core::Command::ChangeSetting { change })
            {
                state.error(err.to_string());
            }
        });

    ui.separator();
    // How long a photo runs when it goes on the timeline. A project setting,
    // through a command like the proxy choice, so it is undone and saved.
    egui::CollapsingHeader::new("Photos")
        .default_open(true)
        .show(ui, |ui| {
            let length = editor.project().settings.photo_length;
            let mut seconds = length.as_seconds_f64();
            let mut change = None;
            ui.horizontal_wrapped(|ui| {
                ui.label("Photo length");
                let response = ui
                    .add(
                        egui::DragValue::new(&mut seconds)
                            .range(0.5..=60.0)
                            .speed(0.1)
                            .fixed_decimals(1)
                            .suffix(" s"),
                    )
                    .on_hover_text(
                        "How long a photo or colour clip runs when you add it. \
                         Clips already on the timeline keep their length.",
                    );
                if response.changed() {
                    change = Some(seconds);
                }
                for preset in [3.0, 5.0, 10.0] {
                    if ui
                        .selectable_label((seconds - preset).abs() < 0.05, format!("{preset} s"))
                        .clicked()
                    {
                        change = Some(preset);
                    }
                }
            });
            if let Some(seconds) = change {
                let value = bettercut_editor_core::foundation::MediaTime::from_ticks(
                    (seconds * bettercut_editor_core::foundation::TICKS_PER_SECOND as f64).round()
                        as i64,
                );
                if value != length
                    && let Err(err) =
                        editor.dispatch(bettercut_editor_core::Command::ChangeSetting {
                            change: SettingChange::PhotoLength(value),
                        })
                {
                    state.error(err.to_string());
                }
            }

            // And the house transition: how long a dissolve is when one is
            // added, so every cut in a piece is the same cut.
            let house = editor.project().settings.transition_length;
            let mut seconds = house.as_seconds_f64();
            let mut change = None;
            ui.horizontal_wrapped(|ui| {
                ui.label("Transition length");
                let response = ui
                    .add(
                        egui::DragValue::new(&mut seconds)
                            .range(0.1..=5.0)
                            .speed(0.05)
                            .fixed_decimals(2)
                            .suffix(" s"),
                    )
                    .on_hover_text(
                        "How long a transition is when you add one. Transitions already on the timeline keep their length.",
                    );
                if response.changed() {
                    change = Some(seconds);
                }
                for preset in [0.25, 0.5, 1.0] {
                    if ui
                        .selectable_label((seconds - preset).abs() < 0.01, format!("{preset} s"))
                        .clicked()
                    {
                        change = Some(preset);
                    }
                }
            });
            if let Some(seconds) = change {
                let value = bettercut_editor_core::foundation::TimelineTime::from_ticks(
                    (seconds * bettercut_editor_core::foundation::TICKS_PER_SECOND as f64).round()
                        as i64,
                );
                if value != house
                    && let Err(err) =
                        editor.dispatch(bettercut_editor_core::Command::ChangeSetting {
                            change: SettingChange::TransitionLength(value),
                        })
                {
                    state.error(err.to_string());
                }
            }
        });

    ui.separator();
    ui.collapsing("System", |ui| {
        // The theme: about the room the editor is in, so it is the
        // interface's to remember (`crate::prefs`), not the project's.
        let mut light = theme::is_light();
        if ui
            .checkbox(&mut light, "Light theme")
            .on_hover_text("A light interface for a bright room. Remembered between runs.")
            .changed()
        {
            theme::set_light(light);
            theme::apply(ui.ctx());
            state.prefs.light_theme = light;
            if let Err(err) = state.prefs.save() {
                state.error(format!("Could not remember the theme: {err}"));
            }
            state.needs_repaint = true;
        }
        // And how big it is drawn: a laptop at arm's length and a monitor
        // across a desk want different sizes of the same interface.
        ui.horizontal_wrapped(|ui| {
            let mut scale = crate::prefs::sane_scale(state.prefs.interface_scale);
            let response = ui
                .add(
                    theme::labeled("interface size", egui::Slider::new(
                        &mut scale,
                        crate::prefs::MIN_SCALE..=crate::prefs::MAX_SCALE,
                    )
                    .step_by(0.05)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))),
                )
                .on_hover_text("How large everything is drawn. Remembered between runs.");
            if response.changed() {
                ui.ctx().set_zoom_factor(scale);
                state.prefs.interface_scale = scale;
                state.needs_repaint = true;
            }
            if (response.drag_stopped() || (response.changed() && !response.dragged()))
                && let Err(err) = state.prefs.save()
            {
                state.error(format!("Could not remember the size: {err}"));
            }
        });
        // How much work a crash may cost: the recovery journal's snapshot
        // interval, the person's to set on their own machine.
        ui.horizontal_wrapped(|ui| {
            let mut seconds = editor.autosave_seconds();
            let response = ui
                .add(
                    theme::labeled("autosave every", egui::Slider::new(&mut seconds, 10..=600)
                        .logarithmic(true)
                        .suffix(" s")),
                )
                .on_hover_text("How often the recovery snapshot is written while you work. Remembered between runs.");
            if response.changed() {
                editor.set_autosave_seconds(seconds);
                state.prefs.autosave_seconds = seconds;
            }
            if (response.drag_stopped() || (response.changed() && !response.dragged()))
                && let Err(err) = state.prefs.save()
            {
                state.error(format!("Could not remember the interval: {err}"));
            }
        });
        if ui
            .checkbox(&mut state.prefs.check_updates, "Check for new versions")
            .on_hover_text("Once a day, ask GitHub whether a newer bettercut is out, and say so in the status bar. Nothing is sent but the request.")
            .changed()
            && let Err(err) = state.prefs.save()
        {
            state.error(format!("Could not remember that: {err}"));
        }
        ui.separator();

        // §44: the editor configures itself for the machine. Showing what it
        // decided means a slow session can be diagnosed without a debug build.
        let hardware = editor.hardware();
        // Everything below the mode line is derived *from* the mode, so it has
        // to be the project's mode and not the hardware's recommendation —
        // otherwise the panel reports a mode beside numbers from a different
        // one, which is what it used to do.
        let mode = editor.project().settings.performance_mode;
        let recommended = hardware.recommended_mode();

        ui.monospace(format!("processors {}", hardware.logical_processors));
        ui.monospace(format!(
            "~cores     {}",
            hardware.estimated_physical_cores()
        ));
        ui.monospace(format!("mode       {mode:?}"));
        if mode != recommended {
            ui.monospace(
                egui::RichText::new(format!("           (machine suggests {recommended:?})"))
                    .small()
                    .color(theme::disabled()),
            );
        }
        ui.monospace(format!("heavy jobs {}", hardware.max_heavy_jobs(mode)));
        ui.monospace(format!(
            "ffmpeg     {} thread(s)/job",
            hardware.ffmpeg_threads_per_job(mode)
        ));
        ui.monospace(format!(
            "frame cache {} MB",
            hardware.frame_cache_bytes(mode) / (1024 * 1024)
        ));
        ui.monospace(format!("proxies    {}p", mode.proxy_resolution().height()));

        // §52/§81: the numbers that decide whether playback is actually
        // working. Counted whether or not anyone looks; showing them is what
        // makes a report reproducible.
        if let Some(stats) = state.playback {
            ui.separator();
            ui.monospace(format!(
                "playback   {}",
                if stats.playing { "playing" } else { "paused" }
            ));
            ui.monospace(format!("preview    {} scale", stats.quality));
            ui.monospace(format!("ring       {} frames", stats.ring_frames));
            ui.monospace(format!("prefetched {}", stats.prefetch_hits));

            // Coloured only when non-zero: a red number that is always there
            // stops being read.
            let dropped = format!("dropped    {}", stats.dropped_frames);
            if stats.dropped_frames > 0 {
                ui.monospace(egui::RichText::new(dropped).color(theme::error_text()))
                    .on_hover_text("§47a.4: frames too late to show. The machine is behind.");
            } else {
                ui.monospace(dropped);
            }

            let underruns = format!("underruns  {}", stats.underruns);
            if stats.underruns > 0 {
                ui.monospace(egui::RichText::new(underruns).color(theme::error_text()))
                    .on_hover_text("§20a: the audio device ran dry. This is audible.");
            } else {
                ui.monospace(underruns);
            }

            if stats.limited_samples > 0 {
                ui.monospace(
                    egui::RichText::new(format!("clipped    {}", stats.limited_samples))
                        .color(theme::error_text()),
                )
                .on_hover_text("§20a.4: the mix is too hot and the limiter is working.");
            }
        }

        // §49/§50: which adapter wgpu picked. Every performance number this
        // project has produced so far came from one discrete GPU, so a report
        // from anywhere else is only meaningful if it names the hardware.
        ui.separator();
        match &state.gpu {
            Some(gpu) => {
                ui.monospace(format!("gpu        {}", gpu.name));
                ui.monospace(format!("           {} · {}", gpu.kind.label(), gpu.backend));
                ui.monospace(format!("driver     {}", gpu.driver));
                ui.monospace(format!("device     {}", gpu.hardware_id));
                if gpu.kind.is_software() {
                    ui.label(
                        egui::RichText::new(
                            "No hardware GPU found — rendering in software, so \
                             playback will be slow.",
                        )
                        .small()
                        .color(theme::error_text()),
                    );
                }
            }
            None => {
                ui.monospace("gpu        unavailable");
            }
        }
    });
}

/// Transport controls, sitting directly above the timeline (§58).
///
/// Duplicates the toolbar's play button on purpose: this is where the eye
/// already is while cutting, and reaching to the top of the window to pause is
/// the kind of friction §88 says an editor must not have.
///
/// Labels are ASCII words rather than transport glyphs — egui's bundled font
/// has no ▶ or ⏮, and a missing glyph renders as an empty box.
pub fn transport(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    preview: Option<&mut crate::Preview>,
) {
    /// §55's `playback.skip`. Ten seconds is the conventional jump, and at
    /// 960,000 ticks/second it is exact.
    const SKIP: i64 = 10;

    ui.horizontal(|ui| {
        // Collect intent first, apply once at the end: `preview` is a single
        // mutable borrow, and both the transport and the seek need it.
        let mut seek_to: Option<TimelineTime> = state.jump_to.take();
        let mut toggle_play = false;
        let playhead = editor.playhead();
        let duration = editor
            .active_sequence()
            .map_or(TimelineTime::ZERO, |s| s.duration());
        let at_start = playhead == TimelineTime::ZERO;

        // Where the playhead is, first: the number read most often.
        ui.label(
            egui::RichText::new(playhead.format_timecode())
                .monospace()
                .size(14.0),
        );
        ui.label(
            egui::RichText::new(format!("/ {}", duration.format_timecode()))
                .monospace()
                .small()
                .color(theme::disabled()),
        );

        ui.separator();
        // The transport, together: start, back, play, forward, end.
        if ui
            .add_enabled(!at_start, egui::Button::new("|<"))
            .on_hover_text("Go to start (Home)")
            .clicked()
        {
            seek_to = Some(TimelineTime::ZERO);
        }
        if ui
            .add_enabled(!at_start, egui::Button::new("-10s"))
            .on_hover_text("Back ten seconds")
            .clicked()
        {
            // Saturating at zero rather than wrapping: a playhead before the
            // start of the timeline is not a position.
            seek_to = Some(
                TimelineTime::from_ticks(
                    playhead.ticks() - TimelineTime::from_seconds(SKIP).ticks(),
                )
                .max(TimelineTime::ZERO),
            );
        }

        let playing = preview.as_ref().is_some_and(|p| p.is_playing());
        let label = if playing { "Pause" } else { "Play" };
        if ui
            .add(egui::Button::new(egui::RichText::new(label).strong()))
            .on_hover_text("Space")
            .clicked()
        {
            toggle_play = true;
        }
        if ui
            .button("+10s")
            .on_hover_text("Forward ten seconds")
            .clicked()
        {
            seek_to = Some(TimelineTime::from_ticks(
                playhead.ticks() + TimelineTime::from_seconds(SKIP).ticks(),
            ));
        }
        if ui
            .add_enabled(playhead < duration, egui::Button::new(">|"))
            .on_hover_text("Go to end (End)")
            .clicked()
        {
            seek_to = Some(duration);
        }

        // J and L's speeds, which the Play button alone cannot show.
        let rate = preview.as_ref().map_or(0, |p| p.shuttle_rate());
        if rate != 0 && rate != 1 {
            ui.label(
                egui::RichText::new(crate::shuttle::describe(rate))
                    .monospace()
                    .color(theme::playhead()),
            )
            .on_hover_text("J / K / L: reverse, stop, forward — press again to go faster");
        }

        ui.separator();
        // Looping: the marked range, or the whole edit without marks.
        let looping = preview.as_ref().is_some_and(|p| p.is_looping());
        let mut toggle_loop = false;
        // How big the preview draws the picture.
        ui.menu_button(format!("View {}", state.preview_zoom.label()), |ui| {
            for zoom in crate::state::PreviewZoom::CHOICES {
                if ui
                    .selectable_label(state.preview_zoom == zoom, zoom.label())
                    .clicked()
                {
                    state.preview_zoom = zoom;
                    state.preview_pan = egui::Vec2::ZERO;
                    state.needs_repaint = true;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Look closer at the picture. Ctrl + scroll over it zooms; drag with the middle button to move around");
        // The preview's comparisons and guides, one menu.
        ui.menu_button("Preview", |ui| {
            if ui
                .selectable_label(looping, "Loop")
                .on_hover_text(crate::keys::keys(
                    "Play the stretch between the in and out marks over and over — \
                     or the whole timeline when there are none (Ctrl+L)",
                ))
                .clicked()
            {
                toggle_loop = true;
            }
            if ui
                .button("Full Screen")
                .on_hover_text("Watch the picture on the whole screen (F). Escape to come back")
                .clicked()
            {
                state.fullscreen = true;
                state.needs_repaint = true;
            }
            // Hearing the sound while the playhead is dragged.
            if ui
                .selectable_label(state.audio_scrub, "Scrub")
                .on_hover_text("Play the sound while the playhead is dragged or stepped")
                .clicked()
            {
                state.audio_scrub = !state.audio_scrub;
                state.needs_repaint = true;
            }

            // Before and after: the preview without any grade or effect.
            if ui
                .selectable_label(state.compare_original, "Before")
                .on_hover_text("Show the picture without its colour grade and effects, to compare. Click again for after")
                .clicked()
            {
                state.compare_original = !state.compare_original;
                state.needs_repaint = true;
            }
            // The same comparison with both halves on screen at once, which is
            // what makes a small change in a grade visible at all.
            if ui
                .selectable_label(state.compare_split, "Split")
                .on_hover_text(
                    "Show the original on one side of the picture and the graded \
                     version on the other. Drag the divider to move it.",
                )
                .clicked()
            {
                state.compare_split = !state.compare_split;
                if state.compare_split {
                    // The two comparisons would fight over the whole frame.
                    state.compare_original = false;
                }
                state.needs_repaint = true;
            }

            // Guides over the picture: thirds, safe margins, a phone app's buttons.
            ui.menu_button(
                if state.preview_guide == crate::state::PreviewGuide::Off {
                    "Guides"
                } else {
                    "Guides •"
                },
                |ui| {
                    for guide in crate::state::PreviewGuide::ALL {
                        if ui
                            .selectable_label(state.preview_guide == guide, guide.label())
                            .on_hover_text(guide.description())
                            .clicked()
                        {
                            state.preview_guide = guide;
                            state.needs_repaint = true;
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui
                        .checkbox(&mut state.preview_timecode, "Timecode")
                        .on_hover_text(
                            "The playhead's timecode in the corner of the preview, for a screen recording sent for notes. Never in the export.",
                        )
                        .changed()
                    {
                        state.needs_repaint = true;
                    }
                },
            )
            .response
            .on_hover_text("Lines over the preview for placing things. Never in the export");
        })
        .response
        .on_hover_text("Loop, full screen, scrub, before and after, split view and guides");
        // A voiceover: record while the edit plays; the take lands where the
        // playhead started.
        let label = match state.voiceover_live {
            Some((seconds, _)) => format!("Stop Recording {:.0}:{:02.0}", (seconds / 60.0).floor(), seconds % 60.0),
            None => "Record Voice".to_owned(),
        };
        let record = ui
            .add(egui::Button::new(if state.voiceover_live.is_some() {
                egui::RichText::new(label).color(theme::error_text())
            } else {
                egui::RichText::new(label)
            }))
            .on_hover_text("Record from the microphone while the edit plays. The recording goes on a sound track where the playhead was");
        if let Some((_, level)) = state.voiceover_live {
            // A small level bar, so a silent microphone shows before the take
            // is wasted.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 8.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2, theme::timeline_background());
            let filled = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(rect.width() * level.clamp(0.0, 1.0), rect.height()),
            );
            ui.painter().rect_filled(filled, 2, theme::playhead());
        }
        if record.clicked() {
            state.voiceover_toggle = true;
        }
        // What can be made from the frame under the playhead, one menu.
        ui.menu_button("Frame", |ui| {
            // Beside the timecode, because the frame it saves is the one that
            // timecode names.
            if ui
                .add_enabled(
                    duration > TimelineTime::ZERO,
                    egui::Button::new("Save Frame"),
                )
                .on_hover_text("Save the frame under the playhead as a full-size PNG")
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("PNG picture", &["png"])
                    .set_file_name(still_file_name(&editor.project().name, playhead))
                    .save_file()
            {
                state.still_request = Some((path, playhead));
            }
            // The whole cut as one picture: a thumbnail hunt, or something to
            // send to someone who cannot open a video.
            if ui
                .add_enabled(
                    duration > TimelineTime::ZERO,
                    egui::Button::new("Contact Sheet"),
                )
                .on_hover_text(
                    "A grid of frames across the video, saved as one PNG. Uses the marked range when there is one",
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("PNG picture", &["png"])
                    .set_file_name(format!("{} contact sheet.png", editor.project().name.trim()))
                    .save_file()
            {
                state.contact_sheet_request = Some(path);
            }

            // The cover: the frame that stands for the video, saved as a picture
            // beside every export.
            let cover = editor.cover_frame();
            let cover_label = match cover {
                Some(at) if at == playhead => "Cover (here)".to_owned(),
                Some(at) => format!("Cover {}", at.format_timecode()),
                None => "Set Cover".to_owned(),
            };
            let cover_button = ui
                .add_enabled(duration > TimelineTime::ZERO, egui::Button::new(cover_label))
                .on_hover_text(
                    "Make the frame under the playhead the video's cover: every export saves it as a picture beside the file. Right-click to clear",
                );
            if cover_button.clicked() {
                match editor.set_cover_frame(Some(playhead)) {
                    Ok(()) => state.info(format!("Cover frame set at {}", playhead.format_timecode())),
                    Err(err) => state.error(err.to_string()),
                }
            }
            if cover_button.secondary_clicked() && cover.is_some() {
                match editor.set_cover_frame(None) {
                    Ok(()) => state.info("Cover frame cleared"),
                    Err(err) => state.error(err.to_string()),
                }
            }
            if ui
                .add_enabled(
                    duration > TimelineTime::ZERO,
                    egui::Button::new("Copy Frame"),
                )
                .on_hover_text("Copy the frame under the playhead, full size, to paste anywhere")
                .clicked()
            {
                state.copy_frame_request = Some(playhead);
            }

            // Render in place: bake the marked stretch so a heavy section plays at
            // rate, and the export of it costs nothing to make twice.
            let marked = editor.active_sequence().and_then(|s| s.marked_range());
            let already = marked.is_some_and(|range| range_is_baked(editor, range));
            let render = ui
                .add_enabled(
                    marked.is_some() && !already,
                    egui::Button::new(if already { "Rendered" } else { "Render In/Out" }),
                )
                .on_hover_text(if marked.is_none() {
                    "Mark in and out on the timeline first (I and O), then bake that stretch to a file"
                } else if already {
                    "This stretch is already rendered. Right-click to throw every render away"
                } else {
                    "Bake the marked stretch to a file so it plays at rate; the export reuses it. Right-click to throw every render away"
                });
            if render.clicked()
                && let Some(range) = marked
            {
                state.render_request = Some(range);
            }
            if render.secondary_clicked() {
                match editor.clear_renders() {
                    0 => state.info("There is nothing rendered to throw away"),
                    n => state.info(format!("Threw away {n} rendered stretch(es)")),
                }
                state.needs_repaint = true;
            }

        })
        .response
        .on_hover_text("Save or copy this frame, set the cover, make a contact sheet, render the marked stretch");

        // §20a: what is going to the device, right now.
        if let Some(stats) = state.playback {
            ui.separator();
            draw_meter(ui, stats.peaks, stats.limited_samples > 0);
            draw_loudness(ui, stats.loudness, state.loudness_target);
        }

        let width = ui.available_width().max(400.0);

        match preview {
            Some(preview) => {
                if let Some(position) = seek_to {
                    editor.set_playhead(position);
                    // Keep the clock with the playhead, or resuming would jump
                    // back to wherever playback last was (§20a.1).
                    preview.seek_to(editor.playhead());
                }
                if toggle_play {
                    preview.set_playing(editor, !playing);
                    state.info(if playing { "Paused" } else { "Playing" });
                }
                if toggle_loop {
                    preview.set_looping(!looping);
                    state.info(if looping { "Loop off" } else { "Loop on" });
                }
            }
            None => {
                // §50: no renderer is not a reason to stop the playhead moving.
                if let Some(position) = seek_to {
                    editor.set_playhead(position);
                }
                if toggle_play {
                    state.error("No preview renderer");
                }
            }
        }

        if seek_to.is_some() || toggle_play {
            state.scroll_to_reveal(editor.playhead(), width);
            state.needs_repaint = true;
        }
    });
}

/// How full the meter's bar is for a peak level, 0.0 to 1.0.
///
/// A decibel scale, not a linear one. Linearly, everything from a whisper to a
/// shout crowds into the top fifth of the bar and the meter is decoration;
/// -60 dB to 0 spreads the range the way the ear hears it, which is the only
/// way the bar says anything useful about how loud the mix is.
pub fn meter_fraction(peak: f32) -> f32 {
    const FLOOR_DB: f32 = -60.0;
    if peak <= 0.0 || !peak.is_finite() {
        return 0.0;
    }
    let db = 20.0 * peak.max(1e-6).log10();
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// A stereo peak meter, two bars high (§20a).
///
/// Drawn on a decibel scale rather than a linear one: linearly, everything from
/// a whisper to a shout crowds into the top fifth of the bar and the meter is
/// decoration. -60 dB to 0 spreads it the way the ear hears it, which is the
/// only way the bar says anything useful about how loud the mix is.
///
/// Turns red when the limiter has had to clamp — the one thing a meter must
/// never be quiet about.
/// The live loudness reading, beside the peak meter
/// (`bettercut_audio::loudness::LiveLoudness`).
///
/// Peaks say whether a sample is about to clip; loudness says how loud the mix
/// *sounds*, which is the number every platform turns a delivery up or down to
/// hit. Both belong here, and neither answers the other's question.
///
/// The short-term reading is the one shown — three seconds, so it moves with
/// the mix rather than with every syllable — and the momentary one is in the
/// hover text for anyone chasing a single line.
fn draw_loudness(ui: &mut egui::Ui, (short, momentary): (Option<f32>, Option<f32>), target: f32) {
    let (text, colour) = match short {
        // Within a loudness unit of the target is on target: no platform, and
        // no listener, can tell 0.4 LU apart.
        Some(lufs) if (lufs - target).abs() <= 1.0 => (format!("{lufs:.1} LUFS"), theme::ok_text()),
        Some(lufs) if lufs > target => (format!("{lufs:.1} LUFS"), theme::caution()),
        Some(lufs) => (format!("{lufs:.1} LUFS"), theme::disabled()),
        None => ("— LUFS".to_owned(), theme::disabled()),
    };
    let hover = match (short, momentary) {
        (Some(short), Some(momentary)) => format!(
            "Short term {short:.1} LUFS over the last three seconds, momentary {momentary:.1} \
             over the last 400 ms, against a target of {target:.0}. \
             Above the target is turned down on delivery; well below it is turned up, noise and all."
        ),
        _ => format!(
            "Nothing to measure yet — play the edit. The target is {target:.0} LUFS; \
             choose another in the master row."
        ),
    };
    ui.label(egui::RichText::new(text).small().monospace().color(colour))
        .on_hover_text(hover);
}

fn draw_meter(ui: &mut egui::Ui, (left, right): (f32, f32), clipping: bool) {
    const WIDTH: f32 = 86.0;
    const BAR: f32 = 5.0;

    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(WIDTH, BAR * 2.0 + 3.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    for (index, peak) in [left, right].into_iter().enumerate() {
        let top = rect.top() + index as f32 * (BAR + 3.0);
        let track = egui::Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(WIDTH, BAR));
        painter.rect_filled(track, 1.0, theme::disabled().gamma_multiply(0.35));

        let filled = WIDTH * meter_fraction(peak);
        if filled > 0.5 {
            let colour = if clipping || peak >= 1.0 {
                theme::playhead()
            } else if peak > 0.7 {
                theme::selection()
            } else {
                theme::audio_clip_top()
            };
            painter.rect_filled(
                egui::Rect::from_min_size(track.min, egui::vec2(filled, BAR)),
                1.0,
                colour,
            );
        }
    }

    let db = |peak: f32| {
        if peak <= 0.0 {
            "-inf".to_owned()
        } else {
            format!("{:.0}", 20.0 * peak.log10())
        }
    };
    response.on_hover_text(format!(
        "Output level: {} dB left, {} dB right",
        db(left),
        db(right)
    ));
}

/// How long an export has left, from how long it has run and how far it has
/// got: "about 40 s left", "about 3 min left". `None` in its first seconds,
/// when the guess would swing wildly.
///
/// Rounded coarsely on purpose — to 5 seconds, then whole minutes — so the
/// number settles rather than flickering every frame.
pub fn export_time_left(elapsed: f64, fraction: f32) -> Option<String> {
    let fraction = f64::from(fraction);
    if !(elapsed.is_finite() && fraction.is_finite()) || elapsed < 3.0 || fraction < 0.03 {
        return None;
    }
    if fraction >= 1.0 {
        return Some("finishing".to_owned());
    }
    let left = elapsed * (1.0 - fraction) / fraction;
    Some(if left < 60.0 {
        format!(
            "about {} s left",
            ((left / 5.0).ceil() * 5.0).max(5.0) as u64
        )
    } else if left < 3600.0 {
        format!("about {} min left", (left / 60.0).round().max(1.0) as u64)
    } else {
        let minutes = (left / 60.0).round() as u64;
        format!("about {} h {} min left", minutes / 60, minutes % 60)
    })
}

/// One asset's poster image, or a placeholder of the same size.
///
/// The placeholder matters: without it the row height changes the moment a
/// thumbnail finishes generating, and the whole list jumps under the pointer.
/// Which filmstrip tile, and where in the file, a pointer `fraction` of the
/// way across a thumbnail shows. `None` outside it or with no tiles.
pub fn scrub_at(fraction: f32, tiles: u32, duration: MediaTime) -> Option<(u32, MediaTime)> {
    if tiles == 0 || !(0.0..=1.0).contains(&fraction) {
        return None;
    }
    let tile = ((fraction * tiles as f32) as u32).min(tiles - 1);
    let at = MediaTime::from_ticks((duration.ticks() as f64 * f64::from(fraction)).round() as i64);
    Some((tile, at))
}

/// A colour clip's tile in the media browser: its colours, top to bottom,
/// where a file would show its poster frame.
fn colour_swatch(ui: &mut egui::Ui, colour: bettercut_editor_core::media::Generated) {
    const BANDS: usize = 16;
    let width = ui.available_width().min(180.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, width * 9.0 / 16.0), egui::Sense::hover());
    // A compound has no colours of its own; the browser draws it as the
    // placeholder grey the generated entry itself would.
    let (top, bottom) = match colour {
        bettercut_editor_core::media::Generated::Colour { top, bottom } => (top, bottom),
        bettercut_editor_core::media::Generated::Compound { .. } => ([32, 34, 38], [32, 34, 38]),
    };
    let band = rect.height() / BANDS as f32;
    for i in 0..BANDS {
        let t = i as f32 / (BANDS - 1) as f32;
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        let y = rect.top() + band * i as f32;
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), y),
                egui::pos2(rect.right(), (y + band + 0.5).min(rect.bottom())),
            ),
            0,
            egui::Color32::from_rgb(
                mix(top[0], bottom[0]),
                mix(top[1], bottom[1]),
                mix(top[2], bottom[2]),
            ),
        );
    }
}

/// Draw a frame around the file a clip was traced back to, and bring it into
/// view — "Find in Media" is useless if the file it found is off the top of a
/// long list.
fn mark_revealed(ui: &egui::Ui, state: &UiState, media: MediaId, rect: egui::Rect) {
    if state.media_reveal != Some(media) {
        return;
    }
    ui.painter().rect_stroke(
        rect.expand(2.0),
        4,
        egui::Stroke::new(2.0, theme::selection()),
        egui::StrokeKind::Outside,
    );
    ui.scroll_to_rect(rect, Some(egui::Align::Center));
}

fn thumbnail(
    ui: &mut egui::Ui,
    state: &mut UiState,
    media: MediaId,
    missing: bool,
    duration: MediaTime,
) {
    let mut clicked_at: Option<MediaTime> = None;
    let width = ui.available_width().min(180.0);
    // 16:9 is only a guess for the placeholder — a real thumbnail draws at its
    // own aspect, letterboxed into this box rather than stretched.
    let size = egui::vec2(width, width * 9.0 / 16.0);

    let texture = state.thumbnails.texture(ui.ctx(), media).cloned();
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    // Hovering skims through the file: the filmstrip's tile under the pointer
    // stands in for the poster, with a line where in the file that is.
    if !missing
        && !duration.is_zero()
        && let Some(pointer) = response.hover_pos()
        && let Some((sheet, tiles)) = state.thumbnails.filmstrip(ui.ctx(), media)
        && let Some((tile, at)) =
            scrub_at((pointer.x - rect.left()) / rect.width(), tiles, duration)
    {
        let sheet_size = sheet.size_vec2();
        let tile_size = egui::vec2(sheet_size.x / tiles as f32, sheet_size.y);
        let scale = (rect.width() / tile_size.x).min(rect.height() / tile_size.y);
        let drawn = egui::Rect::from_center_size(rect.center(), tile_size * scale);
        let u0 = tile as f32 / tiles as f32;
        ui.painter()
            .rect_filled(rect, 4, theme::timeline_background());
        ui.painter().image(
            sheet.id(),
            drawn,
            egui::Rect::from_min_max(
                egui::pos2(u0, 0.0),
                egui::pos2(u0 + 1.0 / tiles as f32, 1.0),
            ),
            egui::Color32::WHITE,
        );
        ui.painter().line_segment(
            [
                egui::pos2(pointer.x, rect.bottom() - 6.0),
                egui::pos2(pointer.x, rect.bottom()),
            ],
            egui::Stroke::new(2.0, theme::playhead()),
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            TimelineTime::from_ticks(at.ticks()).format_timecode(),
            egui::FontId::monospace(10.0),
            egui::Color32::WHITE,
        );
        if response.clicked() {
            clicked_at = Some(at);
        }
        draw_marks(ui, state, media, rect, duration);
        if let Some(at) = clicked_at {
            match state.mark_media(media, at) {
                Some((from, to)) => state.info(format!(
                    "Marked {} to {} — “Add to timeline” places that part",
                    TimelineTime::from_ticks(from.ticks()).format_timecode(),
                    TimelineTime::from_ticks(to.ticks()).format_timecode()
                )),
                None => state.info("In-point marked; click again for the out-point"),
            }
        }
        if response.secondary_clicked() {
            state.clear_media_marks(media);
        }
        ui.ctx().request_repaint();
        return;
    }
    draw_marks(ui, state, media, rect, duration);

    match texture {
        Some(handle) if !missing => {
            let image = handle.size_vec2();
            let scale = (rect.width() / image.x).min(rect.height() / image.y);
            let drawn = egui::Rect::from_center_size(rect.center(), image * scale);
            ui.painter()
                .rect_filled(rect, 4, theme::timeline_background());
            ui.painter().image(
                handle.id(),
                drawn,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        _ => {
            ui.painter()
                .rect_filled(rect, 4, theme::timeline_background());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if missing { "missing" } else { "…" },
                egui::FontId::proportional(12.0),
                theme::disabled(),
            );
        }
    }
}

/// What the inspector needs to know about the selected video clip.
///
/// A struct rather than the tuple this started as. Milestone 8 added a field
/// per effect, and each one shifted the positional indices every reader used —
/// a rename the compiler cannot catch, because `.4` is still valid after the
/// meaning of position 4 changes.
#[derive(Clone, Copy)]
struct VideoLook {
    timeline: bettercut_editor_core::timeline::TimelineRange,
    media_id: MediaId,
    opacity: f32,
    transform: bettercut_editor_core::timeline::Transform,
    color: bettercut_editor_core::timeline::ColorAdjust,
    blur: f32,
    /// Where in the source the playhead is, or `None` when it is off the clip.
    /// A keyframe goes at the frame the user is looking at, so this is also
    /// whether one can be added at all (§24).
    source_time: Option<MediaTime>,
    /// Animation state per parameter, indexed as [`AnimatedParameter::ALL`].
    keys: [KeyState; AnimatedParameter::ALL.len()],
}

/// What the keyframe button on one row should show.
#[derive(Clone, Copy, Default)]
struct KeyState {
    /// The parameter is driven by keys rather than by its slider.
    animated: bool,
    /// A key sits exactly at the playhead, so the button deletes rather than
    /// adds.
    at_playhead: bool,
}

impl VideoLook {
    /// Read a clip at the playhead.
    ///
    /// The values are the *animated* ones — what is on screen — so a slider
    /// always starts from the number the user can see. With no keys that is
    /// simply the static value.
    fn of(clip: &VideoClip, playhead: TimelineTime) -> Self {
        let source_time = clip
            .timeline
            .contains(playhead)
            .then(|| clip.source_time_at(playhead));
        // Off the clip, show its first frame rather than nothing: the inspector
        // still has to display something, and the opening value is the least
        // surprising choice.
        let look = clip.look_at(source_time.unwrap_or(clip.source.start));

        Self {
            timeline: clip.timeline,
            media_id: clip.media_id,
            opacity: look.opacity,
            transform: look.transform,
            color: look.color,
            blur: look.blur,
            source_time,
            keys: AnimatedParameter::ALL.map(|parameter| KeyState {
                animated: clip.keyframes.is_animated(parameter),
                at_playhead: source_time
                    .is_some_and(|at| clip.keyframes.get(parameter, at).is_some()),
            }),
        }
    }

    /// The state of one control's row: a control that writes two parameters
    /// (position, scale) counts as animated when either of them is.
    fn row(&self, property: bettercut_editor_core::ClipProperty) -> KeyState {
        property
            .animated()
            .into_iter()
            .flatten()
            .filter_map(|(parameter, _)| {
                let index = AnimatedParameter::ALL
                    .iter()
                    .position(|p| *p == parameter)?;
                self.keys.get(index).copied()
            })
            .fold(KeyState::default(), |acc, state| KeyState {
                animated: acc.animated || state.animated,
                at_playhead: acc.at_playhead || state.at_playhead,
            })
    }
}

/// Opacity, transform and effects for the selected video clip (§59, §45).
///
/// Every control dispatches while being dragged, so the preview updates live,
/// and the whole drag collapses into one undo step — `continuing` is true
/// except on the frame the drag starts (§11: history holds intentions, not
/// mouse samples).
/// Which page of the Inspector's clip section is showing.
///
/// Tabs rather than a stack of collapsing headers. Stacked, the controls a user
/// reaches for constantly sat below ones they touch once a project, and the
/// panel grew a scrollbar as soon as anything was opened. Across the top, every
/// group is one click away and the panel never changes height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorTab {
    /// Where the picture *is*: transform, opacity, how it fills the frame.
    #[default]
    Video,
    /// The choices made about it: blend, mask, chroma key.
    ///
    /// Split from Video once there were four of them. The comment above says
    /// why tabs exist at all, and one tab holding both placement and every
    /// effect was the scrollbar it was written to avoid.
    Effects,
    /// Brightness, contrast, saturation.
    Colours,
    Audio,
    Speed,
    /// The keyframes on this clip, and a way to move between them.
    Animation,
}

impl InspectorTab {
    pub const ALL: [Self; 6] = [
        Self::Video,
        Self::Effects,
        Self::Colours,
        Self::Audio,
        Self::Speed,
        Self::Animation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Effects => "Effects",
            Self::Colours => "Colours",
            Self::Audio => "Audio",
            Self::Speed => "Speed",
            Self::Animation => "Animation",
        }
    }
}

/// The tab strip.
///
/// Tabs for things the clip does not have stay **visible but disabled** — an
/// audio clip still shows Video, greyed. A strip that changed shape with the
/// selection would move the tab under the pointer between one click and the
/// next.
fn inspector_tabs(ui: &mut egui::Ui, state: &mut UiState, has_video: bool, has_audio: bool) {
    ui.horizontal_wrapped(|ui| {
        for tab in InspectorTab::ALL {
            let enabled = match tab {
                InspectorTab::Video
                | InspectorTab::Effects
                | InspectorTab::Colours
                | InspectorTab::Animation => has_video,
                InspectorTab::Audio => has_audio,
                // Nothing to configure yet, but the tab is where it will be.
                InspectorTab::Speed => true,
            };
            let selected = state.inspector_tab == tab;
            let response = ui
                .add_enabled_ui(enabled, |ui| ui.selectable_label(selected, tab.label()))
                .inner;
            if response.clicked() {
                state.inspector_tab = tab;
            }
        }
    });

    // A tab that has just become unavailable — the selection changed to an
    // audio clip while Video was open — would otherwise show an explanation
    // with no way back that looked like the panel had broken.
    let available = match state.inspector_tab {
        InspectorTab::Video
        | InspectorTab::Effects
        | InspectorTab::Colours
        | InspectorTab::Animation => has_video,
        InspectorTab::Audio => has_audio,
        InspectorTab::Speed => true,
    };
    if !available {
        state.inspector_tab = if has_video {
            InspectorTab::Video
        } else {
            InspectorTab::Audio
        };
    }
}

/// The whole-video controls, shown when no clip is selected.
///
/// Deliberately the same rows the clip tabs use, minus the keyframe buttons: a
/// sequence-wide adjustment is one value for the whole video, so there is no
/// instant for a key to sit at. That is why these are plain sliders rather than
/// `keyed_row` — the missing diamond is the honest signal.
fn master_properties(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    use bettercut_editor_core::ClipProperty;

    let Some(sequence) = editor.active_sequence() else {
        return;
    };
    let master = sequence.master;
    let master_volume = sequence.master_volume;
    let mut change: Option<(ClipProperty, bool)> = None;
    let mut reset: Option<ClipProperty> = None;

    match state.inspector_tab {
        InspectorTab::Video => {
            let mut opacity = master.opacity;
            let response = master_row(
                ui,
                ClipProperty::Opacity(master.opacity),
                &mut reset,
                |ui| {
                    ui.add(theme::labeled(
                        "opacity",
                        egui::Slider::new(&mut opacity, 0.0..=1.0),
                    ))
                },
            );
            if response.changed() {
                change = Some((ClipProperty::Opacity(opacity), response.dragged()));
            }

            // §22: what the letterbox is made of. Here rather than on a clip
            // because it is a fact about the film, not about a shot — and it
            // is the difference between vertical footage in a wide frame
            // looking framed and looking like a mistake.
            let mut background = master.background;
            let response = master_row(
                ui,
                ClipProperty::Background(master.background),
                &mut reset,
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("background");
                        ui.color_edit_button_rgb(&mut background)
                            .on_hover_text("Shown in the bars and gaps, behind every track")
                    })
                    .inner
                },
            );
            if response.changed() {
                change = Some((ClipProperty::Background(background), false));
            }

            let mut scale = master.transform.scale.x;
            let current = ClipProperty::Scale {
                x: master.transform.scale.x,
                y: master.transform.scale.y,
            };
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "scale",
                    egui::Slider::new(&mut scale, 0.05..=4.0).logarithmic(true),
                ))
            });
            if response.changed() {
                change = Some((
                    ClipProperty::Scale { x: scale, y: scale },
                    response.dragged(),
                ));
            }

            let current = ClipProperty::Position {
                x: master.transform.position.x,
                y: master.transform.position.y,
            };
            master_row(ui, current, &mut reset, |ui| {
                ui.label("position");
                let mut x = master.transform.position.x;
                let mut y = master.transform.position.y;
                let rx = ui.add(egui::DragValue::new(&mut x).speed(0.005).range(-2.0..=2.0));
                let ry = ui.add(egui::DragValue::new(&mut y).speed(0.005).range(-2.0..=2.0));
                if rx.changed() || ry.changed() {
                    change = Some((
                        ClipProperty::Position { x, y },
                        rx.dragged() || ry.dragged(),
                    ));
                }
            });

            let mut rotation = master.transform.rotation_degrees;
            let current = ClipProperty::Rotation(master.transform.rotation_degrees);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "rotation",
                    egui::Slider::new(&mut rotation, -180.0..=180.0),
                ))
            });
            if response.changed() {
                change = Some((ClipProperty::Rotation(rotation), response.dragged()));
            }

            let mut blur = master.blur;
            let response = master_row(ui, ClipProperty::Blur(master.blur), &mut reset, |ui| {
                ui.add(theme::labeled(
                    "blur",
                    egui::Slider::new(&mut blur, 0.0..=bettercut_editor_core::timeline::MAX_BLUR)
                        .suffix("%"),
                ))
            });
            if response.changed() {
                change = Some((ClipProperty::Blur(blur), response.dragged()));
            }
            if blur > 0.0 {
                ui.label(
                    egui::RichText::new(
                        "This blurs the assembled picture, not each clip, which costs an extra full-frame pass.",
                    )
                    .small()
                    .color(theme::disabled()),
                );
            }
        }
        // §45's blur is the one effect the whole video has: a key is about the
        // screen a clip was shot against and a mask about the picture it was
        // drawn on, and neither is a property of the finished film.
        InspectorTab::Effects => unavailable(
            ui,
            "Keying and masking belong to a clip. Select one to give it either.",
        ),
        InspectorTab::Colours => {
            let mut brightness = master.color.brightness;
            let current = ClipProperty::Brightness(master.color.brightness);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "brightness",
                    egui::Slider::new(&mut brightness, 0.0..=2.0),
                ))
            });
            if response.changed() {
                change = Some((ClipProperty::Brightness(brightness), response.dragged()));
            }

            let mut contrast = master.color.contrast;
            let current = ClipProperty::Contrast(master.color.contrast);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "contrast",
                    egui::Slider::new(&mut contrast, 0.0..=2.0),
                ))
            });
            if response.changed() {
                change = Some((ClipProperty::Contrast(contrast), response.dragged()));
            }

            let mut saturation = master.color.saturation;
            let current = ClipProperty::Saturation(master.color.saturation);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "saturation",
                    egui::Slider::new(&mut saturation, 0.0..=2.0),
                ))
            });
            if response.changed() {
                change = Some((ClipProperty::Saturation(saturation), response.dragged()));
            }

            let mut vibrance = master.color.vibrance;
            let current = ClipProperty::Vibrance(master.color.vibrance);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "vibrance",
                    egui::Slider::new(&mut vibrance, -1.0..=1.0)
                        .custom_formatter(|v, _| warmth_label(v, "duller", "vivid")),
                ))
            });
            if response.changed() {
                change = Some((ClipProperty::Vibrance(vibrance), response.dragged()));
            }

            let mut wheels = master.color.wheels;
            let current = ClipProperty::Wheels(master.color.wheels);
            let response = master_row(ui, current, &mut reset, |ui| {
                let lift = crate::wheels::wheel(ui, "lift", &mut wheels.lift);
                let gamma = crate::wheels::wheel(ui, "gamma", &mut wheels.gamma);
                let gain = crate::wheels::wheel(ui, "gain", &mut wheels.gain);
                lift | gamma | gain
            });
            if response.changed() {
                change = Some((ClipProperty::Wheels(wheels), response.dragged()));
            }

            let mut pick = master.color.secondary;
            let current = ClipProperty::Secondary(master.color.secondary);
            let response = master_row(ui, current, &mut reset, |ui| {
                secondary_controls(ui, &mut pick)
            });
            if response.changed() {
                change = Some((ClipProperty::Secondary(pick), response.dragged()));
            }

            let mut temperature = master.color.temperature;
            let current = ClipProperty::Temperature(master.color.temperature);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "temperature",
                    egui::Slider::new(&mut temperature, -1.0..=1.0)
                        .custom_formatter(|v, _| warmth_label(v, "cool", "warm")),
                ))
            });
            if response.changed() {
                change = Some((ClipProperty::Temperature(temperature), response.dragged()));
            }

            let mut tint = master.color.tint;
            let current = ClipProperty::Tint(master.color.tint);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "tint",
                    egui::Slider::new(&mut tint, -1.0..=1.0)
                        .custom_formatter(|v, _| warmth_label(v, "green", "magenta")),
                ))
            });
            if response.changed() {
                change = Some((ClipProperty::Tint(tint), response.dragged()));
            }

            // Here and not on a clip: a vignette frames the frame.
            let mut vignette = master.vignette * 100.0;
            let current = ClipProperty::Vignette(master.vignette);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "vignette",
                    egui::Slider::new(&mut vignette, 0.0..=100.0).suffix("%"),
                ))
                .on_hover_text("Darken the edges of the frame, leaving the middle as it is")
            });
            if response.changed() {
                change = Some((ClipProperty::Vignette(vignette / 100.0), response.dragged()));
            }

            let mut grain = master.grain * 100.0;
            let current = ClipProperty::Grain(master.grain);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(theme::labeled(
                    "grain",
                    egui::Slider::new(&mut grain, 0.0..=100.0).suffix("%"),
                ))
                .on_hover_text("Film grain over the whole picture, moving every frame")
            });
            if response.changed() {
                change = Some((ClipProperty::Grain(grain / 100.0), response.dragged()));
            }

            // Black bands top and bottom, over everything: the widescreen
            // cinema look in one click.
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Cinematic bars")
                        .small()
                        .color(theme::disabled()),
                );
                let now = master.bars;
                if ui
                    .selectable_label(now == 0.0, "None")
                    .on_hover_text("No bars")
                    .clicked()
                    && now != 0.0
                {
                    change = Some((ClipProperty::Bars(0.0), false));
                }
                for (label, shape) in bettercut_editor_core::timeline::BAR_PRESETS {
                    if ui
                        .selectable_label((now - shape).abs() < 0.001, label)
                        .on_hover_text(
                            "Black bars top and bottom, cutting the picture to this shape, over titles too",
                        )
                        .clicked()
                    {
                        change = Some((ClipProperty::Bars(shape), false));
                    }
                }
            });

            // A logo in a corner of every frame, from the project's pictures.
            {
                use bettercut_editor_core::timeline::watermark::{Watermark, WatermarkCorner};
                let mark = editor.active_sequence().and_then(|s| s.watermark);
                let pictures: Vec<(bettercut_editor_core::foundation::MediaId, String)> = editor
                    .project()
                    .media
                    .iter()
                    .filter(|m| m.is_still() && m.generated.is_none())
                    .map(|m| (m.id, m.display_name().to_owned()))
                    .collect();
                let mut next = mark;
                let mut dragging = false;
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("Watermark")
                            .small()
                            .color(theme::disabled()),
                    );
                    let chosen_name = mark
                        .and_then(|m| pictures.iter().find(|(id, _)| *id == m.media))
                        .map_or("None".to_owned(), |(_, name)| name.clone());
                    egui::ComboBox::from_id_salt("watermark picture")
                        .selected_text(chosen_name)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(mark.is_none(), "None").clicked() {
                                next = None;
                            }
                            for (id, name) in &pictures {
                                let on = mark.is_some_and(|m| m.media == *id);
                                if ui.selectable_label(on, name).clicked() && !on {
                                    next = Some(match mark {
                                        Some(existing) => Watermark {
                                            media: *id,
                                            ..existing
                                        },
                                        None => Watermark::new(*id),
                                    });
                                }
                            }
                        })
                        .response
                        .on_hover_text(if pictures.is_empty() {
                            "Import a logo or photo first, then pick it here"
                        } else {
                            "A picture from the project to show in a corner of every frame"
                        });
                    if let Some(current) = next.as_mut() {
                        for corner in WatermarkCorner::ALL {
                            if ui
                                .selectable_label(current.corner == corner, corner.label())
                                .clicked()
                            {
                                current.corner = corner;
                            }
                        }
                        let mut size = current.size * 100.0;
                        let size_slider = ui.add(theme::labeled(
                            "size",
                            egui::Slider::new(&mut size, 3.0..=50.0).suffix("%"),
                        ));
                        if size_slider.changed() {
                            current.size = size / 100.0;
                        }
                        let mut opacity = current.opacity * 100.0;
                        let opacity_slider = ui.add(theme::labeled(
                            "opacity",
                            egui::Slider::new(&mut opacity, 0.0..=100.0).suffix("%"),
                        ));
                        if opacity_slider.changed() {
                            current.opacity = opacity / 100.0;
                        }
                        dragging = size_slider.dragged() || opacity_slider.dragged();
                    }
                });
                if next != mark {
                    match editor.set_watermark(next, dragging) {
                        Ok(()) => state.needs_repaint = true,
                        Err(err) => state.error(err.to_string()),
                    }
                }
            }

            // The visualizer, once one has been made from a sound clip.
            if let Some(visualizer) = editor.active_sequence().and_then(|s| s.visualizer.clone()) {
                let mut next = visualizer.clone();
                let mut dragging = false;
                let mut remove = false;
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("Visualizer")
                            .small()
                            .color(theme::disabled()),
                    );
                    let bars = ui.add(theme::labeled(
                        "bars",
                        egui::Slider::new(
                            &mut next.bars,
                            4..=bettercut_editor_core::timeline::visualizer::MAX_BARS,
                        ),
                    ));
                    dragging |= bars.dragged();
                    let mut height = next.height * 100.0;
                    let tall = ui.add(
                        theme::labeled("height", egui::Slider::new(
                            &mut height,
                            5.0..=bettercut_editor_core::timeline::visualizer::MAX_VISUALIZER_HEIGHT
                                * 100.0,
                        )
                        .suffix("%")),
                    );
                    if tall.changed() {
                        next.height = height / 100.0;
                    }
                    dragging |= tall.dragged();
                    if crate::swatches::colour_edit(ui, state, &mut next.colour, "") {
                        dragging |= ui.input(|i| i.pointer.any_down() && !i.pointer.any_pressed());
                    }
                    ui.checkbox(&mut next.top, "Top");
                    remove = ui
                        .button("Remove")
                        .on_hover_text("Take the visualizer off")
                        .clicked();
                });
                let result = if remove {
                    Some(editor.set_visualizer(None, false))
                } else if next != visualizer {
                    Some(editor.set_visualizer(Some(next), dragging))
                } else {
                    None
                };
                match result {
                    Some(Ok(())) => state.needs_repaint = true,
                    Some(Err(err)) => state.error(err.to_string()),
                    None => {}
                }
            }

            // A bar that fills as the video plays, the way short videos show
            // how far through they are.
            let bar = master.progress_bar;
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Progress bar")
                        .small()
                        .color(theme::disabled()),
                );
                let max = bettercut_editor_core::timeline::MAX_PROGRESS_BAR;
                for (label, height) in [("None", 0.0), ("Thin", max * 0.3), ("Thick", max)] {
                    if ui
                        .selectable_label((bar.height - height).abs() < 1e-4, label)
                        .clicked()
                    {
                        change = Some((
                            ClipProperty::ProgressBar(
                                bettercut_editor_core::timeline::ProgressBar { height, ..bar },
                            ),
                            false,
                        ));
                    }
                }
                if bar.is_visible() {
                    let mut colour = bar.colour;
                    if crate::swatches::colour_edit(ui, state, &mut colour, "Bar colour") {
                        let dragging =
                            ui.input(|i| i.pointer.any_down() && !i.pointer.any_pressed());
                        change = Some((
                            ClipProperty::ProgressBar(
                                bettercut_editor_core::timeline::ProgressBar { colour, ..bar },
                            ),
                            dragging,
                        ));
                    }
                    let mut top = bar.top;
                    if ui
                        .checkbox(&mut top, "Top")
                        .on_hover_text("Along the top of the frame instead of the bottom")
                        .changed()
                    {
                        change = Some((
                            ClipProperty::ProgressBar(
                                bettercut_editor_core::timeline::ProgressBar { top, ..bar },
                            ),
                            false,
                        ));
                    }
                }
            });

            // A copy sent out for notes: the timecode and the file name over
            // the picture, so a note can name the frame it is about.
            ui.horizontal_wrapped(|ui| {
                let burn = master.burn_in.clamped();
                ui.label(
                    egui::RichText::new("Burn-in")
                        .small()
                        .color(theme::disabled()),
                );
                let mut timecode = burn.timecode;
                if ui
                    .checkbox(&mut timecode, "Timecode")
                    .on_hover_text("Draw the time of each frame over the picture")
                    .changed()
                {
                    change = Some((
                        ClipProperty::BurnIn(bettercut_editor_core::timeline::BurnIn {
                            timecode,
                            ..burn
                        }),
                        false,
                    ));
                }
                let mut file_name = burn.file_name;
                if ui
                    .checkbox(&mut file_name, "File name")
                    .on_hover_text("Draw the name of the file the shot came from")
                    .changed()
                {
                    change = Some((
                        ClipProperty::BurnIn(bettercut_editor_core::timeline::BurnIn {
                            file_name,
                            ..burn
                        }),
                        false,
                    ));
                }
                if burn.is_visible() {
                    let mut top = burn.top;
                    if ui
                        .checkbox(&mut top, "Top")
                        .on_hover_text("Along the top of the frame instead of the bottom")
                        .changed()
                    {
                        change = Some((
                            ClipProperty::BurnIn(bettercut_editor_core::timeline::BurnIn {
                                top,
                                ..burn
                            }),
                            false,
                        ));
                    }
                    let mut size = burn.size;
                    let response = ui.add(theme::labeled(
                        "size",
                        egui::Slider::new(
                            &mut size,
                            bettercut_editor_core::timeline::MIN_BURN_IN_SIZE
                                ..=bettercut_editor_core::timeline::MAX_BURN_IN_SIZE,
                        )
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    ));
                    if response.changed() {
                        change = Some((
                            ClipProperty::BurnIn(bettercut_editor_core::timeline::BurnIn {
                                size,
                                ..burn
                            }),
                            response.dragged(),
                        ));
                    }
                }
            });

            // The whole video has looks too: grading the finished picture is
            // the usual way to give an edit one feel (§22).
            looks_row(ui, editor, state, None, master.color, false);
        }
        InspectorTab::Audio => {
            // §20a.4's master gain: everything at once, after every clip and
            // track has been mixed. Applied to the export as well as to what
            // plays here — the two must not differ (§46).
            let mut volume = master_volume;
            let response = master_row(ui, ClipProperty::Gain(master_volume), &mut reset, |ui| {
                ui.add(theme::labeled(
                    "volume",
                    egui::Slider::new(
                        &mut volume,
                        0.0..=bettercut_editor_core::timeline::sequence::MAX_MASTER_VOLUME,
                    )
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                ))
            });
            if response.changed() {
                change = Some((ClipProperty::Gain(volume), response.dragged()));
            }
            // What a platform will make of this mix, and a way to land on it.
            // Every platform turns a video up or down to its own target on
            // delivery; this is how to arrive there already.
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Loudness")
                        .small()
                        .color(theme::disabled()),
                );
                match state.loudness_measured {
                    Some(lufs) => {
                        ui.label(egui::RichText::new(format!("{lufs:.1} LUFS")).monospace());
                    }
                    None => {
                        ui.label(
                            egui::RichText::new("not measured")
                                .small()
                                .color(theme::disabled()),
                        );
                    }
                }
                for (name, target) in bettercut_audio::loudness::LOUDNESS_TARGETS {
                    if ui
                        .button(format!("{target:.0}"))
                        .on_hover_text(format!(
                            "{name}: measure the mix and set the master volume \
                             so it lands at {target:.0} LUFS"
                        ))
                        .clicked()
                    {
                        state.info("Measuring the mix…");
                        state.loudness_target = target;
                        crate::loudness::match_to(editor, state, target);
                    }
                }
            });

            ui.label(
                egui::RichText::new(
                    "Applies to the export too. Select a clip to set its own volume.",
                )
                .small()
                .color(theme::disabled()),
            );
        }
        InspectorTab::Speed => unavailable(
            ui,
            "Speed applies to one clip at a time. Select a clip to re-time it.",
        ),
        InspectorTab::Animation => unavailable(
            ui,
            "Whole-video adjustments hold one value throughout, so there is no instant to key them at. Select a clip to animate its controls.",
        ),
    }

    if let Some(property) = reset
        && let Err(err) = editor.reset_sequence_parameter(property)
    {
        state.error(err.to_string());
    }
    if let Some((property, continuing)) = change {
        match editor.set_sequence_value(property, continuing) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// One whole-video control: the slider, then its reset.
///
/// The keyframe button's place is left empty rather than filled with a disabled
/// one, so the rows still line up with the clip tabs and the absence reads as
/// "not applicable" rather than "broken".
fn master_row<R>(
    ui: &mut egui::Ui,
    current: bettercut_editor_core::ClipProperty,
    reset: &mut Option<bettercut_editor_core::ClipProperty>,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal_wrapped(|ui| {
        ui.add_space(18.0);
        let result = control(ui);

        let changed = !current.is_default();
        let button = egui::Button::new(egui::RichText::new("\u{21ba}").color(if changed {
            theme::clip_text()
        } else {
            theme::disabled()
        }))
        .frame(false)
        .min_size(egui::vec2(18.0, 18.0));

        if ui
            .add_enabled(changed, button)
            .on_hover_text(format!(
                "Reset {} for the whole video",
                current.kind().to_lowercase()
            ))
            .clicked()
        {
            *reset = Some(current);
        }
        result
    })
    .inner
}

/// A tab with nothing in it yet, saying so plainly.
fn unavailable(ui: &mut egui::Ui, message: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(message).color(theme::disabled()));
}

/// Brightness, contrast and saturation (§45's cheap colour adjustment).
/// The shape of a clip's picture and of the frame it is drawn in, for the fit
/// and fill buttons. `None` when the media's size is not known.
fn aspects(editor: &Editor, media: MediaId) -> Option<(f32, f32)> {
    let asset = editor.project().media_asset(media)?;
    let sequence = editor.active_sequence()?;
    let aspect = |w: u32, h: u32| (w > 0 && h > 0).then(|| w as f32 / h as f32);
    Some((
        aspect(asset.width, asset.height)?,
        aspect(sequence.resolution.width, sequence.resolution.height)?,
    ))
}

/// A white-balance slider's value, said in words.
///
/// "-0.45" is a number about nothing; "cool 45%" is the direction the picture
/// moves. Zero is "neutral" rather than "cool 0%", because no shift is not a
/// small amount of one.
/// The secondary's five controls: which colours, then what happens to them.
///
/// The pick is a hue slider labelled by the colour's name and a width; the
/// shifts are three sliders, each with an "as it was" in the middle. One
/// response for the lot, so a drag on any of them is one gesture.
fn secondary_controls(
    ui: &mut egui::Ui,
    pick: &mut bettercut_editor_core::timeline::HslSecondary,
) -> egui::Response {
    use bettercut_editor_core::timeline::HslSecondary;
    ui.vertical(|ui| {
        ui.label(
            egui::RichText::new("Secondary")
                .small()
                .color(theme::disabled()),
        );
        let hue = ui
            .add(theme::labeled(
                "pick",
                egui::Slider::new(&mut pick.hue, 0.0..=1.0)
                    .custom_formatter(|v, _| hue_name(v as f32).to_owned()),
            ))
            .on_hover_text("Which colour to work on: the middle of the range");
        let width = ui
            .add(theme::labeled(
                "reach",
                egui::Slider::new(&mut pick.width, 0.01..=HslSecondary::MAX_WIDTH)
                    .custom_formatter(|v, _| format!("{:.0}°", v * 360.0)),
            ))
            .on_hover_text("How far either side of the pick the change reaches");
        let shift = ui
            .add(theme::labeled(
                "hue",
                egui::Slider::new(&mut pick.hue_shift, -1.0..=1.0)
                    .custom_formatter(|v, _| format!("{:+.0}°", v * 180.0)),
            ))
            .on_hover_text("Turn the picked colours towards another colour");
        let saturation = ui
            .add(theme::labeled(
                "colour",
                egui::Slider::new(&mut pick.saturation, -1.0..=1.0)
                    .custom_formatter(|v, _| warmth_label(v, "less", "more")),
            ))
            .on_hover_text("More or less of the picked colour");
        let luminance = ui
            .add(theme::labeled(
                "brightness",
                egui::Slider::new(&mut pick.luminance, -1.0..=1.0)
                    .custom_formatter(|v, _| warmth_label(v, "darker", "brighter")),
            ))
            .on_hover_text("The picked colours brighter or darker, on their own");
        hue | width | shift | saturation | luminance
    })
    .inner
}

/// The name a hue goes by, for a slider that would otherwise read 0.37.
fn hue_name(turn: f32) -> &'static str {
    let names = [
        "red", "orange", "yellow", "lime", "green", "teal", "cyan", "azure", "blue", "violet",
        "magenta", "rose",
    ];
    let index = ((turn.rem_euclid(1.0) * names.len() as f32 + 0.5) as usize) % names.len();
    names[index]
}

fn warmth_label(value: f64, below: &str, above: &str) -> String {
    let percent = (value.abs() * 100.0).round() as i32;
    if percent == 0 {
        return "neutral".to_string();
    }
    let direction = if value < 0.0 { below } else { above };
    format!("{direction} {percent}%")
}

/// One-click grades, built from the colour controls (§45).
///
/// Presets rather than a curve editor: these are the adjustments people
/// actually reach for, and each is a handful of numbers that only mean
/// something together — which is why applying one is a single undo step.
///
/// Named for what they do to a shot, not for a film stock: "Punchy" says what
/// to expect, "Kodachrome" says it only to someone who already knows.
///
/// Written as full literals rather than `..ColorAdjust::IDENTITY`, so that a
/// new colour control cannot be added without someone deciding, here, what each
/// of these looks does with it.
pub const LOOKS: [(&str, ColorAdjust); 8] = [
    (
        "None",
        ColorAdjust {
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
            vibrance: 0.0,
            wheels: bettercut_editor_core::timeline::ColorWheels::IDENTITY,
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
    ),
    (
        "Punchy",
        ColorAdjust {
            brightness: 1.02,
            contrast: 1.25,
            saturation: 1.25,
            temperature: 0.0,
            tint: 0.0,
            vibrance: 0.25,
            wheels: bettercut_editor_core::timeline::ColorWheels::IDENTITY,
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
    ),
    (
        "Soft",
        ColorAdjust {
            brightness: 1.06,
            contrast: 0.9,
            saturation: 0.92,
            temperature: 0.0,
            tint: 0.0,
            vibrance: 0.1,
            wheels: bettercut_editor_core::timeline::ColorWheels::IDENTITY,
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
    ),
    (
        "Faded",
        ColorAdjust {
            brightness: 1.1,
            contrast: 0.82,
            saturation: 0.72,
            temperature: 0.0,
            tint: 0.0,
            vibrance: -0.1,
            wheels: bettercut_editor_core::timeline::ColorWheels::IDENTITY,
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
    ),
    (
        "Moody",
        ColorAdjust {
            brightness: 0.88,
            contrast: 1.18,
            saturation: 0.82,
            temperature: -0.18,
            tint: 0.0,
            vibrance: 0.15,
            wheels: bettercut_editor_core::timeline::ColorWheels::IDENTITY,
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
    ),
    // The two the white balance made possible. Golden hour and overcast are
    // what people are actually asking for when they reach for a filter.
    (
        "Warm",
        ColorAdjust {
            brightness: 1.02,
            contrast: 1.05,
            saturation: 1.08,
            temperature: 0.45,
            tint: 0.05,
            vibrance: 0.2,
            wheels: bettercut_editor_core::timeline::ColorWheels::IDENTITY,
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
    ),
    (
        "Cool",
        ColorAdjust {
            brightness: 1.0,
            contrast: 1.08,
            saturation: 0.95,
            temperature: -0.45,
            tint: -0.05,
            vibrance: 0.1,
            wheels: bettercut_editor_core::timeline::ColorWheels::IDENTITY,
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
    ),
    (
        "Black & white",
        ColorAdjust {
            brightness: 1.0,
            contrast: 1.08,
            saturation: 0.0,
            temperature: 0.0,
            tint: 0.0,
            vibrance: 0.0,
            wheels: bettercut_editor_core::timeline::ColorWheels::IDENTITY,
            secondary: bettercut_editor_core::timeline::HslSecondary::IDENTITY,
        },
    ),
];
/// Which preset produced `color`, and how strongly, if any did.
///
/// Recovered from the grade rather than remembered beside it. A clip stores the
/// numbers it ended up with and nothing else, so there is one record of what
/// the picture looks like — and the strength slider still knows where to sit
/// when a project is opened a week later, which a remembered value in the
/// interface would not.
///
/// "None" is skipped: everything is zero strength towards the identity, and
/// reporting that would light the None button up over every hand-made grade.
pub fn look_of(color: ColorAdjust) -> Option<(&'static str, f32)> {
    // An untouched clip is zero strength towards *every* look, so without this
    // the row would light up whichever preset happens to come first and offer
    // a strength slider sitting at nothing. Ungraded is its own answer.
    if color.is_identity() {
        return None;
    }
    LOOKS
        .iter()
        .skip(1)
        .find_map(|(name, look)| color.strength_towards(*look).map(|at| (*name, at)))
}

/// The row of looks. `clip` is `None` for the whole video.
///
/// Disabled while the colour is animated: keys override a static value (§24),
/// so the button would appear to do nothing.
fn looks_row(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: Option<bettercut_editor_core::foundation::ClipId>,
    color: ColorAdjust,
    animated: bool,
) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Looks").strong());
    let current = look_of(color);
    // A grade off the presets' lines has no strength to show, and the slider
    // starts where a click would put it.
    let mut strength = current.map_or(1.0, |(_, at)| at);
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for (name, look) in LOOKS {
            let response = ui.add_enabled(
                !animated,
                egui::Button::selectable(
                    current.map(|(on, _)| on) == Some(name)
                        || (name == "None" && current.is_none() && color.is_identity()),
                    name,
                ),
            );
            if response
                .on_disabled_hover_text(
                    "This clip's colour is animated, so a look would be overridden by its keyframes.",
                )
                .clicked()
            {
                chosen = Some(look);
            }
        }
    });
    // The dial CapCut puts under every filter: the same look, applied less.
    // Only offered once there is a look to weaken — on a hand-made grade there
    // is no line to slide along.
    if let Some((name, _)) = current {
        let full = LOOKS
            .iter()
            .find_map(|(candidate, look)| (*candidate == name).then_some(*look))
            .unwrap_or(ColorAdjust::IDENTITY);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            if ui
                .add_enabled(
                    !animated,
                    theme::labeled(
                        "strength",
                        egui::Slider::new(&mut strength, 0.0..=1.0).fixed_decimals(2),
                    ),
                )
                .on_hover_text("How much of the look to apply")
                .changed()
            {
                chosen = Some(ColorAdjust::IDENTITY.lerp(full, strength));
            }
        });
    }

    // The user's own looks, under the built-in ones: a grade arrived at once
    // and wanted again on every clip from the same camera.
    let mut apply_saved: Option<crate::looks::SavedLook> = None;
    if !state.user_looks.is_empty() {
        let mut remove: Option<String> = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("mine").small().color(theme::disabled()));
            for (name, look) in state.user_looks.all().to_vec() {
                let response = ui.add_enabled(
                    !animated,
                    // Selected on the grade alone: two looks can share a grade
                    // and differ in their finish, and the button is showing
                    // which grade is on the clip.
                    egui::Button::selectable(look.colour.near_enough(color), &name),
                );
                let response = response.on_hover_text(if look.has_effects() {
                    "Its grade and its finish — blur, sharpen, glow, old film, vignette. Right-click to forget it"
                } else {
                    "Right-click to forget this look"
                });
                if response.clicked() {
                    apply_saved = Some(look);
                }
                response.context_menu(|ui| {
                    if ui.button(format!("Forget “{name}”")).clicked() {
                        remove = Some(name.clone());
                        ui.close();
                    }
                });
            }
        });
        if let Some(name) = remove {
            state.user_looks.remove(&name);
            state.needs_repaint = true;
        }
    }

    // Saving the grade on screen. A name box rather than "Look 4": the point
    // of a saved look is finding it again.
    ui.horizontal_wrapped(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut state.look_name_draft)
                .desired_width(120.0)
                .char_limit(crate::looks::MAX_NAME)
                .hint_text("name this look"),
        );
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Save look").clicked() || entered) && !state.look_name_draft.trim().is_empty()
        {
            let name = std::mem::take(&mut state.look_name_draft);
            // What the clip looks like now, finish and all — saving only the
            // grade would quietly drop half of what the user is looking at.
            let look = clip.and_then(|clip| editor.video_clip(clip)).map_or_else(
                || crate::looks::SavedLook::graded(color),
                |video| crate::looks::SavedLook {
                    colour: color,
                    blur: video.blur,
                    sharpen: video.sharpen,
                    glow: video.glow,
                    old_film: video.old_film,
                    vignette: video.vignette,
                },
            );
            match state.user_looks.save(&name, look) {
                Ok(()) => {
                    state.info(format!("Saved the look “{}”", name.trim()));
                    state.needs_repaint = true;
                }
                Err(why) => state.error(why),
            }
        }
    });

    if let Some(look) = apply_saved {
        match clip {
            // The whole video takes the grade and nothing else: a master has
            // no glow or sharpen of its own to put a finish on.
            None => chosen = Some(look.colour),
            Some(clip) => {
                // One undo step for the lot, through the path Paste Look uses.
                use bettercut_editor_core::ClipProperty;
                let properties = vec![
                    ClipProperty::Brightness(look.colour.brightness),
                    ClipProperty::Contrast(look.colour.contrast),
                    ClipProperty::Saturation(look.colour.saturation),
                    ClipProperty::Temperature(look.colour.temperature),
                    ClipProperty::Tint(look.colour.tint),
                    ClipProperty::Vibrance(look.colour.vibrance),
                    ClipProperty::Blur(look.blur),
                    ClipProperty::Sharpen(look.sharpen),
                    ClipProperty::Glow(look.glow),
                    ClipProperty::OldFilm(look.old_film),
                    ClipProperty::Vignette(look.vignette),
                ];
                match editor.paste_look(&properties, [clip]) {
                    Ok(_) => state.needs_repaint = true,
                    Err(err) => state.error(err.to_string()),
                }
            }
        }
    }

    if let Some(look) = chosen {
        match editor.set_color_adjust(clip, look) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }
}

fn clip_colour_properties(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
    look: VideoLook,
) {
    use bettercut_editor_core::ClipProperty;

    let color = look.color;
    ui.add_space(4.0);

    // One-click looks, above the controls they set: pick one, then tune.
    let current = editor.filter_of(clip);
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new("Filter")
                .small()
                .color(theme::disabled()),
        );
        for filter in bettercut_editor_core::filters::Filter::ALL {
            if ui
                .selectable_label(current == Some(filter), filter.label())
                .on_hover_text(filter.description())
                .clicked()
            {
                chosen = Some(filter);
            }
        }
    });
    if let Some(filter) = chosen {
        let onto: Vec<_> = if state.selected_clips.contains(&clip) {
            state.selected_clips.iter().copied().collect()
        } else {
            vec![clip]
        };
        match editor.apply_filter(filter, onto) {
            Ok(n) if n > 1 => state.info(format!("{} on {n} clips", filter.label())),
            Ok(_) => {}
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
        return;
    }
    ui.add_space(4.0);

    let mut change: Option<(ClipProperty, bool)> = None;
    let mut toggle: Option<ClipProperty> = None;
    let mut reset: Option<ClipProperty> = None;

    let mut brightness = color.brightness;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Brightness(color.brightness),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "brightness",
                egui::Slider::new(&mut brightness, 0.0..=2.0),
            ))
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Brightness(brightness), response.dragged()));
    }

    let mut contrast = color.contrast;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Contrast(color.contrast),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "contrast",
                egui::Slider::new(&mut contrast, 0.0..=2.0),
            ))
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Contrast(contrast), response.dragged()));
    }

    let mut saturation = color.saturation;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Saturation(color.saturation),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "saturation",
                egui::Slider::new(&mut saturation, 0.0..=2.0),
            ))
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Saturation(saturation), response.dragged()));
    }

    // Vibrance under saturation, because it is the same control with a
    // conscience: it pushes hardest where there is least colour, so skin holds
    // while a flat sky comes back.
    let mut vibrance = color.vibrance;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Vibrance(color.vibrance),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "vibrance",
                egui::Slider::new(&mut vibrance, -1.0..=1.0)
                    .custom_formatter(|v, _| warmth_label(v, "duller", "vivid")),
            ))
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Vibrance(vibrance), response.dragged()));
    }

    // The wheels: lift for the darks, gamma for the middle, gain for the
    // brights. Under the multipliers because they are a grade built on top of
    // the correction, and a cast is rarely the same colour all the way up.
    let mut wheels = color.wheels;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Wheels(color.wheels),
        &mut toggle,
        &mut reset,
        |ui| {
            let lift = crate::wheels::wheel(ui, "lift", &mut wheels.lift);
            let gamma = crate::wheels::wheel(ui, "gamma", &mut wheels.gamma);
            let gain = crate::wheels::wheel(ui, "gain", &mut wheels.gain);
            lift | gamma | gain
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Wheels(wheels), response.dragged()));
    }

    // The secondary: one range of hue picked out and moved, everything else
    // left alone. Under the wheels because it is the finer tool — the wheels
    // set the whole picture, this fixes the one colour that is still wrong.
    let mut pick = color.secondary;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Secondary(color.secondary),
        &mut toggle,
        &mut reset,
        |ui| secondary_controls(ui, &mut pick),
    );
    if response.changed() {
        change = Some((ClipProperty::Secondary(pick), response.dragged()));
    }

    // The white balance, below the three multipliers and above the looks —
    // it is a correction, and the looks are built on top of it.
    let mut temperature = color.temperature;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Temperature(color.temperature),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "temperature",
                egui::Slider::new(&mut temperature, -1.0..=1.0)
                    .custom_formatter(|v, _| warmth_label(v, "cool", "warm")),
            ))
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Temperature(temperature), response.dragged()));
    }

    let mut tint = color.tint;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Tint(color.tint),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "tint",
                egui::Slider::new(&mut tint, -1.0..=1.0)
                    .custom_formatter(|v, _| warmth_label(v, "green", "magenta")),
            ))
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Tint(tint), response.dragged()));
    }

    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(
            "0 saturation is black and white; 1.0 is untouched. Temperature and tint are 0 when the picture is left alone.",
        )
        .small()
        .color(theme::disabled()),
    );

    let animated = [
        AnimatedParameter::Brightness,
        AnimatedParameter::Contrast,
        AnimatedParameter::Saturation,
        AnimatedParameter::Temperature,
        AnimatedParameter::Tint,
    ]
    .iter()
    .any(|p| {
        look.keys[AnimatedParameter::ALL
            .iter()
            .position(|a| a == p)
            .unwrap_or(0)]
        .animated
    });
    looks_row(ui, editor, state, Some(clip), color, animated);

    apply_row_actions(editor, state, clip, change, toggle, reset);

    // A frame around this one shot: darker edges on its own picture.
    let vignette = editor.video_clip(clip).map_or(0.0, |c| c.vignette);
    let mut next_vignette = vignette * 100.0;
    let response = ui
        .add(theme::labeled(
            "vignette",
            egui::Slider::new(&mut next_vignette, 0.0..=100.0).suffix("%"),
        ))
        .on_hover_text("Darken the edges of this clip's picture, leaving its middle as it is");
    if response.changed() {
        apply_clip_property(
            editor,
            state,
            clip,
            bettercut_editor_core::ClipProperty::Vignette(next_vignette / 100.0),
            response.dragged(),
        );
    }

    ui.add_space(8.0);
    colour_wheels(ui, editor, state, clip);
    colour_mixer(ui, editor, state, clip);
    curves_editor(ui, editor, state, clip);
}

/// One colour range at a time: pick a colour, then move its hue, saturation
/// and lightness. Written through the curves, like the wheels.
fn colour_mixer(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::timeline::curves::{ColourMixer, MIXER_BANDS};

    let Some(current) = editor.video_clip(clip).map(|c| c.curves) else {
        return;
    };
    let mut next = current;
    let mut dragging = false;
    let band_id = ui.id().with(("mixer band", clip));
    let mut band: usize = ui.data(|d| d.get_temp(band_id)).unwrap_or(0);

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Colour mixer").strong());
        for (index, (name, hue)) in MIXER_BANDS.iter().enumerate() {
            let [r, g, b] = hue_colour(*hue);
            let touched = next.mixer.bands[index] != Default::default();
            let mut text = egui::RichText::new(&name[..1])
                .color(egui::Color32::from_rgb(r, g, b))
                .size(15.0);
            // A range that has been changed stands out from those left alone.
            if touched {
                text = text.strong().underline();
            }
            if ui
                .add(egui::Button::selectable(band == index, text))
                .on_hover_text(*name)
                .clicked()
            {
                band = index;
            }
        }
        if !current.mixer.is_neutral()
            && ui
                .small_button("Reset")
                .on_hover_text("Every colour range back to as shot")
                .clicked()
        {
            next.mixer = ColourMixer::default();
        }
    });
    ui.data_mut(|d| d.insert_temp(band_id, band));

    let chosen = &mut next.mixer.bands[band];
    for (value, text, hint) in [
        (
            &mut chosen.hue,
            "hue",
            "Move this colour towards its neighbours round the wheel",
        ),
        (
            &mut chosen.saturation,
            "saturation",
            "More or less of this colour",
        ),
        (
            &mut chosen.lightness,
            "lightness",
            "This colour brighter or darker",
        ),
    ] {
        let mut percent = *value * 100.0;
        let response = ui
            .add(theme::labeled(
                text,
                egui::Slider::new(&mut percent, -100.0..=100.0).suffix("%"),
            ))
            .on_hover_text(format!("{} — {}", MIXER_BANDS[band].0, hint));
        if response.changed() {
            *value = percent / 100.0;
            dragging |= response.dragged();
        }
    }

    if next != current {
        apply_clip_property(
            editor,
            state,
            clip,
            bettercut_editor_core::ClipProperty::Curves(next),
            dragging,
        );
    }
}

/// A fully saturated colour for `hue` degrees, for a swatch.
fn hue_colour(hue: f32) -> [u8; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [r, g, b].map(|v: f32| (v * 255.0) as u8)
}

/// Three colour wheels — shadows, midtones, highlights — each a disc to drag a
/// colour into and a brightness slider beneath. Double-click a wheel to centre
/// it. Written through the curves, like the tone, so they share a table.
fn colour_wheels(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::timeline::curves::ColourWheels;

    let Some(current) = editor.video_clip(clip).map(|c| c.curves) else {
        return;
    };
    let mut next = current;
    let mut dragging = false;

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Colour wheels").strong());
        if !current.wheels.is_neutral()
            && ui
                .small_button("Centre all")
                .on_hover_text("Take every wheel back to no change")
                .clicked()
        {
            next.wheels = ColourWheels::default();
        }
    });
    let side = ((ui.available_width() - 16.0) / 3.0).clamp(60.0, 110.0);
    ui.horizontal_wrapped(|ui| {
        let wheels = [
            ("Shadows", &mut next.wheels.shadows),
            ("Midtones", &mut next.wheels.midtones),
            ("Highlights", &mut next.wheels.highlights),
        ];
        for (name, offsets) in wheels {
            ui.vertical(|ui| {
                ui.set_width(side);
                ui.label(egui::RichText::new(name).small());
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
                let painter = ui.painter_at(rect);
                let centre = rect.center();
                let radius = side / 2.0 - 2.0;
                // The hue ring, in wedges, fading to grey in the middle.
                let wedges = 36;
                for i in 0..wedges {
                    let a0 = i as f32 / wedges as f32 * std::f32::consts::TAU;
                    let a1 = (i + 1) as f32 / wedges as f32 * std::f32::consts::TAU;
                    let rgb = ColourWheels::offsets((a0 + a1) / 2.0, 1.0, 0.0)
                        .map(|v| ((0.5 + v * 0.5).clamp(0.0, 1.0) * 255.0) as u8);
                    let point = |a: f32| centre + radius * egui::vec2(a.cos(), -a.sin());
                    let mut mesh = egui::Mesh::default();
                    mesh.colored_vertex(centre, egui::Color32::from_gray(128));
                    mesh.colored_vertex(point(a0), egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                    mesh.colored_vertex(point(a1), egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                    mesh.add_triangle(0, 1, 2);
                    painter.add(egui::Shape::mesh(mesh));
                }
                let (_, _, brightness) = ColourWheels::position(*offsets);
                if (response.dragged() || response.clicked())
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    let d = pointer - centre;
                    let new_angle = (-d.y).atan2(d.x);
                    let new_distance = (d.length() / radius).min(1.0);
                    *offsets = ColourWheels::offsets(new_angle, new_distance, brightness);
                    dragging |= response.dragged();
                }
                if response.double_clicked() {
                    *offsets = [0.0; 3];
                }
                let (angle, distance, _) = ColourWheels::position(*offsets);
                let dot = centre + radius * distance * egui::vec2(angle.cos(), -angle.sin());
                painter.circle_stroke(dot, 5.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                response.on_hover_text(format!(
                    "Drag to push a colour into the {}. Double-click to centre",
                    name.to_lowercase()
                ));

                let mut level = brightness * 100.0;
                // As wide as the wheel above it. Through `theme::labeled` it
                // was a label column and a full slider wide, three of them made
                // the tab twice the inspector's width, and the preview drew
                // over the inspector's left edge.
                ui.spacing_mut().slider_width = side;
                let slider =
                    ui.add(egui::Slider::new(&mut level, -100.0..=100.0).show_value(false));
                if slider.changed() {
                    let (a, d, _) = ColourWheels::position(*offsets);
                    *offsets = ColourWheels::offsets(a, d, level / 100.0);
                    dragging |= slider.dragged();
                }
            });
        }
    });

    if next != current {
        apply_clip_property(
            editor,
            state,
            clip,
            bettercut_editor_core::ClipProperty::Curves(next),
            dragging,
        );
    }
}

/// Which curve the curves editor is showing: 0 master, then red, green, blue.
pub fn curve_mut(
    curves: &mut bettercut_editor_core::timeline::curves::ColourCurves,
    channel: usize,
) -> &mut bettercut_editor_core::timeline::curves::Curve {
    match channel {
        1 => &mut curves.red,
        2 => &mut curves.green,
        3 => &mut curves.blue,
        _ => &mut curves.master,
    }
}

/// The handle nearest `pointer` in a curve drawn in `rect`, if one is close
/// enough to grab.
pub fn curve_handle_at(
    curve: &bettercut_editor_core::timeline::curves::Curve,
    rect: egui::Rect,
    pointer: egui::Pos2,
) -> Option<usize> {
    let last = (curve.len() - 1) as f32;
    curve
        .iter()
        .enumerate()
        .map(|(i, y)| {
            let at = egui::pos2(
                rect.left() + rect.width() * i as f32 / last,
                rect.bottom() - rect.height() * y,
            );
            (i, at.distance(pointer))
        })
        .filter(|(_, d)| *d <= 14.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Colour curves: a channel picker and a square to drag the five handles in.
fn curves_editor(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::timeline::curves::{STRAIGHT, sample};

    let Some(current) = editor.video_clip(clip).map(|c| c.curves) else {
        return;
    };
    let mut next = current;
    let channel_id = ui.id().with(("curve channel", clip));
    let mut channel: usize = ui.data(|d| d.get_temp(channel_id)).unwrap_or(0);

    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Curves").strong());
        for (index, name) in ["RGB", "R", "G", "B"].into_iter().enumerate() {
            if ui.selectable_label(channel == index, name).clicked() {
                channel = index;
            }
        }
        if !current.is_straight()
            && ui
                .small_button("Straighten")
                .on_hover_text("Put all four curves back to straight lines")
                .clicked()
        {
            next = bettercut_editor_core::timeline::curves::ColourCurves {
                tone: current.tone,
                wheels: current.wheels,
                mixer: current.mixer,
                ..Default::default()
            };
        }
    });

    // The tone over the whole picture: sepia, or two colours of your choosing.
    {
        use bettercut_editor_core::timeline::curves::Tone;
        let tone = current.tone;
        let mut chosen = None;
        let mut dragging = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Tone").small().color(theme::disabled()));
            if ui.selectable_label(tone.is_none(), "None").clicked() {
                chosen = Some(Tone::None);
            }
            if ui
                .selectable_label(tone == Tone::Sepia, "Sepia")
                .on_hover_text("The warm brown of an old photograph")
                .clicked()
            {
                chosen = Some(Tone::Sepia);
            }
            let is_duotone = matches!(tone, Tone::Duotone { .. });
            if ui
                .selectable_label(is_duotone, "Duotone")
                .on_hover_text("Shadows in one colour, highlights in another")
                .clicked()
                && !is_duotone
            {
                chosen = Some(Tone::DUOTONE);
            }
            if let Tone::Duotone {
                mut shadow,
                mut highlight,
            } = tone
            {
                let a = crate::swatches::colour_edit(ui, state, &mut shadow, "Shadow colour");
                let b = crate::swatches::colour_edit(ui, state, &mut highlight, "Highlight colour");
                if a || b {
                    chosen = Some(Tone::Duotone { shadow, highlight });
                    dragging = ui.input(|i| i.pointer.any_down() && !i.pointer.any_pressed());
                }
            }
        });
        if let Some(chosen) = chosen
            && chosen != tone
        {
            apply_clip_property(
                editor,
                state,
                clip,
                bettercut_editor_core::ClipProperty::Tone(chosen),
                dragging,
            );
            return;
        }
    }
    ui.data_mut(|d| d.insert_temp(channel_id, channel));

    let side = ui.available_width().min(220.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2, theme::timeline_background());
    let grid = egui::Stroke::new(1.0, theme::grid_line());
    for i in 1..4 {
        let t = i as f32 / 4.0;
        painter.line_segment(
            [
                egui::pos2(rect.left() + rect.width() * t, rect.top()),
                egui::pos2(rect.left() + rect.width() * t, rect.bottom()),
            ],
            grid,
        );
        painter.line_segment(
            [
                egui::pos2(rect.left(), rect.top() + rect.height() * t),
                egui::pos2(rect.right(), rect.top() + rect.height() * t),
            ],
            grid,
        );
    }
    painter.line_segment([rect.left_bottom(), rect.right_top()], grid);

    // Grab the nearest handle on press, then move it up and down while held.
    let grab_id = ui.id().with(("curve grab", clip));
    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let grabbed = curve_handle_at(curve_mut(&mut next, channel), rect, pointer);
        ui.data_mut(|d| d.insert_temp(grab_id, grabbed));
    }
    let grabbed: Option<usize> = ui.data(|d| d.get_temp(grab_id)).flatten();
    if response.dragged()
        && let (Some(handle), Some(pointer)) = (grabbed, response.interact_pointer_pos())
    {
        let y = ((rect.bottom() - pointer.y) / rect.height()).clamp(0.0, 1.0);
        curve_mut(&mut next, channel)[handle] = y;
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<Option<usize>>(grab_id));
    }
    if response.double_clicked() {
        *curve_mut(&mut next, channel) = STRAIGHT;
    }

    let colour = match channel {
        1 => egui::Color32::from_rgb(235, 80, 80),
        2 => egui::Color32::from_rgb(80, 210, 100),
        3 => egui::Color32::from_rgb(90, 140, 240),
        _ => egui::Color32::from_gray(230),
    };
    let curve = *curve_mut(&mut next, channel);
    let points: Vec<egui::Pos2> = (0..=64)
        .map(|i| {
            let x = i as f32 / 64.0;
            egui::pos2(
                rect.left() + rect.width() * x,
                rect.bottom() - rect.height() * sample(&curve, x),
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, colour)));
    for (i, y) in curve.iter().enumerate() {
        let at = egui::pos2(
            rect.left() + rect.width() * i as f32 / 4.0,
            rect.bottom() - rect.height() * y,
        );
        painter.circle_filled(at, if grabbed == Some(i) { 6.0 } else { 4.5 }, colour);
    }
    response.on_hover_text("Drag a handle up to brighten that part of the range, down to darken it. Double-click to straighten this curve.");

    if next != current {
        apply_clip_property(
            editor,
            state,
            clip,
            bettercut_editor_core::ClipProperty::Curves(next),
            grabbed.is_some(),
        );
    }
}

/// The keyframes on this clip: how many, where, and a way back to the defaults.
fn clip_animation(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
    look: &VideoLook,
) {
    ui.add_space(4.0);

    let animated = look.keys.iter().any(|key| key.animated);
    if !animated {
        ui.label(
            egui::RichText::new(
                "Nothing on this clip is animated yet.

Press the ○ beside any                  control in Video or Colours to pin its value at the playhead.                  Move the playhead, change the value, and it moves between them.",
            )
            .color(theme::disabled()),
        );
        return;
    }

    animation_summary(ui, editor, state, clip, look);
    keyframe_graph(ui, editor, state, clip, look);

    ui.add_space(6.0);
    // Only the keys. This used to call the whole-clip reset, which also put
    // every value back to default — so a careful colour grade disappeared
    // behind a button that said nothing about colour. The hover text admitted
    // it, which is no defence when the label reads unambiguously.
    if ui
        .button("Remove all keyframes")
        .on_hover_text(
            "Stop animating every control. Each one goes back to reading its \
             own value; nothing else changes.",
        )
        .clicked()
    {
        match editor.clear_clip_keyframes(clip) {
            Ok(()) => {
                state.needs_repaint = true;
                state.info("Keyframes removed");
            }
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// The choices made about a picture: blend, mask, chroma key (§22).
///
/// Split from `clip_video_properties`, which had grown to eight control groups
/// — placement and every effect in one tab, which is the scrollbar
/// `InspectorTab`'s own comment says tabs exist to avoid.
///
/// Blur stays with the Video tab despite being an effect, because it is
/// *animatable*: it shares the keyframe button and the reset machinery with
/// opacity and the transform, and the three of them are one group. Nothing
/// here is keyframable — a mask shape or a blend mode is a choice, not a dial.
/// One folding section on the Effects tab.
///
/// Four effects, three of them a dozen controls each, made a column nobody
/// could scan — and three of the four are off on almost every clip. Folded, the
/// tab opens as four lines saying what this clip has, with those sections
/// already open and the rest a click away.
fn effect_section(
    ui: &mut egui::Ui,
    effect: crate::effects::Effect,
    active: bool,
    refold: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    let id = ui.make_persistent_id(effect.title());
    let mut header =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, active);
    if refold {
        header.set_open(active);
    }
    header
        .show_header(ui, |ui| {
            ui.label(effect.title());
            // Says which of the folded sections are doing something, so the
            // user does not have to open all four to find out.
            if active {
                ui.label(egui::RichText::new("on").small().color(theme::marker()));
            }
        })
        .body(body);
}

fn clip_effect_properties(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use crate::effects::Effect;
    use bettercut_editor_core::ClipProperty;

    ui.add_space(4.0);
    let mut change: Option<(ClipProperty, bool)> = None;

    // Which sections open is a fact about *this* clip, and egui remembers a
    // fold by the header's id — so the answer is recomputed whenever the
    // selection moves rather than inherited from the clip before.
    let refold = state.effects_for != Some(clip);
    state.effects_for = Some(clip);
    let active = editor.video_clip(clip).map_or([false; 6], |clip| {
        Effect::ALL.map(|effect| effect.active(clip))
    });

    // Clip animations first: of everything on this tab it is the one a phone
    // editor puts on almost every clip, and burying it under the mask would be
    // hiding the common case behind the rare one.
    effect_section(ui, Effect::Animation, active[0], refold, |ui| {
        use bettercut_editor_core::timeline::MotionKind;

        let mut motion = editor
            .video_clip(clip)
            .map(|clip| clip.motion)
            .unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            ui.label("animation")
                .on_hover_text("How this shot arrives and leaves, on its own");
        });
        let (mut intro, mut outro) = (motion.intro, motion.outro);
        let touched = motion_ends(
            ui,
            [
                ("in", "clip intro", &mut intro),
                ("out", "clip outro", &mut outro),
            ],
            &MotionKind::ALL,
        );
        if touched {
            motion.intro = intro;
            motion.outro = outro;
            change = Some((ClipProperty::Motion(motion), false));
        }

        // Beside the animation, which is the usual reason to want it. A shot
        // that is not moving pays nothing for having it on, so it is a plain
        // switch rather than a dial.
        let mut blurring = editor.video_clip(clip).is_some_and(|clip| clip.motion_blur);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            if ui
                .checkbox(&mut blurring, "motion blur")
                .on_hover_text("Smear the shot along the way it is moving")
                .changed()
            {
                change = Some((ClipProperty::MotionBlur(blurring), false));
            }
        });
    });

    // §22's blend mode. Beside opacity, because the two together are how an
    // overlay is laid on.
    effect_section(ui, Effect::Blend, active[1], refold, |ui| {
        use bettercut_editor_core::timeline::BlendMode;

        let current = editor
            .video_clip(clip)
            .map(|clip| clip.blend)
            .unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            theme::row_label(ui, "blend");
            for mode in BlendMode::ALL {
                if ui
                    .add(egui::Button::selectable(current == mode, mode.label()))
                    .on_hover_text(match mode {
                        BlendMode::Normal => "Cover what is beneath",
                        BlendMode::Screen => {
                            "Never darkens — black disappears, for glows and leaks"
                        }
                        BlendMode::Multiply => "Never lightens — white disappears, for shadows",
                        BlendMode::Add => "Brightest of all, for sparks and flares",
                    })
                    .clicked()
                    && current != mode
                {
                    change = Some((ClipProperty::Blend(mode), false));
                }
            }
        });
    });

    // The mask. The shape first, because the rest of the controls only mean
    // something once there is a shape for them to act on.
    effect_section(ui, Effect::Mask, active[2], refold, |ui| {
        use bettercut_editor_core::timeline::{Mask, MaskShape};

        let mask = editor.video_clip(clip).and_then(|clip| clip.mask);
        ui.horizontal_wrapped(|ui| {
            theme::row_label(ui, "mask");
            if ui
                .add(egui::Button::selectable(mask.is_none(), "None"))
                .on_hover_text("Show the whole picture")
                .clicked()
                && mask.is_some()
            {
                change = Some((ClipProperty::Mask(None), false));
            }
            for shape in MaskShape::ALL {
                let selected = mask.is_some_and(|mask| mask.shape == shape);
                if ui
                    .add(egui::Button::selectable(selected, shape.label()))
                    .on_hover_text(match shape {
                        MaskShape::Linear => "Keep one side of a straight edge",
                        MaskShape::Rectangle => "Keep a box",
                        MaskShape::Ellipse => "Keep an oval",
                        MaskShape::Star => "Keep a star",
                        MaskShape::Heart => "Keep a heart",
                        MaskShape::Mirror => "Keep a band between two straight edges",
                    })
                    .clicked()
                    && !selected
                {
                    // Keeps whatever the other dials were set to, so trying
                    // the shapes does not reset the work of placing one.
                    change = Some((
                        ClipProperty::Mask(Some(Mask {
                            shape,
                            ..mask.unwrap_or_default()
                        })),
                        false,
                    ));
                }
            }
        });

        if let Some(mask) = mask {
            let mut next = mask;
            let mut moved = false;

            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                ui.label("centre");
                moved |= ui
                    .add(
                        egui::DragValue::new(&mut next.center[0])
                            .speed(0.005)
                            .prefix("x "),
                    )
                    .changed();
                moved |= ui
                    .add(
                        egui::DragValue::new(&mut next.center[1])
                            .speed(0.005)
                            .prefix("y "),
                    )
                    .changed();
            });

            // A linear mask has no inside, so its size means nothing.
            if mask.shape != MaskShape::Linear {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(12.0);
                    ui.label("size");
                    // A mirror band runs the full width; only its height is its own.
                    if mask.shape != MaskShape::Mirror {
                        moved |= ui
                            .add(
                                egui::DragValue::new(&mut next.size[0])
                                    .speed(0.005)
                                    .prefix("w "),
                            )
                            .changed();
                    }
                    moved |= ui
                        .add(
                            egui::DragValue::new(&mut next.size[1])
                                .speed(0.005)
                                .prefix("h "),
                        )
                        .changed();
                });
            }

            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(theme::labeled(
                        "feather",
                        egui::Slider::new(&mut next.feather, 0.0..=1.0).fixed_decimals(2),
                    ))
                    .on_hover_text("How far the edge takes to fade out")
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(theme::labeled(
                        "angle",
                        egui::Slider::new(&mut next.rotation_degrees, -180.0..=180.0).suffix("°"),
                    ))
                    .changed();
                if ui
                    .checkbox(&mut next.invert, "invert")
                    .on_hover_text("Keep the outside instead")
                    .changed()
                {
                    moved = true;
                }
            });

            if moved {
                change = Some((ClipProperty::Mask(Some(next)), true));
            }
        }
    });

    // The chroma key. The three dials that decide what a screen is are kept
    // together, because none of them means anything without the others.
    effect_section(ui, Effect::ChromaKey, active[3], refold, |ui| {
        // By brightness: a logo on black, a title on white.
        {
            use bettercut_editor_core::timeline::LumaKey;
            let luma = editor.video_clip(clip).and_then(|clip| clip.luma_key);
            let mut on = luma.is_some();
            ui.horizontal_wrapped(|ui| {
                ui.add_space(4.0);
                if ui
                    .checkbox(&mut on, "by brightness")
                    .on_hover_text("Make the dark parts transparent — or the bright ones — for a logo on black or a title on white")
                    .changed()
                {
                    change = Some((ClipProperty::LumaKey(on.then(LumaKey::default)), false));
                }
            });
            if let Some(luma) = luma {
                let mut next = luma;
                let mut moved = false;
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(12.0);
                    let response = ui.add(theme::labeled(
                        "cut at",
                        egui::Slider::new(&mut next.threshold, 0.0..=1.0)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    ));
                    moved |= response.dragged();
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(12.0);
                    let response = ui.add(theme::labeled(
                        "softness",
                        egui::Slider::new(&mut next.softness, 0.0..=LumaKey::MAX_SOFTNESS)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    ));
                    moved |= response.dragged();
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(12.0);
                    ui.selectable_value(&mut next.keep_bright, true, "drop the dark");
                    ui.selectable_value(&mut next.keep_bright, false, "drop the bright");
                });
                if next != luma {
                    change = Some((ClipProperty::LumaKey(Some(next)), moved));
                }
            }
            ui.separator();
        }

        let key = editor.video_clip(clip).and_then(|clip| clip.chroma_key);
        let mut on = key.is_some();
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            if ui
                .checkbox(&mut on, "green screen")
                .on_hover_text("Make one colour transparent")
                .changed()
            {
                change = Some((
                    ClipProperty::ChromaKey(
                        on.then(bettercut_editor_core::timeline::ChromaKey::default),
                    ),
                    false,
                ));
            }
        });

        if let Some(key) = key {
            let mut next = key;
            let mut moved = false;
            let mut colour = [key.color[0], key.color[1], key.color[2]];

            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                ui.label("colour");
                if ui
                    .color_edit_button_rgb(&mut colour)
                    .on_hover_text("The screen's colour")
                    .changed()
                {
                    next.color = colour;
                    moved = true;
                }

                // The wheel is the fallback, not the instrument. No two green
                // screens are the same green once a light has been near them,
                // and the right answer is already on screen.
                let picking = state.picking_key == Some(clip);
                if ui
                    .add(egui::Button::selectable(picking, "pick"))
                    .on_hover_text("Click the screen in the picture above")
                    .clicked()
                {
                    state.picking_key = (!picking).then_some(clip);
                    // One tool at a time: crop mode also takes over the picture.
                    if state.picking_key.is_some() {
                        state.cropping = None;
                    }
                    state.needs_repaint = true;
                }
            });

            let max = bettercut_editor_core::timeline::ChromaKey::MAX_SPREAD;
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(theme::labeled(
                        "tolerance",
                        egui::Slider::new(&mut next.tolerance, 0.0..=max).fixed_decimals(2),
                    ))
                    .on_hover_text("How far from that colour still counts as screen")
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(theme::labeled(
                        "softness",
                        egui::Slider::new(&mut next.softness, 0.0..=max).fixed_decimals(2),
                    ))
                    .on_hover_text("How quickly the edge gives way — what saves hair")
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(theme::labeled(
                        "spill",
                        egui::Slider::new(&mut next.spill, 0.0..=1.0).fixed_decimals(2),
                    ))
                    .on_hover_text(
                        "How much of the screen's colour to take back out of the subject",
                    )
                    .changed();
            });

            if moved {
                // A drag on any of these is one gesture, like every other
                // slider here (§11).
                change = Some((ClipProperty::ChromaKey(Some(next)), true));
            }
        }
    });

    // RGB split and glitch: the break-up looks. Last, as the rarest.
    effect_section(ui, Effect::Glitch, active[4], refold, |ui| {
        let (split, glitch, pixelate) = editor.video_clip(clip).map_or((0.0, 0.0, 0.0), |clip| {
            (clip.rgb_split, clip.glitch, clip.pixelate)
        });
        let max = bettercut_editor_core::timeline::MAX_GLITCH;
        let (mut next_split, mut next_glitch, mut next_pixelate) = (split, glitch, pixelate);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let response = ui
                .add(theme::labeled(
                    "RGB split",
                    egui::Slider::new(&mut next_split, 0.0..=max).suffix("%"),
                ))
                .on_hover_text("Pull the red and blue apart, like a cheap lens or an old tape");
            if response.changed() {
                change = Some((ClipProperty::RgbSplit(next_split), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let response = ui
                .add(theme::labeled(
                    "glitch",
                    egui::Slider::new(&mut next_glitch, 0.0..=max).suffix("%"),
                ))
                .on_hover_text("Throw bands of the picture sideways, different ones every frame");
            if response.changed() {
                change = Some((ClipProperty::Glitch(next_glitch), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let response = ui
                .add(theme::labeled(
                    "pixelate",
                    egui::Slider::new(&mut next_pixelate, 0.0..=max).suffix("%"),
                ))
                .on_hover_text(
                    "Show the picture as coarse blocks. Add a mask to cover just a face or a plate",
                );
            if response.changed() {
                change = Some((ClipProperty::Pixelate(next_pixelate), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let mut next_zoom = editor.video_clip(clip).map_or(0.0, |clip| clip.zoom_blur);
            let response = ui
                .add(
                    theme::labeled("zoom blur", egui::Slider::new(&mut next_zoom, 0.0..=max)
                        .suffix("%")),
                )
                .on_hover_text("Streak the picture outwards from the middle, a rush forward. Good on a cut or a beat");
            if response.changed() {
                change = Some((ClipProperty::ZoomBlur(next_zoom), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let mut next_lens = editor.video_clip(clip).map_or(0.0, |clip| clip.lens);
            let response = ui
                .add(
                    theme::labeled("lens", egui::Slider::new(&mut next_lens, -1.0..=1.0)
                        .custom_formatter(|v, _| warmth_label(v, "barrel", "pincushion"))),
                )
                .on_hover_text("Straighten a wide lens's bowed lines, or bow them. Pushing out uncovers the corners; scale up a little to fill them");
            if response.changed() {
                change = Some((ClipProperty::Lens(next_lens), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let mut next_levels = editor
                .video_clip(clip)
                .map_or(0.0, |clip| clip.posterise)
                .round() as u8;
            let response = ui
                .add(
                    theme::labeled("posterise", egui::Slider::new(&mut next_levels, 0..=16)
                        .custom_formatter(|v, _| {
                            if v < 2.0 { "off".to_owned() } else { format!("{v:.0} levels") }
                        })),
                )
                .on_hover_text("Hold each colour channel to a few levels: a poster, a print, a game from years ago. Two is the harshest");
            if response.changed() {
                let levels = if next_levels < 2 { 0.0 } else { f32::from(next_levels) };
                change = Some((ClipProperty::Posterise(levels), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let mut next_smooth = editor.video_clip(clip).map_or(0.0, |clip| clip.smooth_skin);
            let response = ui
                .add(theme::labeled(
                    "smooth skin",
                    egui::Slider::new(&mut next_smooth, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                ))
                .on_hover_text(
                    "Soften skin — pores and blotches — while eyes, lips and hair keep their edges",
                );
            if response.changed() {
                change = Some((ClipProperty::SmoothSkin(next_smooth), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let mut next_glow = editor.video_clip(clip).map_or(0.0, |clip| clip.glow);
            let response = ui
                .add(theme::labeled(
                    "glow",
                    egui::Slider::new(&mut next_glow, 0.0..=max).suffix("%"),
                ))
                .on_hover_text("Let the bright parts bleed soft light around them, a dreamy bloom");
            if response.changed() {
                change = Some((ClipProperty::Glow(next_glow), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let mut next_film = editor.video_clip(clip).map_or(0.0, |clip| clip.old_film);
            let response = ui
                .add(theme::labeled(
                    "old film",
                    egui::Slider::new(&mut next_film, 0.0..=max).suffix("%"),
                ))
                .on_hover_text("Scratches, dust and a flickering exposure, like a worn old print");
            if response.changed() {
                change = Some((ClipProperty::OldFilm(next_film), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let mut next_leak = editor.video_clip(clip).map_or(0.0, |clip| clip.light_leak);
            let response = ui
                .add(theme::labeled(
                    "light leak",
                    egui::Slider::new(&mut next_leak, 0.0..=max).suffix("%"),
                ))
                .on_hover_text(
                    "A warm glow drifting across the frame, like light spilling into an old camera",
                );
            if response.changed() {
                change = Some((ClipProperty::LightLeak(next_leak), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let mut next_flare = editor.video_clip(clip).map_or(0.0, |clip| clip.lens_flare);
            let response = ui
                .add(theme::labeled(
                    "lens flare",
                    egui::Slider::new(&mut next_flare, 0.0..=max).suffix("%"),
                ))
                .on_hover_text(
                    "A bright light near the corner with its streak and reflections, as if shot into the sun",
                );
            if response.changed() {
                change = Some((ClipProperty::LensFlare(next_flare), response.dragged()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let mut next_pulse = editor.video_clip(clip).map_or(0.0, |clip| clip.beat_pulse);
            let response = ui
                .add(theme::labeled("beat pulse", egui::Slider::new(&mut next_pulse, 0.0..=max).suffix("%")))
                .on_hover_text("Punch in on every marker and ease back. Add markers on the beat first (M, or Detect Beats)");
            if response.changed() {
                change = Some((ClipProperty::BeatPulse(next_pulse), response.dragged()));
            }
        });
    });

    // Reflections: part of the shot, repeated and mirrored over the rest.
    effect_section(ui, Effect::Mirror, active[5], refold, |ui| {
        use bettercut_editor_core::timeline::Reflection;
        let current = editor
            .video_clip(clip)
            .map_or(Reflection::None, |clip| clip.reflection);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            for kind in Reflection::ALL {
                let hint = match kind {
                    Reflection::None => "The picture as shot",
                    Reflection::LeftRight => "The left half, with its mirror image on the right",
                    Reflection::TopBottom => "The top half, with its mirror image below",
                    Reflection::FourWay => "The top-left quarter, mirrored into all four corners",
                    Reflection::Kaleidoscope => "A wedge from the centre, mirrored round in six",
                };
                if ui
                    .selectable_label(current == kind, kind.label())
                    .on_hover_text(hint)
                    .clicked()
                    && current != kind
                {
                    change = Some((ClipProperty::Reflection(kind), false));
                }
            }
        });
    });

    if let Some((property, continuing)) = change {
        apply_clip_property(editor, state, clip, property, continuing);
    }
}

/// The colour clips the Add Colour menu offers; any of them can be recoloured
/// afterwards in the Inspector.
pub const COLOUR_CLIP_PRESETS: [(&str, bettercut_editor_core::media::Generated); 6] = {
    use bettercut_editor_core::media::Generated;
    [
        ("Black", Generated::solid([0, 0, 0])),
        ("White", Generated::solid([255, 255, 255])),
        ("Blue", Generated::solid([30, 110, 230])),
        (
            "Sunset",
            Generated::Colour {
                top: [255, 120, 60],
                bottom: [120, 40, 140],
            },
        ),
        (
            "Ocean",
            Generated::Colour {
                top: [40, 200, 220],
                bottom: [20, 50, 140],
            },
        ),
        (
            "Night",
            Generated::Colour {
                top: [40, 40, 70],
                bottom: [5, 5, 15],
            },
        ),
    ]
};

/// A colour clip's colours: one, or two for a gradient from top to bottom.
/// Nothing for a clip made from a file.
fn clip_fill(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::media::Generated;

    let Some((media, colour)) = editor.colour_of(clip) else {
        return;
    };
    let Generated::Colour { top, bottom } = colour else {
        return;
    };
    let mut top = top;
    let mut bottom = bottom;
    let mut gradient = top != bottom;
    let mut changed = false;

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label("Colour");
        changed |= crate::swatches::colour_edit(ui, state, &mut top, "");
        if gradient {
            ui.label("to");
            changed |= crate::swatches::colour_edit(ui, state, &mut bottom, "");
        }
        if ui
            .checkbox(&mut gradient, "Gradient")
            .on_hover_text("Fade from the first colour at the top to a second at the bottom")
            .changed()
        {
            changed = true;
            // Turning a gradient on starts from a darker shade of the colour,
            // so it shows at once; turning it off keeps the top colour.
            bottom = if gradient {
                top.map(|c| (c as f32 * 0.35) as u8)
            } else {
                top
            };
        }
    });
    if !gradient {
        bottom = top;
    }
    if changed {
        // A drag around the picker is one undo step: carried on while the
        // pointer is still down from the press that started it.
        let continuing = ui.input(|i| i.pointer.any_down() && !i.pointer.any_pressed());
        match editor.set_colour(media, Generated::Colour { top, bottom }, continuing) {
            Ok(_) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }
    ui.separator();
}

fn clip_video_properties(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
    look: VideoLook,
) {
    use bettercut_editor_core::ClipProperty;

    // Colour lives in its own tab now, so it is deliberately not read here.
    let VideoLook {
        opacity,
        transform,
        blur,
        ..
    } = look;

    ui.add_space(4.0);
    let mut change: Option<(ClipProperty, bool)> = None;
    let mut toggle: Option<ClipProperty> = None;
    let mut reset: Option<ClipProperty> = None;

    let mut value = opacity;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Opacity(opacity),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "opacity",
                egui::Slider::new(&mut value, 0.0..=1.0),
            ))
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Opacity(value), response.dragged()));
    }

    // One control for both axes: non-uniform scale is a distortion effect, not
    // something a user reaches for while cutting, and two boxes would imply it
    // is the normal case.
    let mut scale = transform.scale.x;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Scale {
            x: transform.scale.x,
            y: transform.scale.y,
        },
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "scale",
                egui::Slider::new(&mut scale, 0.05..=4.0).logarithmic(true),
            ))
        },
    );
    if response.changed() {
        change = Some((
            ClipProperty::Scale { x: scale, y: scale },
            response.dragged(),
        ));
    }

    // Fit leaves bars at the sides; fill crops to cover the frame. The two
    // together are what reframing landscape footage for a vertical edit needs,
    // and neither is obvious from a scale slider alone.
    if let Some((source_aspect, output_aspect)) = aspects(editor, look.media_id) {
        let fill = bettercut_editor_core::timeline::fill_scale(source_aspect, output_aspect);
        let near = |a: f32, b: f32| (a - b).abs() < 0.005;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            if ui
                .add(egui::Button::selectable(near(scale, 1.0), "Fit"))
                .on_hover_text(
                    "Show the whole picture, bars at the sides if it is a different shape",
                )
                .clicked()
            {
                change = Some((ClipProperty::Scale { x: 1.0, y: 1.0 }, false));
            }
            let same_shape = near(fill, 1.0);
            if ui
                .add_enabled(
                    !same_shape,
                    egui::Button::selectable(near(scale, fill), "Fill"),
                )
                .on_hover_text("Cover the frame, cropping what does not fit")
                .on_disabled_hover_text("This clip is already the shape of the frame")
                .clicked()
            {
                change = Some((ClipProperty::Scale { x: fill, y: fill }, false));
            }
        });
    }

    // What fills the frame when the shot does not (§36). Beside Fit, because
    // it is the answer to the bars Fit leaves.
    if let Some((source_aspect, output_aspect)) = aspects(editor, look.media_id)
        && bettercut_editor_core::timeline::fill_scale(source_aspect, output_aspect) > 1.001
    {
        let current = editor
            .video_clip(clip)
            .map(|clip| clip.backdrop)
            .unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            theme::row_label(ui, "behind");
            for option in bettercut_editor_core::timeline::Backdrop::ALL {
                if ui
                    .add(egui::Button::selectable(current == option, option.label()))
                    .on_hover_text(match option {
                        bettercut_editor_core::timeline::Backdrop::None => {
                            "Black bars where the shapes disagree"
                        }
                        bettercut_editor_core::timeline::Backdrop::Blur => {
                            "Fill the frame with a blurred copy of this shot"
                        }
                        bettercut_editor_core::timeline::Backdrop::Image(_) => "",
                    })
                    .clicked()
                    && current != option
                {
                    change = Some((ClipProperty::Backdrop(option), false));
                }
            }
            // A picture from the project: the ones already imported, since a
            // backdrop is chosen, not hunted for.
            let pictures: Vec<(MediaId, String)> = editor
                .project()
                .media
                .iter()
                .filter(|asset| asset.is_still() && !asset.missing && asset.id != look.media_id)
                .map(|asset| (asset.id, asset.file_name.clone()))
                .collect();
            let chosen = match current {
                bettercut_editor_core::timeline::Backdrop::Image(media) => pictures
                    .iter()
                    .find(|(id, _)| *id == media)
                    .map(|(_, name)| name.clone()),
                _ => None,
            };
            let response = ui.add_enabled_ui(!pictures.is_empty(), |ui| {
                ui.menu_button(chosen.as_deref().unwrap_or("Picture"), |ui| {
                    for (media, name) in &pictures {
                        if ui.button(name).clicked() {
                            change = Some((
                                ClipProperty::Backdrop(
                                    bettercut_editor_core::timeline::Backdrop::Image(*media),
                                ),
                                false,
                            ));
                        }
                    }
                })
            });
            if pictures.is_empty() {
                response
                    .response
                    .on_disabled_hover_text("Import a photo to use it behind the shot");
            } else {
                response
                    .response
                    .on_hover_text("Fill the frame with one of the project's photos");
            }
        });
    }

    // A slow zoom, written as keyframes (§24). Beside the scale it animates,
    // because that is what it changes — and what to adjust afterwards.
    let current = editor.movement_of(clip);
    let mut movement = None;
    ui.horizontal_wrapped(|ui| {
        theme::row_label(ui, "movement");
        for option in Movement::ALL {
            if ui
                .add(egui::Button::selectable(
                    current == Some(option),
                    option.label(),
                ))
                .on_hover_text(match option {
                    Movement::None => "No movement; any zoom or pan keyframes are removed",
                    Movement::ZoomIn => "Grow slowly across the clip",
                    Movement::ZoomOut => "Start close and pull back",
                    Movement::PanLeft => {
                        "Slide slowly to the left, a little enlarged so no edge shows"
                    }
                    Movement::PanRight => {
                        "Slide slowly to the right, a little enlarged so no edge shows"
                    }
                    Movement::PanUp => "Slide slowly upwards, a little enlarged so no edge shows",
                    Movement::PanDown => {
                        "Slide slowly downwards, a little enlarged so no edge shows"
                    }
                })
                .clicked()
            {
                movement = Some(option);
            }
        }
    });
    // How far it travels, and — for a montage — whether every other shot
    // turns the other way.
    let selected: Vec<bettercut_editor_core::foundation::ClipId> = state
        .selected_clips
        .iter()
        .copied()
        .filter(|selected| editor.video_clip(*selected).is_some())
        .collect();
    ui.horizontal_wrapped(|ui| {
        theme::row_label(ui, "travel");
        for option in bettercut_editor_core::timeline::MovementStrength::ALL {
            if ui
                .add(egui::Button::selectable(
                    state.movement_strength == option,
                    option.label(),
                ))
                .on_hover_text("How far the next movement moves")
                .clicked()
            {
                state.movement_strength = option;
            }
        }
        if selected.len() > 1 {
            ui.checkbox(&mut state.movement_alternates, "alternate")
                .on_hover_text(
                    "Turn every other shot the other way, so a montage does not read as a conveyor belt",
                );
        }
    });
    if selected.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("{} clips selected", selected.len()))
                    .small()
                    .color(theme::disabled()),
            );
        });
    }

    if let Some(movement) = movement {
        // More than one picture selected means the montage: every one of them
        // takes it, as a single undo step.
        let outcome = if selected.len() > 1 {
            editor
                .set_movement_on(
                    &selected,
                    movement,
                    state.movement_strength,
                    state.movement_alternates,
                )
                .map(|given| {
                    if given < selected.len() {
                        state.info(format!(
                            "{given} of {} clips took it; the rest are animated already",
                            selected.len()
                        ));
                    }
                })
        } else {
            editor.set_movement_at(clip, movement, state.movement_strength)
        };
        match outcome {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }

    // Camera shake, also written as keys: beside the movement because both are
    // generated motion, and both are adjusted afterwards in the same place.
    let shake = editor.shake_of(clip);
    let mut shake_choice: Option<Option<bettercut_editor_core::timeline::ShakeStrength>> = None;
    ui.horizontal_wrapped(|ui| {
        theme::row_label(ui, "shake");
        if ui
            .add(egui::Button::selectable(shake.is_none(), "None"))
            .on_hover_text("No shake; the shake's keyframes are removed")
            .clicked()
        {
            shake_choice = Some(None);
        }
        for strength in bettercut_editor_core::timeline::ShakeStrength::ALL {
            if ui
                .add(egui::Button::selectable(
                    shake == Some(strength),
                    strength.label(),
                ))
                .on_hover_text(strength.description())
                .clicked()
            {
                shake_choice = Some(Some(strength));
            }
        }
    });
    if let Some(choice) = shake_choice {
        match editor.set_shake(clip, choice) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }

    let position = ClipProperty::Position {
        x: transform.position.x,
        y: transform.position.y,
    };
    keyed_row(ui, &look, position, &mut toggle, &mut reset, |ui| {
        ui.label("position");
        let mut x = transform.position.x;
        let mut y = transform.position.y;
        // Normalized units: 1.0 is a whole frame width, so the useful range is
        // about ±1 and a coarse step would make centring impossible.
        let rx = ui.add(egui::DragValue::new(&mut x).speed(0.005).range(-2.0..=2.0));
        let ry = ui.add(egui::DragValue::new(&mut y).speed(0.005).range(-2.0..=2.0));
        if rx.changed() || ry.changed() {
            change = Some((
                ClipProperty::Position { x, y },
                rx.dragged() || ry.dragged(),
            ));
        }
    });

    // The pivot: where the picture turns and scales about, 0..1 across and
    // down it. The picture stays put as it moves (`editor_core::pivot`); the
    // reset is a property reset like any other, and does move it.
    let pivot = ClipProperty::Anchor {
        x: transform.anchor.x,
        y: transform.anchor.y,
    };
    let mut pivot_to: Option<(bettercut_editor_core::timeline::Vec2, bool)> = None;
    keyed_row(ui, &look, pivot, &mut toggle, &mut reset, |ui| {
        ui.label("pivot");
        let mut x = transform.anchor.x;
        let mut y = transform.anchor.y;
        let rx = ui.add(egui::DragValue::new(&mut x).speed(0.005).range(0.0..=1.0));
        let ry = ui.add(egui::DragValue::new(&mut y).speed(0.005).range(0.0..=1.0));
        if rx.changed() || ry.changed() {
            pivot_to = Some((
                bettercut_editor_core::timeline::Vec2::new(x, y),
                rx.dragged() || ry.dragged(),
            ));
        }
        rx | ry
    });
    if let Some((anchor, continuing)) = pivot_to
        && let Err(err) = editor.set_pivot_keeping_place(clip, anchor, continuing)
    {
        state.error(err.to_string());
    }

    let mut rotation = transform.rotation_degrees;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Rotation(transform.rotation_degrees),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "rotation",
                egui::Slider::new(&mut rotation, -180.0..=180.0),
            ))
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Rotation(rotation), response.dragged()));
    }

    // §22's crop, above the placement controls because it runs before them:
    // what is left after cropping is the picture the transform then places.
    let crop = editor
        .video_clip(clip)
        .map(|c| c.crop)
        .unwrap_or(bettercut_editor_core::timeline::Crop::NONE);
    let mut cropped = crop;
    let crop_response = keyed_row(
        ui,
        &look,
        ClipProperty::Crop(crop),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.vertical(|ui| {
                ui.label("crop");
                // Each edge on its own, in per cent of the source, which is how
                // a crop is stated everywhere else and what keeps it meaningful
                // when the proxy is swapped for the full-size media (§13).
                let mut changed = false;
                let mut dragging = false;
                for (label, edge) in [
                    ("left", &mut cropped.left),
                    ("right", &mut cropped.right),
                    ("top", &mut cropped.top),
                    ("bottom", &mut cropped.bottom),
                ] {
                    let mut percent = *edge * 100.0;
                    let response = ui.add(theme::labeled(
                        label,
                        egui::Slider::new(&mut percent, 0.0..=95.0).suffix("%"),
                    ));
                    if response.changed() {
                        *edge = percent / 100.0;
                        changed = true;
                        dragging |= response.dragged();
                    }
                }
                (changed, dragging)
            })
        },
    );
    let (crop_changed, crop_dragging) = crop_response.inner;
    if crop_changed {
        change = Some((ClipProperty::Crop(cropped), crop_dragging));
    }

    // A shape in one click: centred, full size, the edges above set to match.
    ui.horizontal_wrapped(|ui| {
        ui.add_space(4.0);
        let now = editor.crop_shape_of(clip);
        let uncropped = editor.video_clip(clip).is_some_and(|c| c.crop.is_none());
        let mut chosen = None;
        if ui
            .add(egui::Button::selectable(uncropped, "Full"))
            .on_hover_text("No crop: the whole picture")
            .clicked()
        {
            chosen = Some(None);
        }
        for (label, w, h) in bettercut_editor_core::crop_shape::CROP_SHAPES {
            if ui
                .add(egui::Button::selectable(now == Some((w, h)), label))
                .on_hover_text(format!("Crop the picture to {label}, keeping its middle"))
                .clicked()
            {
                chosen = Some(Some((w, h)));
            }
        }
        if let Some(shape) = chosen {
            match editor.crop_to_shape(clip, shape) {
                Ok(()) => state.needs_repaint = true,
                Err(err) => state.error(err.to_string()),
            }
        }
    });

    // Dragging the edges on the picture, which is how a crop is actually made;
    // the sliders above are for the exact number afterwards.
    ui.horizontal_wrapped(|ui| {
        ui.add_space(4.0);
        let cropping = state.cropping == Some(clip);
        if ui
            .add(egui::Button::selectable(
                cropping,
                if cropping {
                    "Done cropping"
                } else {
                    "Crop in preview"
                },
            ))
            .on_hover_text(
                "Drag the edges of the picture to crop it. Esc or this button \
                 when finished; the move and scale handles come back.",
            )
            .clicked()
        {
            state.cropping = (!cropping).then_some(clip);
            // One tool at a time: the eyedropper also takes over the picture.
            if state.cropping.is_some() {
                state.picking_key = None;
                state.corner_pin_mode = false;
            }
            state.needs_repaint = true;
        }

        // §45's corner pin: the same idea one step further — each corner on
        // its own, for setting this picture into a screen in the shot beneath.
        let pinning = state.corner_pin_mode;
        if ui
            .add(egui::Button::selectable(
                pinning,
                if pinning {
                    "Done pinning"
                } else {
                    "Corner pin"
                },
            ))
            .on_hover_text(
                "Drag the picture's four corners one at a time, to lay it on a \
                 screen or a wall in the shot beneath it.",
            )
            .clicked()
        {
            state.corner_pin_mode = !pinning;
            if state.corner_pin_mode {
                state.cropping = None;
                state.picking_key = None;
            }
            state.needs_repaint = true;
        }
        let pinned = editor
            .video_clip(clip)
            .is_some_and(|video| !video.corner_pin.is_none());
        if pinned
            && ui
                .button("Unpin")
                .on_hover_text("Put all four corners back")
                .clicked()
        {
            apply_clip_property(
                editor,
                state,
                clip,
                ClipProperty::CornerPin(bettercut_editor_core::timeline::CornerPin::NONE),
                false,
            );
        }
    });

    // Beside the rotation, because both answer "which way is this shot facing".
    // Toggles rather than a -1 in the scale: mirroring is not a size, and a
    // clip covers the same part of the frame either way.
    ui.horizontal_wrapped(|ui| {
        theme::row_label(ui, "mirror");
        for axis in bettercut_editor_core::timeline::FlipAxis::ALL {
            let on = axis.is_set(&transform);
            if ui
                .add(egui::Button::selectable(
                    on,
                    match axis {
                        bettercut_editor_core::timeline::FlipAxis::Horizontal => "Horizontal",
                        bettercut_editor_core::timeline::FlipAxis::Vertical => "Vertical",
                    },
                ))
                .on_hover_text(match axis {
                    bettercut_editor_core::timeline::FlipAxis::Horizontal => "Mirror left to right",
                    bettercut_editor_core::timeline::FlipAxis::Vertical => "Mirror top to bottom",
                })
                .clicked()
            {
                change = Some((ClipProperty::Flip { axis, on: !on }, false));
            }
        }
    });

    // One slider, so no header of its own — but it does not belong with the
    // colour group either: everything in there is free, and this is not.
    let mut amount = blur;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Blur(blur),
        &mut toggle,
        &mut reset,
        |ui| {
            ui.add(theme::labeled(
                "blur",
                egui::Slider::new(&mut amount, 0.0..=bettercut_editor_core::timeline::MAX_BLUR)
                    .suffix("%"),
            ))
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Blur(amount), response.dragged()));
    }
    // Tilt-shift: a sharp band the blur keeps off, for a miniature or a face
    // held clear in a soft street. Only worth showing with a blur to shape.
    if amount > 0.0 {
        let (mut band, mut centre) = editor
            .video_clip(clip)
            .map_or((0.0, 0.5), |c| (c.tilt_band, c.tilt_centre));
        let mut moved = false;
        let mut touched = false;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            let r = ui
                .add(
                    theme::labeled("sharp band", egui::Slider::new(&mut band, 0.0..=1.0)
                        .custom_formatter(|v, _| if v <= 0.0 { "off".to_owned() } else { format!("{:.0}%", v * 100.0) })),
                )
                .on_hover_text("Keep a band across the picture sharp, blurring more away from it: a tilt-shift miniature");
            touched |= r.changed();
            moved |= r.dragged();
        });
        if band > 0.0 {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(4.0);
                let r = ui.add(theme::labeled(
                    "band centre",
                    egui::Slider::new(&mut centre, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                ));
                touched |= r.changed();
                moved |= r.dragged();
            });
        }
        if touched {
            change = Some((ClipProperty::TiltShift { band, centre }, moved));
        }
    }
    if amount > 0.0 {
        // §45 rates blur Medium and §44 says to avoid expensive realtime
        // effects on weak hardware. Saying so where the slider is beats
        // leaving the user to wonder why playback got choppy.
        ui.label(
            egui::RichText::new("Blur costs more than the controls above; playback may drop.")
                .small()
                .color(theme::disabled()),
        );
    }

    // Beside the blur, as its opposite. Not keyed — a sharpen that eases in is
    // not something anyone asks for — so a plain row with its own reset.
    let sharpen = editor.video_clip(clip).map_or(0.0, |c| c.sharpen);
    let mut sharpened = sharpen;
    ui.horizontal_wrapped(|ui| {
        ui.add_space(18.0);
        let response = ui
            .add(theme::labeled(
                "sharpen",
                egui::Slider::new(
                    &mut sharpened,
                    0.0..=bettercut_editor_core::timeline::MAX_SHARPEN,
                )
                .suffix("%"),
            ))
            .on_hover_text("Bring out edges and fine detail, leaving flat areas as they are");
        if response.changed() {
            change = Some((ClipProperty::Sharpen(sharpened), response.dragged()));
        }
        let touched = !ClipProperty::Sharpen(sharpen).is_default();
        if ui
            .add_enabled(
                touched,
                egui::Button::new(egui::RichText::new("\u{21ba}").color(if touched {
                    theme::clip_text()
                } else {
                    theme::disabled()
                }))
                .frame(false)
                .min_size(egui::vec2(18.0, 18.0)),
            )
            .on_hover_text("Reset sharpen")
            .clicked()
        {
            reset = Some(ClipProperty::Sharpen(sharpen));
        }
    });

    apply_row_actions(editor, state, clip, change, toggle, reset);

    clip_border(ui, editor, state, clip);

    clip_shadow(ui, editor, state, clip);

    clip_lut(ui, editor, state, clip);

    clip_transition(ui, editor, state, clip);
}

/// Rounded corners and a border on one clip: the framed picture-in-picture
/// look. Both sliders are shares of the picture's shorter side, so the frame
/// keeps its look however big the clip is.
fn clip_border(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::timeline::{Border, MAX_BORDER_WIDTH};

    let Some(border) = editor.video_clip(clip).map(|c| c.border) else {
        return;
    };
    let mut next = border;
    let mut radius = border.radius * 100.0;
    let mut width = border.width / MAX_BORDER_WIDTH * 100.0;
    let mut dragging = false;
    let mut changed = false;

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.add_space(18.0);
        let response = ui
            .add(theme::labeled(
                "corners",
                egui::Slider::new(&mut radius, 0.0..=100.0).suffix("%"),
            ))
            .on_hover_text(
                "Round the corners of the picture; all the way makes a square clip a circle",
            );
        if response.changed() {
            next.radius = radius / 100.0;
            dragging |= response.dragged();
            changed = true;
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.add_space(18.0);
        let response = ui
            .add(theme::labeled(
                "border",
                egui::Slider::new(&mut width, 0.0..=100.0).suffix("%"),
            ))
            .on_hover_text("A solid border around the picture, following its corners");
        if response.changed() {
            next.width = width / 100.0 * MAX_BORDER_WIDTH;
            dragging |= response.dragged();
            changed = true;
        }
        if border.width > 0.0 {
            let mut colour = border.colour;
            if crate::swatches::colour_edit(ui, state, &mut colour, "Border colour") {
                next.colour = colour;
                dragging |= ui.input(|i| i.pointer.any_down() && !i.pointer.any_pressed());
                changed = true;
            }
        }
        if !border.is_none()
            && ui
                .add(
                    egui::Button::new(egui::RichText::new("\u{21ba}").color(theme::clip_text()))
                        .frame(false)
                        .min_size(egui::vec2(18.0, 18.0)),
                )
                .on_hover_text("Square corners, no border")
                .clicked()
        {
            next = Border::NONE;
            changed = true;
        }
    });
    if changed && next != border {
        apply_clip_property(
            editor,
            state,
            clip,
            bettercut_editor_core::ClipProperty::Border(next),
            dragging,
        );
    }
}

/// A drop shadow under one clip: how dark, how soft, how far and which way.
/// Only the darkness shows at first — the rest is already set to a soft
/// shadow falling down and to the right, so one slider gives a good one.
fn clip_shadow(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::timeline::{MAX_SHADOW_DISTANCE, MAX_SHADOW_SOFTNESS, Shadow};

    let Some(shadow) = editor.video_clip(clip).map(|c| c.shadow) else {
        return;
    };
    let mut next = shadow;
    let mut dragging = false;
    let mut changed = false;
    let mut row = |ui: &mut egui::Ui, value: &mut f32, range: f32, text: &str, hint: &str| {
        let mut percent = *value / range * 100.0;
        let response = ui
            .horizontal(|ui| {
                ui.add_space(18.0);
                ui.add(theme::labeled(
                    text,
                    egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%"),
                ))
                .on_hover_text(hint)
            })
            .inner;
        if response.changed() {
            *value = percent / 100.0 * range;
            dragging |= response.dragged();
            changed = true;
        }
    };

    row(
        ui,
        &mut next.opacity,
        1.0,
        "shadow",
        "A soft shadow under the picture, lifting it off what is behind",
    );
    if shadow.is_visible() {
        row(
            ui,
            &mut next.softness,
            MAX_SHADOW_SOFTNESS,
            "softness",
            "How blurred the shadow's edge is",
        );
        row(
            ui,
            &mut next.distance,
            MAX_SHADOW_DISTANCE,
            "distance",
            "How far the shadow falls from the picture",
        );
        ui.horizontal_wrapped(|ui| {
            ui.add_space(18.0);
            let response = ui
                .add(theme::labeled(
                    "angle",
                    egui::Slider::new(&mut next.angle_degrees, 0.0..=359.0).suffix("°"),
                ))
                .on_hover_text("Which way the shadow falls: 90° is straight down");
            if response.changed() {
                dragging |= response.dragged();
                changed = true;
            }
            let mut colour = shadow.colour;
            if crate::swatches::colour_edit(ui, state, &mut colour, "Shadow colour") {
                next.colour = colour;
                dragging |= ui.input(|i| i.pointer.any_down() && !i.pointer.any_pressed());
                changed = true;
            }
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("\u{21ba}").color(theme::clip_text()))
                        .frame(false)
                        .min_size(egui::vec2(18.0, 18.0)),
                )
                .on_hover_text("No shadow")
                .clicked()
            {
                next = Shadow::NONE;
                changed = true;
            }
        });
    }
    if changed && next != shadow {
        apply_clip_property(
            editor,
            state,
            clip,
            bettercut_editor_core::ClipProperty::Shadow(next),
            dragging,
        );
    }
}

/// A colour lookup table on one clip: which, and how much.
///
/// A combo of the project's tables with the import at the bottom of it, so
/// "use the LUT my colourist sent" is one place to look.
fn clip_lut(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::ClipProperty;
    use bettercut_editor_core::timeline::ClipLut;

    let Some(current) = editor.video_clip(clip).map(|c| c.lut) else {
        return;
    };
    let luts: Vec<(bettercut_editor_core::foundation::LutId, String, bool)> = editor
        .project()
        .luts
        .iter()
        .map(|lut| (lut.id, lut.name.clone(), lut.path.exists()))
        .collect();
    let mut chosen: Option<Option<ClipLut>> = None;
    let mut import = false;

    ui.horizontal_wrapped(|ui| {
        ui.add_space(18.0);
        ui.label("LUT");
        let selected = current
            .and_then(|lut| luts.iter().find(|(id, _, _)| *id == lut.lut))
            .map_or_else(
                || "None".to_owned(),
                |(_, name, there)| {
                    if *there {
                        name.clone()
                    } else {
                        format!("{name} (file missing)")
                    }
                },
            );
        egui::ComboBox::from_id_salt(("clip lut", clip))
            .selected_text(selected)
            .width(180.0)
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), "None").clicked() {
                    chosen = Some(None);
                }
                for (id, name, there) in &luts {
                    let label = if *there {
                        name.clone()
                    } else {
                        format!("{name} (file missing)")
                    };
                    if ui
                        .selectable_label(current.is_some_and(|lut| lut.lut == *id), label)
                        .clicked()
                    {
                        let strength = current.map_or(1.0, |lut| lut.strength);
                        chosen = Some(Some(ClipLut { lut: *id, strength }));
                    }
                }
                ui.separator();
                if ui
                    .button("Import .cube…")
                    .on_hover_text("A colour grade from Resolve, Premiere or a LUT pack")
                    .clicked()
                {
                    import = true;
                }
            });
    });

    let mut dragging = false;
    if let Some(lut) = current {
        let mut strength = lut.strength * 100.0;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(18.0);
            let response = ui.add(theme::labeled(
                "LUT strength",
                egui::Slider::new(&mut strength, 0.0..=100.0).suffix("%"),
            ));
            if response.changed() {
                dragging = response.dragged();
                chosen = Some(Some(ClipLut {
                    lut: lut.lut,
                    strength: strength / 100.0,
                }));
            }
        });
    }

    if import
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("LUT", &["cube"])
            .pick_file()
    {
        match editor.import_lut(&path) {
            Ok(id) => {
                chosen = Some(Some(ClipLut::new(id)));
                state.info("LUT imported");
            }
            Err(err) => state.error(err.to_string()),
        }
    }

    if let Some(lut) = chosen {
        match editor.set_clip_property(clip, ClipProperty::Lut(lut), dragging) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// The transition on the cut at the end of this clip (§25).
///
/// Only shown when there is a cut there. A control that is always visible and
/// almost always refuses would be worse than one that appears when it applies.
fn clip_transition(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::timeline::{MIN_TRANSITION, TransitionKind};

    let existing = editor.video_clip(clip).and_then(|c| c.transition_out);
    // Room for *any* kind: a cut with no handles still takes a fade through
    // black, and hiding the whole section there would hide the one that works.
    let room: Vec<_> = TransitionKind::ALL
        .into_iter()
        .map(|kind| (kind, editor.transition_room(clip, kind)))
        .collect();
    if room.iter().all(|(_, r)| r.is_none()) {
        return; // no clip straight after this one — nothing to fade into
    }

    ui.add_space(10.0);
    ui.separator();
    ui.label(egui::RichText::new("Transition out").strong());
    ui.label(
        egui::RichText::new("Applies to the cut between this clip and the next.")
            .small()
            .color(theme::disabled()),
    );
    ui.add_space(2.0);

    let mut chosen: Option<Option<TransitionKind>> = None;
    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(existing.is_none(), "None").clicked() {
            chosen = Some(None);
        }
        for (kind, available) in &room {
            let usable = available.is_some_and(|r| r >= MIN_TRANSITION);
            let selected = existing.is_some_and(|t| t.kind == *kind);
            let response = ui
                .add_enabled(usable, egui::Button::selectable(selected, kind.label()))
                .on_hover_text(kind.description())
                .on_disabled_hover_text("Not enough spare footage either side of the cut.");
            if response.clicked() {
                chosen = Some(Some(*kind));
            }
        }
    });

    if let Some(transition) = existing {
        // The slider stops where the media does, so the length shown is always
        // one that will actually render (§25).
        let limit = room
            .iter()
            .find(|(kind, _)| *kind == transition.kind)
            .and_then(|(_, r)| *r)
            .unwrap_or(MIN_TRANSITION);
        let mut seconds = transition.duration.ticks() as f64 / 960_000.0;
        let response = ui.add(theme::labeled(
            "seconds",
            egui::Slider::new(
                &mut seconds,
                MIN_TRANSITION.ticks() as f64 / 960_000.0..=limit.ticks() as f64 / 960_000.0,
            )
            .fixed_decimals(2),
        ));
        if response.changed() {
            // Back to ticks immediately: §74 keeps positions integral, and the
            // slider's float is only how the control reports itself.
            let ticks = bettercut_editor_core::foundation::TimelineTime::from_ticks(
                (seconds * 960_000.0).round() as i64,
            );
            if let Err(err) = editor.set_transition_duration(clip, ticks) {
                state.error(err.to_string());
            } else {
                state.needs_repaint = true;
            }
        }
    }

    match chosen {
        Some(Some(kind)) => match editor.set_transition(clip, kind) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        },
        Some(None) => match editor.remove_transition(clip) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        },
        None => {}
    }
}

/// How fast one clip plays.
///
/// Presets first and the slider second, because "make it 2×" is the whole of
/// what most people want and hunting for it on a slider is worse than a button
/// that says so. The slider is there for the rest.
fn clip_speed(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::foundation::Rational;

    let Some(existing) = editor.video_clip(clip) else {
        return;
    };
    let speed = existing.speed;
    let length = existing.timeline.duration();

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Speed").strong());
        ui.label(
            egui::RichText::new(format!("{:.2}×", speed.as_f64()))
                .monospace()
                .color(theme::keyframe()),
        );
    });
    ui.label(
        egui::RichText::new(format!(
            "Plays {} of footage in {}",
            TimelineTime::from_ticks(existing.source.duration().ticks()).format_timecode(),
            length.format_timecode()
        ))
        .small()
        .color(theme::disabled()),
    );

    // Fitting the shot to a length rather than choosing a number: "end it
    // here" is the question people actually have (`editor_core::rate_stretch`).
    ui.add_space(6.0);
    let playhead = editor.playhead();
    let can_stretch = editor
        .rate_stretch_speed(clip, playhead)
        .filter(|_| playhead > existing.timeline.start);
    ui.horizontal_wrapped(|ui| {
        let response = ui
            .add_enabled(
                can_stretch.is_some(),
                egui::Button::new("Stretch to playhead"),
            )
            .on_hover_text(
                "Re-time the shot so it ends at the playhead, keeping all of the footage it plays",
            );
        if let Some(speed) = can_stretch {
            ui.label(
                egui::RichText::new(format!("would be {:.2}\u{d7}", speed.as_f64()))
                    .small()
                    .color(theme::disabled()),
            );
        } else {
            ui.label(
                egui::RichText::new("put the playhead after the clip's start")
                    .small()
                    .color(theme::disabled()),
            );
        }
        if response.clicked() {
            match editor.rate_stretch(clip, playhead, false) {
                Ok(speed) => {
                    state.info(format!("Stretched to {:.2}\u{d7}", speed.as_f64()));
                    state.needs_repaint = true;
                }
                Err(err) => state.error(err.to_string()),
            }
        }
    });

    ui.add_space(6.0);
    let mut chosen: Option<Rational> = None;

    // Exact ratios, not decimals: 1/3 is a real speed and 0.33 is not the same
    // number. Everything downstream is integer arithmetic (§74).
    const PRESETS: [(&str, i64, i64); 6] = [
        ("0.25×", 1, 4),
        ("0.5×", 1, 2),
        ("1×", 1, 1),
        ("1.5×", 3, 2),
        ("2×", 2, 1),
        ("4×", 4, 1),
    ];

    ui.horizontal_wrapped(|ui| {
        let reversed = editor.is_reversed(clip);
        if ui
            .add(egui::Button::selectable(reversed, "Reverse"))
            .on_hover_text("Play the clip backwards, with its sound")
            .clicked()
        {
            match editor.set_reversed(clip, !reversed) {
                Ok(()) => state.needs_repaint = true,
                Err(err) => state.error(err.to_string()),
            }
        }
        for (label, num, den) in PRESETS {
            let preset = Rational::new(num, den).unwrap_or(Rational::ONE);
            if ui
                .add(egui::Button::selectable(speed == preset, label))
                .clicked()
            {
                chosen = Some(preset);
            }
        }
    });

    // The slider works in hundredths and is converted to an exact ratio, so a
    // dragged 1.75 is stored as 7/4 rather than as a float that has to be
    // multiplied into every position for the life of the project.
    let mut factor = speed.as_f64();
    let response = ui.add(theme::labeled(
        "rate",
        egui::Slider::new(
            &mut factor,
            bettercut_editor_core::timeline::MIN_SPEED.as_f64()
                ..=bettercut_editor_core::timeline::MAX_SPEED.as_f64(),
        )
        .logarithmic(true)
        .fixed_decimals(2)
        .suffix("×"),
    ));
    let dragging = response.dragged();
    if response.changed() {
        chosen = Rational::new((factor * 100.0).round() as i64, 100);
    }

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(
            "Clips after this one on the same track move to keep the cut tight. \
             Other tracks stay where they are.",
        )
        .small()
        .color(theme::disabled()),
    );

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Ramp").strong())
        .on_hover_text("Speed up and slow down across the clip");
    ui.horizontal_wrapped(|ui| {
        speed_ramp_buttons(ui, editor, state, clip);
    });
    ui.label(
        egui::RichText::new("A ramp cuts the clip into short pieces, each at its own speed.")
            .small()
            .color(theme::disabled()),
    );

    speed_curve_editor(ui, editor, state, clip);

    // Slowed footage glides between its frames instead of repeating them.
    if let Some(video) = editor.video_clip(clip) {
        let mut smooth = video.smooth_motion;
        let slowed = video.speed.num() < video.speed.den();
        if ui
            .add_enabled(
                slowed,
                egui::Checkbox::new(&mut smooth, "Smooth slow motion"),
            )
            .on_hover_text(
                "Blend each frame into the next so slowed footage glides instead of stepping",
            )
            .on_disabled_hover_text("For clips slowed below normal speed")
            .changed()
        {
            apply_clip_property(
                editor,
                state,
                clip,
                bettercut_editor_core::ClipProperty::SmoothMotion(smooth),
                false,
            );
        }
    }

    // The sound this clip carries, whether the clip is the picture or the
    // sound itself: re-timing moves the pitch with the speed unless it is
    // held.
    let sounds: Vec<bettercut_editor_core::foundation::ClipId> = editor
        .linked_with(clip)
        .into_iter()
        .filter(|id| editor.audio_clip(*id).is_some())
        .collect();
    if let Some(first) = sounds.first().copied() {
        let mut keep = editor
            .audio_clip(first)
            .is_some_and(|audio| audio.keep_pitch);
        if ui
            .checkbox(&mut keep, "Keep pitch")
            .on_hover_text(
                "Hold the voice where it is when the speed changes, instead of letting it rise and fall with it",
            )
            .changed()
        {
            for sound in &sounds {
                apply_clip_property(
                    editor,
                    state,
                    *sound,
                    bettercut_editor_core::ClipProperty::KeepPitch(keep),
                    false,
                );
            }
        }
    }

    if let Some(speed) = chosen {
        match editor.set_clip_speed(clip, speed, dragging) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// A speed curve drawn by hand: faster where the line is high, slower where it
/// is low, across the clip from left to right.
///
/// The presets above are five shapes; this is the sixth. It becomes the same
/// thing in the end — a run of pieces, each at its own exact speed — so the
/// curve is a way of *writing* a ramp rather than a second kind of re-timing.
fn speed_curve_editor(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::speed_curve::{MAX_PIECES, MAX_SPEED, MIN_PIECES, MIN_SPEED};

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Curve").strong())
        .on_hover_text("Draw the speed across the clip");

    let height = 110.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4, theme::timeline_background());

    let inner = rect.shrink(8.0);
    let x_of = |at: f32| inner.left() + at.clamp(0.0, 1.0) * inner.width();
    let at_of = |x: f32| ((x - inner.left()) / inner.width().max(1.0)).clamp(0.0, 1.0);
    // Speeds are read on a log scale: half speed and double speed are the same
    // distance from normal, which is how they feel.
    let span = MAX_SPEED.ln() - MIN_SPEED.ln();
    let y_of = |speed: f32| {
        let through = (speed.clamp(MIN_SPEED, MAX_SPEED).ln() - MIN_SPEED.ln()) / span;
        inner.bottom() - through * inner.height()
    };
    let speed_of = |y: f32| {
        let through = ((inner.bottom() - y) / inner.height().max(1.0)).clamp(0.0, 1.0);
        (MIN_SPEED.ln() + through * span).exp()
    };

    // Normal speed, to read the shape against.
    let normal = y_of(1.0);
    painter.line_segment(
        [
            egui::pos2(inner.left(), normal),
            egui::pos2(inner.right(), normal),
        ],
        egui::Stroke::new(1.0, theme::grid_line()),
    );

    let curve = state.speed_curve.clone();
    let line: Vec<egui::Pos2> = (0..=64)
        .map(|step| {
            let at = step as f32 / 64.0;
            egui::pos2(x_of(at), y_of(curve.speed_at(at)))
        })
        .collect();
    painter.add(egui::Shape::line(
        line,
        egui::Stroke::new(1.5, theme::selection()),
    ));
    for (at, speed) in curve.points() {
        painter.circle_filled(egui::pos2(x_of(*at), y_of(*speed)), 4.0, theme::selection());
    }
    painter.text(
        egui::pos2(inner.left() + 2.0, inner.top()),
        egui::Align2::LEFT_TOP,
        format!("{MAX_SPEED:.0}×"),
        egui::FontId::proportional(10.0),
        theme::disabled(),
    );
    painter.text(
        egui::pos2(inner.left() + 2.0, inner.bottom()),
        egui::Align2::LEFT_BOTTOM,
        format!("{MIN_SPEED:.1}×"),
        egui::FontId::proportional(10.0),
        theme::disabled(),
    );

    let pointer = response.interact_pointer_pos().or_else(|| {
        ui.ctx()
            .pointer_latest_pos()
            .filter(|pos| rect.contains(*pos))
    });
    let nearest = |pos: egui::Pos2| -> Option<usize> {
        curve
            .points()
            .iter()
            .enumerate()
            .map(|(index, (at, speed))| (egui::pos2(x_of(*at), y_of(*speed)).distance(pos), index))
            .filter(|(distance, _)| *distance <= 9.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, index)| index)
    };

    if response.drag_started()
        && let Some(pos) = pointer
    {
        state.speed_curve_drag = nearest(pos);
    }
    if response.dragged()
        && let Some(pos) = pointer
        && let Some(index) = state.speed_curve_drag
    {
        state
            .speed_curve
            .move_point(index, at_of(pos.x), speed_of(pos.y));
        state.needs_repaint = true;
    }
    if response.drag_stopped() {
        state.speed_curve_drag = None;
    }
    if response.double_clicked()
        && let Some(pos) = pointer
    {
        match nearest(pos) {
            Some(index) => state.speed_curve.remove_point(index),
            None => {
                let index = state.speed_curve.add_point(at_of(pos.x));
                state
                    .speed_curve
                    .move_point(index, at_of(pos.x), speed_of(pos.y));
            }
        }
        state.needs_repaint = true;
    }

    ui.horizontal_wrapped(|ui| {
        let mut pieces = state.speed_curve_pieces;
        if ui
            .add(theme::labeled(
                "pieces",
                egui::Slider::new(&mut pieces, MIN_PIECES..=MAX_PIECES).integer(),
            ))
            .on_hover_text("How many steps the curve is cut into")
            .changed()
        {
            state.speed_curve_pieces = pieces;
        }
        let flat = state.speed_curve.is_flat();
        if ui
            .add_enabled(!flat, egui::Button::new("Apply curve"))
            .on_hover_text("Cut the clip into pieces and give each the speed the curve asks for")
            .on_disabled_hover_text("Drag the line first — a flat curve changes nothing")
            .clicked()
        {
            let factors = state.speed_curve.factors(state.speed_curve_pieces);
            match editor.apply_speed_factors(clip, &factors, "Speed Curve") {
                Ok(pieces) => {
                    if let Some(first) = pieces.first() {
                        state.select_only(*first);
                    }
                    state.info(format!("Speed curve: {} pieces", pieces.len()));
                }
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
        }
        if ui
            .button("Flat")
            .on_hover_text("Back to normal speed all the way across")
            .clicked()
        {
            state.speed_curve = bettercut_editor_core::SpeedCurve::default();
            state.needs_repaint = true;
        }
    });
    ui.label(
        egui::RichText::new("Drag a point; double-click to add one, or to remove one.")
            .small()
            .color(theme::disabled()),
    );
}

/// One button per speed ramp preset. Shared by the Inspector and the clip's
/// right-click menu, so the two never offer different curves. True when one
/// was clicked, so a menu knows to close.
pub fn speed_ramp_buttons(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) -> bool {
    let mut chosen = false;
    for ramp in bettercut_editor_core::SpeedRamp::ALL {
        if ui
            .button(ramp.label())
            .on_hover_text(ramp.description())
            .clicked()
        {
            chosen = true;
            match editor.apply_speed_ramp(clip, ramp) {
                Ok(pieces) => {
                    // The clip it was is gone; the first piece keeps the
                    // Inspector open on the ramp.
                    if let Some(first) = pieces.first() {
                        state.select_only(*first);
                    }
                    state.info(format!(
                        "{} ramp: {} pieces, each at its own speed",
                        ramp.label(),
                        pieces.len()
                    ));
                }
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
        }
    }
    chosen
}

/// Everything about one text overlay (§26).
///
/// The style is dispatched as a whole rather than field by field, because that
/// is how the model holds it and how the rasterizer consumes it. A per-field
/// command would buy a finer undo history for a control nobody adjusts one
/// field at a time — and would multiply the number of commands by twelve.
/// An adjustment clip's controls: the grade it applies, and how strongly.
///
/// The same colour controls, the same blur and the same looks as a clip's own,
/// because an adjustment is those controls applied to a stretch of the edit
/// instead of to one shot — and learning a second set of names for the same
/// grade would be the interface's fault, not the user's.
fn adjustment_properties(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    let Some(existing) = editor.adjustment_clip(clip) else {
        return;
    };
    let range = existing.timeline;
    let mut look = existing.look;

    ui.label(egui::RichText::new("Adjustment").strong());
    ui.monospace(format!("start     {}", range.start.format_timecode()));
    ui.monospace(format!("duration  {}", range.duration().format_timecode()));
    ui.label(
        egui::RichText::new(
            "Grades every picture beneath it for as long as it runs. Titles on \
             top are left alone.",
        )
        .small()
        .color(theme::disabled()),
    );
    ui.add_space(6.0);

    // Collected while drawing and dispatched afterwards, as every panel here
    // does: the controls read the project and the change borrows it mutably.
    // `Some(dragging)` when something changed this frame.
    let mut changed: Option<bool> = None;
    let mut note = |response: egui::Response| {
        if response.changed() {
            changed = Some(response.dragged());
        }
    };

    ui.label("colour");
    note(ui.add(theme::labeled(
        "brightness",
        egui::Slider::new(&mut look.color.brightness, 0.0..=2.0),
    )));
    note(ui.add(theme::labeled(
        "contrast",
        egui::Slider::new(&mut look.color.contrast, 0.0..=2.0),
    )));
    note(ui.add(theme::labeled(
        "saturation",
        egui::Slider::new(&mut look.color.saturation, 0.0..=2.0),
    )));
    note(
        ui.add(theme::labeled(
            "temperature",
            egui::Slider::new(&mut look.color.temperature, -1.0..=1.0)
                .custom_formatter(|v, _| warmth_label(v, "cool", "warm")),
        )),
    );
    note(
        ui.add(theme::labeled(
            "tint",
            egui::Slider::new(&mut look.color.tint, -1.0..=1.0)
                .custom_formatter(|v, _| warmth_label(v, "green", "magenta")),
        )),
    );

    ui.add_space(4.0);
    note(
        ui.add(theme::labeled(
            "blur",
            egui::Slider::new(
                &mut look.blur,
                0.0..=bettercut_editor_core::timeline::MAX_BLUR,
            )
            .suffix("%"),
        )),
    );
    let mut vignette = look.vignette * 100.0;
    let response = ui
        .add(theme::labeled(
            "vignette",
            egui::Slider::new(&mut vignette, 0.0..=100.0).suffix("%"),
        ))
        .on_hover_text("Darken the edges of the frame while this adjustment runs");
    if response.changed() {
        look.vignette = vignette / 100.0;
    }
    note(response);
    let mut grain = look.grain * 100.0;
    let response = ui
        .add(theme::labeled(
            "grain",
            egui::Slider::new(&mut grain, 0.0..=100.0).suffix("%"),
        ))
        .on_hover_text("Film grain over the frame while this adjustment runs");
    if response.changed() {
        look.grain = grain / 100.0;
    }
    note(response);
    // As a percentage, because "how much of this grade" is read as a share.
    let mut strength = look.strength * 100.0;
    let response = ui.add(theme::labeled(
        "strength",
        egui::Slider::new(&mut strength, 0.0..=100.0).suffix("%"),
    ));
    if response.changed() {
        look.strength = strength / 100.0;
        changed = Some(response.dragged());
    }

    // The same one-click grades a clip offers, onto the colour only — the blur
    // and strength are left as they were set.
    ui.add_space(4.0);
    ui.label("looks");
    let current = look_of(look.color).map(|(name, _)| name);
    ui.horizontal_wrapped(|ui| {
        for (name, preset) in LOOKS {
            let on = match current {
                Some(found) => found == name,
                None => name == "None" && look.color.is_identity(),
            };
            if ui.add(egui::Button::selectable(on, name)).clicked() {
                look.color = preset;
                changed = Some(false);
            }
        }
    });

    ui.add_space(8.0);
    let mut remove = false;
    if ui
        .button("Remove adjustment")
        .on_hover_text("Take this adjustment off the timeline")
        .clicked()
    {
        remove = true;
    }

    if let Some(dragging) = changed
        && let Err(err) = editor.set_adjustment_look(clip, look, dragging)
    {
        state.error(err.to_string());
    }
    if remove {
        match editor.remove_adjustment(clip) {
            Ok(()) => state.clear_selection(),
            Err(err) => state.error(err.to_string()),
        }
    }
    if changed.is_some() || remove {
        state.needs_repaint = true;
    }
}

fn text_properties(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::TextProperty;
    use bettercut_editor_core::text::{
        Alignment, Background, FontFamily, FontWeight, Shadow, Stroke,
    };

    let Some(existing) = editor.text_clip(clip) else {
        return;
    };
    let range = existing.timeline;
    let mut text = existing.text.clone();
    let mut style = existing.style.clone();
    let transform = existing.transform;
    let opacity = existing.opacity;
    let smearing = existing.motion_blur;
    let mut animation = existing.animation;
    let shape = existing.shape.clone();
    let counter = existing.counter;
    let highlight = existing.highlight;

    ui.monospace(format!("start     {}", range.start.format_timecode()));
    ui.monospace(format!("duration  {}", range.duration().format_timecode()));
    ui.add_space(6.0);

    // Collected during the draw and dispatched after it, for the same reason
    // every other panel here does: the controls read the project while drawing
    // and each of these borrows it mutably.
    let mut change: Option<(TextProperty, bool)> = None;
    let mut look: Option<bettercut_editor_core::text::TitleLook> = None;
    let mut remove = false;

    if let Some(mut shape) = shape {
        // A shape has no words, font or looks: its own controls instead, and
        // the title's placement and animation below as usual.
        ui.label(egui::RichText::new("Shape").strong());
        let mut reshaped = false;
        let mut dragging = false;
        ui.horizontal_wrapped(|ui| {
            for kind in bettercut_editor_core::text::ShapeKind::ALL {
                if ui
                    .add(egui::Button::selectable(shape.kind == kind, kind.label()))
                    .clicked()
                    && shape.kind != kind
                {
                    shape.kind = kind;
                    reshaped = true;
                }
            }
        });
        let mut size = |ui: &mut egui::Ui, value: &mut f32, label: &str| {
            let response = ui.add(theme::labeled(
                label,
                egui::Slider::new(value, 8.0..=3840.0)
                    .logarithmic(true)
                    .suffix(" px"),
            ));
            if response.changed() {
                dragging |= response.dragged();
                reshaped = true;
            }
        };
        size(ui, &mut shape.width, "width");
        size(ui, &mut shape.height, "height");
        if matches!(
            shape.kind,
            bettercut_editor_core::text::ShapeKind::RoundedRectangle
                | bettercut_editor_core::text::ShapeKind::SpeechBubble
        ) {
            let response = ui.add(theme::labeled(
                "corners",
                egui::Slider::new(&mut shape.corner_radius, 0.0..=400.0),
            ));
            if response.changed() {
                dragging |= response.dragged();
                reshaped = true;
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("fill");
            if colour_button(ui, &mut shape.fill) {
                reshaped = true;
            }
        });
        let mut outlined = shape.outline.is_some();
        if ui.checkbox(&mut outlined, "outline").changed() {
            shape.outline = outlined.then_some(bettercut_editor_core::text::Stroke {
                width: 8.0,
                color: bettercut_editor_core::text::Rgba::BLACK,
            });
            reshaped = true;
        }
        if let Some(outline) = &mut shape.outline {
            ui.horizontal_wrapped(|ui| {
                if colour_button(ui, &mut outline.color) {
                    reshaped = true;
                }
                let response = ui.add(theme::labeled(
                    "width",
                    egui::Slider::new(&mut outline.width, 1.0..=120.0),
                ));
                if response.changed() {
                    dragging |= response.dragged();
                    reshaped = true;
                }
            });
        }
        if reshaped {
            change = Some((TextProperty::Shape(Some(shape)), dragging));
        }
    } else {
        if let Some(mut counter) = counter {
            // A timer's words are its number: its own controls instead of the
            // text box, and the title's style, placement and animation below.
            ui.label(egui::RichText::new("Timer").strong());
            let before = counter;
            let mut dragging = false;
            ui.horizontal_wrapped(|ui| {
                for direction in bettercut_editor_core::timeline::CountDirection::ALL {
                    if ui
                        .selectable_label(counter.direction == direction, direction.label())
                        .clicked()
                    {
                        counter.direction = direction;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("starts at");
                let mut seconds = counter.from.ticks() as f64
                    / bettercut_editor_core::foundation::TICKS_PER_SECOND as f64;
                let response = ui.add(
                    egui::DragValue::new(&mut seconds)
                        .range(0.0..=36_000.0)
                        .speed(0.1)
                        .suffix(" s"),
                );
                if response.changed() {
                    dragging = response.dragged();
                    counter.from = bettercut_editor_core::foundation::TimelineTime::from_ticks(
                        (seconds * bettercut_editor_core::foundation::TICKS_PER_SECOND as f64)
                            .round() as i64,
                    );
                }
                // The usual countdown ends exactly as the clip does.
                if ui
                    .button("Match length")
                    .on_hover_text(
                        "Start the count at the clip's length, so it reaches zero as it ends",
                    )
                    .clicked()
                {
                    counter.from = range.duration();
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("shows");
                for format in bettercut_editor_core::timeline::CountFormat::ALL {
                    if ui
                        .selectable_label(counter.format == format, format.label())
                        .clicked()
                    {
                        counter.format = format;
                    }
                }
            });
            if counter != before {
                change = Some((TextProperty::Counter(Some(counter)), dragging));
            }
            if ui
                .small_button("Show text instead")
                .on_hover_text("Turn this timer back into an ordinary title")
                .clicked()
            {
                change = Some((TextProperty::Counter(None), false));
            }
        } else {
            ui.label(egui::RichText::new("Text").strong());
            let response = ui.add(
                egui::TextEdit::multiline(&mut text)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text("Type something"),
            );
            if response.changed() {
                // Typing is a gesture like a drag: one undo step for the sentence, not
                // one per character.
                change = Some((TextProperty::Content(text.clone()), true));
            }

            // Karaoke-style: each word lit as it comes, timed across the clip.
            ui.horizontal_wrapped(|ui| {
                let mut on = highlight.is_some();
                if ui
                    .checkbox(&mut on, "light each word")
                    .on_hover_text(
                        "Colour each word in turn across the clip, the way karaoke captions do",
                    )
                    .changed()
                {
                    change = Some((
                        TextProperty::Highlight(on.then_some(Editor::DEFAULT_HIGHLIGHT)),
                        false,
                    ));
                }
                if let Some(mut colour) = highlight
                    && colour_button(ui, &mut colour)
                {
                    change = Some((TextProperty::Highlight(Some(colour)), false));
                }
            });
        }

        // A whole look in one click: the style and where it sits, which for a lower
        // third is half of what it is.
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Look").strong());
            for option in bettercut_editor_core::text::TitleLook::ALL {
                let selected = bettercut_editor_core::text::TextStyle::title(option) == style;
                if ui
                    .selectable_label(selected, option.label())
                    .on_hover_text(match option {
                        bettercut_editor_core::text::TitleLook::Headline => {
                            "Big and bold, near the middle"
                        }
                        bettercut_editor_core::text::TitleLook::LowerThird => {
                            "A boxed strip low on the left, for a name or a place"
                        }
                        bettercut_editor_core::text::TitleLook::Quote => {
                            "Light serif with room to breathe"
                        }
                        bettercut_editor_core::text::TitleLook::Typewriter => {
                            "Monospaced and spaced out — pairs with the typewriter entrance"
                        }
                    })
                    .clicked()
                    && !selected
                {
                    look = Some(option);
                }
            }
        });

        let mut restyled = false;

        // Colour and legibility in one click, keeping the size and place the title
        // already has — the half of a look people change most often.
        ui.add_space(4.0);
        let current_preset = bettercut_editor_core::text::TextPreset::of(&style);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Style").strong());
            for preset in bettercut_editor_core::text::TextPreset::ALL {
                if ui
                    .add(egui::Button::selectable(
                        current_preset == Some(preset),
                        preset.label(),
                    ))
                    .on_hover_text(preset.description())
                    .clicked()
                    && current_preset != Some(preset)
                {
                    style = preset.applied_to(&style);
                    restyled = true;
                }
            }
        });

        // The user's own styles, under the built-in ones: the way *this* video
        // titles things, so the twentieth caption matches the first.
        ui.add_space(4.0);
        let mut forget_style: Option<String> = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Mine").strong());
            if state.title_styles.is_empty() {
                ui.label(
                    egui::RichText::new("name this one below to use it again")
                        .small()
                        .color(theme::disabled()),
                );
            }
            for (name, saved) in state.title_styles.all().to_vec() {
                let response = ui
                    .add(egui::Button::selectable(saved == style, &name))
                    .on_hover_text(
                        "Font, size, colour, outline, shadow and spacing. Right-click to forget it",
                    );
                if response.clicked() && saved != style {
                    style = saved.clone();
                    restyled = true;
                }
                response.context_menu(|ui| {
                    if ui
                        .button(format!("Forget \u{201c}{name}\u{201d}"))
                        .clicked()
                    {
                        forget_style = Some(name.clone());
                        ui.close();
                    }
                });
            }
        });
        if let Some(name) = forget_style {
            state.title_styles.remove(&name);
            state.needs_repaint = true;
        }
        ui.horizontal_wrapped(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut state.title_style_name_draft)
                    .desired_width(120.0)
                    .char_limit(crate::title_styles::MAX_NAME)
                    .hint_text("name this style"),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Save style").clicked() || entered)
                && !state.title_style_name_draft.trim().is_empty()
            {
                let name = std::mem::take(&mut state.title_style_name_draft);
                match state.title_styles.save(&name, style.clone()) {
                    Ok(()) => {
                        state.info(format!(
                            "Saved the title style \u{201c}{}\u{201d}",
                            name.trim()
                        ));
                        state.needs_repaint = true;
                    }
                    Err(why) => state.error(why),
                }
            }
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Font").strong());

        ui.horizontal_wrapped(|ui| {
            ui.label("family");
            let current = match &style.family {
                FontFamily::SansSerif => "Sans",
                FontFamily::Serif => "Serif",
                FontFamily::Monospace => "Mono",
                FontFamily::Named(name) => name.as_str(),
            };
            egui::ComboBox::from_id_salt("text family")
                .selected_text(current)
                .width(190.0)
                .show_ui(ui, |ui| {
                    if family_picker(ui, state, &mut style.family) {
                        restyled = true;
                    }
                });
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("weight");
            for weight in FontWeight::ALL {
                if ui
                    .add(egui::Button::selectable(
                        style.weight == weight,
                        weight.label(),
                    ))
                    .clicked()
                {
                    style.weight = weight;
                    restyled = true;
                }
            }
        });

        let mut dragging = false;
        let mut row = |ui: &mut egui::Ui, widget: theme::Labeled<'_>| {
            let response = ui.add(widget);
            if response.changed() {
                dragging |= response.dragged();
                true
            } else {
                false
            }
        };

        restyled |= row(
            ui,
            theme::labeled(
                "size",
                egui::Slider::new(
                    &mut style.size,
                    bettercut_editor_core::text::MIN_SIZE..=bettercut_editor_core::text::MAX_SIZE,
                )
                .logarithmic(true),
            ),
        );
        restyled |= row(
            ui,
            theme::labeled(
                "line spacing",
                egui::Slider::new(&mut style.line_height, 0.5..=3.0),
            ),
        );
        restyled |= row(
            ui,
            theme::labeled(
                "letter spacing",
                egui::Slider::new(&mut style.letter_spacing, -0.2..=1.0),
            ),
        );
        // One line whatever it says: a name or a number that must not wrap or
        // run off comes down in size until it fits its width instead.
        ui.checkbox(&mut style.shrink_to_fit, "Shrink to fit")
        .on_hover_text(
            "A line longer than the title's width comes down in size to fit on one line, instead of wrapping or running off the edge",
        );
        // Text round an arch or a smile, for a badge or a logo look.
        let mut bend = style.curve * 100.0;
        if row(
            ui,
            theme::labeled(
                "curve",
                egui::Slider::new(&mut bend, -100.0..=100.0).suffix("%"),
            ),
        ) {
            style.curve = bend / 100.0;
            restyled = true;
        }

        ui.horizontal_wrapped(|ui| {
            ui.label("colour");
            if colour_button(ui, &mut style.color) {
                restyled = true;
            }
            // A second colour shades the letters from top to bottom.
            let mut shaded = style.gradient.is_some();
            if ui
                .checkbox(&mut shaded, "to")
                .on_hover_text(
                    "Shade the letters from the first colour at the top to a second at the bottom",
                )
                .changed()
            {
                style.gradient =
                    shaded.then_some(bettercut_editor_core::text::Rgba::opaque(255, 170, 40));
                restyled = true;
            }
            if let Some(bottom) = style.gradient.as_mut()
                && colour_button(ui, bottom)
            {
                restyled = true;
            }
            if ui
                .add(egui::Button::selectable(style.italic, "Italic"))
                .clicked()
            {
                style.italic = !style.italic;
                restyled = true;
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("align");
            for align in Alignment::ALL {
                if ui
                    .add(egui::Button::selectable(
                        style.align == align,
                        align.label(),
                    ))
                    .clicked()
                {
                    style.align = align;
                    restyled = true;
                }
            }
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Legibility").strong());
        ui.label(
            egui::RichText::new(
                "White text over an unknown shot is unreadable about half the time.",
            )
            .small()
            .color(theme::disabled()),
        );

        let mut outlined = style.stroke.is_some();
        if ui.checkbox(&mut outlined, "outline").changed() {
            style.stroke = outlined.then(Stroke::default);
            restyled = true;
        }
        if let Some(stroke) = &mut style.stroke {
            ui.horizontal_wrapped(|ui| {
                if colour_button(ui, &mut stroke.color) {
                    restyled = true;
                }
                let response = ui.add(theme::labeled(
                    "width",
                    egui::Slider::new(&mut stroke.width, 0.0..=20.0),
                ));
                if response.changed() {
                    dragging |= response.dragged();
                    restyled = true;
                }
            });
        }

        let mut shadowed = style.shadow.is_some();
        if ui.checkbox(&mut shadowed, "shadow").changed() {
            style.shadow = shadowed.then(Shadow::default);
            restyled = true;
        }
        if let Some(shadow) = &mut style.shadow {
            ui.horizontal_wrapped(|ui| {
                if colour_button(ui, &mut shadow.color) {
                    restyled = true;
                }
                let x = ui.add(
                    egui::DragValue::new(&mut shadow.offset_x)
                        .speed(0.5)
                        .prefix("x "),
                );
                let y = ui.add(
                    egui::DragValue::new(&mut shadow.offset_y)
                        .speed(0.5)
                        .prefix("y "),
                );
                if x.changed() || y.changed() {
                    dragging |= x.dragged() || y.dragged();
                    restyled = true;
                }
            });
            let response = ui.add(theme::labeled(
                "blur",
                egui::Slider::new(&mut shadow.blur, 0.0..=60.0),
            ));
            if response.changed() {
                dragging |= response.dragged();
                restyled = true;
            }
        }

        let mut boxed = style.background.is_some();
        if ui.checkbox(&mut boxed, "background").changed() {
            style.background = boxed.then(Background::default);
            restyled = true;
        }
        if let Some(background) = &mut style.background {
            ui.horizontal_wrapped(|ui| {
                if colour_button(ui, &mut background.color) {
                    restyled = true;
                }
                let padding = ui.add(
                    egui::DragValue::new(&mut background.padding)
                        .speed(0.5)
                        .range(0.0..=200.0)
                        .prefix("pad "),
                );
                let radius = ui.add(
                    egui::DragValue::new(&mut background.corner_radius)
                        .speed(0.5)
                        .range(0.0..=200.0)
                        .prefix("round "),
                );
                if padding.changed() || radius.changed() {
                    dragging |= padding.dragged() || radius.dragged();
                    restyled = true;
                }
            });
        }

        if restyled {
            change = Some((TextProperty::Style(Box::new(style)), dragging));
        }
    }

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Placement").strong());

    // The same controls, the same ranges and the same feel as a video clip's
    // (§54): a title is a layer, and there is no reason for it to behave
    // differently from any other one.
    let mut position = transform.position;
    ui.horizontal_wrapped(|ui| {
        ui.label("position");
        let x = ui.add(
            egui::DragValue::new(&mut position.x)
                .speed(0.005)
                .range(-2.0..=2.0),
        );
        let y = ui.add(
            egui::DragValue::new(&mut position.y)
                .speed(0.005)
                .range(-2.0..=2.0),
        );
        if x.changed() || y.changed() {
            change = Some((
                TextProperty::Position {
                    x: position.x,
                    y: position.y,
                },
                x.dragged() || y.dragged(),
            ));
        }
    });

    let mut scale = transform.scale.x;
    let response = ui.add(theme::labeled(
        "scale",
        egui::Slider::new(&mut scale, 0.05..=4.0).logarithmic(true),
    ));
    if response.changed() {
        change = Some((
            TextProperty::Scale { x: scale, y: scale },
            response.dragged(),
        ));
    }

    let mut rotation = transform.rotation_degrees;
    let response = ui.add(theme::labeled(
        "rotation",
        egui::Slider::new(&mut rotation, -180.0..=180.0),
    ));
    if response.changed() {
        change = Some((TextProperty::Rotation(rotation), response.dragged()));
    }

    let mut value = opacity;
    let response = ui.add(theme::labeled(
        "opacity",
        egui::Slider::new(&mut value, 0.0..=1.0),
    ));
    if response.changed() {
        change = Some((TextProperty::Opacity(value), response.dragged()));
    }

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Animation").strong());
    if text_animation(ui, &mut animation) {
        change = Some((TextProperty::Animation(animation), false));
    }

    // A movement that keeps going while the title is up: a sticker that pulses
    // or a caption that floats.
    ui.horizontal_wrapped(|ui| {
        ui.label("loop");
        let mut looping = animation.looping;
        if ui.selectable_label(looping.is_none(), "None").clicked() {
            looping = None;
        }
        for kind in bettercut_editor_core::timeline::LoopMotion::ALL {
            if ui
                .selectable_label(looping == Some(kind), kind.label())
                .clicked()
            {
                looping = Some(kind);
            }
        }
        if looping != animation.looping {
            animation.looping = looping;
            change = Some((TextProperty::Animation(animation), false));
        }
    });

    // Across the whole frame for the title's whole length: credits or a
    // ticker. Trim the title to set the speed.
    ui.horizontal_wrapped(|ui| {
        ui.label("scroll");
        let mut scroll = animation.scroll;
        if ui.selectable_label(scroll.is_none(), "None").clicked() {
            scroll = None;
        }
        for kind in bettercut_editor_core::timeline::Scroll::ALL {
            if ui
                .selectable_label(scroll == Some(kind), kind.label())
                .on_hover_text(match kind {
                    bettercut_editor_core::timeline::Scroll::Credits => {
                        "Roll up from below the frame to above it, over the title's length"
                    }
                    bettercut_editor_core::timeline::Scroll::Ticker => {
                        "Run from right to left across the frame, over the title's length"
                    }
                })
                .clicked()
            {
                scroll = Some(kind);
            }
        }
        if scroll != animation.scroll {
            animation.scroll = scroll;
            change = Some((TextProperty::Animation(animation), false));
        }
    });

    // Beside the animation, as on a clip: a spin or a pop is over in a third
    // of a second, which is exactly when a smear is worth having.
    let mut blurring = smearing;
    if ui
        .checkbox(&mut blurring, "motion blur")
        .on_hover_text("Smear the title along the way it is moving")
        .changed()
    {
        change = Some((TextProperty::MotionBlur(blurring), false));
    }

    ui.add_space(8.0);
    if ui.button("Remove title").clicked() {
        remove = true;
    }

    if let Some((property, continuing)) = change {
        match editor.set_text_property(clip, property, continuing) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }
    // After the property change, so a look chosen in the same frame as a
    // keystroke wins: the look is the later, larger decision.
    if let Some(look) = look {
        match editor.set_title_look(clip, look) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }
    if remove {
        match editor.remove_text(clip) {
            Ok(()) => {
                state.selected_clips.remove(&clip);
                state.needs_repaint = true;
            }
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// A title's entrance and exit (§26): a preset and a length for each end.
///
/// Returns whether anything changed. A length drag is not coalesced into one
/// undo step the way a slider elsewhere is, deliberately: the slider moves in
/// tenths of a second, so a drag is a handful of steps, and each is a length
/// someone might want to go back to.
fn text_animation(
    ui: &mut egui::Ui,
    animation: &mut bettercut_editor_core::timeline::TextAnimation,
) -> bool {
    use bettercut_editor_core::timeline::MotionKind;

    motion_ends(
        ui,
        [
            ("in", "text intro", &mut animation.intro),
            ("out", "text outro", &mut animation.outro),
        ],
        &MotionKind::FOR_TEXT,
    )
}

/// The entrance-and-exit control itself, for whatever it is animating.
///
/// One control for titles and for clips because they are one feature
/// (§26): the presets are the same, the lengths are the same, and two controls
/// that looked alike but behaved differently would be worse than either.
fn motion_ends(
    ui: &mut egui::Ui,
    ends: [(
        &str,
        &str,
        &mut Option<bettercut_editor_core::timeline::Motion>,
    ); 2],
    kinds: &[bettercut_editor_core::timeline::MotionKind],
) -> bool {
    use bettercut_editor_core::timeline::{DEFAULT_MOTION, Motion};

    let mut changed = false;
    for (label, salt, end) in ends {
        ui.horizontal_wrapped(|ui| {
            ui.add_sized([28.0, 18.0], egui::Label::new(label));
            egui::ComboBox::from_id_salt(salt)
                .selected_text(end.map_or("None", |m| m.kind.label()))
                .width(110.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(end.is_none(), "None").clicked() {
                        *end = None;
                        changed = true;
                    }
                    for &kind in kinds {
                        let chosen = end.is_some_and(|m| m.kind == kind);
                        if ui.selectable_label(chosen, kind.label()).clicked() && !chosen {
                            // Keep the length when only the kind changes.
                            let length = end.map_or(DEFAULT_MOTION, |m| m.duration);
                            *end = Some(Motion::new(kind, length));
                            changed = true;
                        }
                    }
                });
            if let Some(motion) = end {
                // Tenths of a second, as a person thinks of a motion's length;
                // turned into ticks at once, and never used as a position.
                let mut tenths = (motion.duration.ticks() as f64 / 96_000.0).round() as i64;
                let response = ui.add(
                    egui::Slider::new(&mut tenths, 1..=30)
                        .custom_formatter(|v, _| format!("{:.1} s", v / 10.0)),
                );
                if response.changed() {
                    *motion = Motion::new(motion.kind, TimelineTime::from_ticks(tenths * 96_000));
                    changed = true;
                }
            }
        });
    }
    changed
}

/// The font list (§26).
///
/// The three generic names first, because they are the ones that mean something
/// on *any* machine: a project using "Sans" opens correctly on a computer that
/// has never heard of the font this one happens to have. Everything installed
/// follows, behind a filter — three hundred families is normal, and the one
/// anyone wants is rarely near the top.
///
/// Returns whether the choice changed.
fn family_picker(
    ui: &mut egui::Ui,
    state: &mut UiState,
    family: &mut bettercut_editor_core::text::FontFamily,
) -> bool {
    use bettercut_editor_core::text::FontFamily;

    let mut changed = false;

    for (generic, label) in [
        (FontFamily::SansSerif, "Sans"),
        (FontFamily::Serif, "Serif"),
        (FontFamily::Monospace, "Mono"),
    ] {
        if ui.selectable_label(*family == generic, label).clicked() {
            *family = generic;
            changed = true;
        }
    }

    // Your own font: copied in, then on offer in every title from now on.
    if ui
        .button("Import Font…")
        .on_hover_text("Add a .ttf or .otf font file, to use in any title")
        .clicked()
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("Fonts", &["ttf", "otf", "ttc"])
            .pick_file()
    {
        state.font_import = Some(path);
    }
    // A font just imported goes straight onto this title.
    if let Some(name) = state.font_imported.take() {
        *family = FontFamily::Named(name);
        changed = true;
    }

    if state.font_families.is_empty() {
        return changed;
    }

    ui.separator();
    ui.add(
        egui::TextEdit::singleline(&mut state.font_filter)
            .hint_text("Search fonts")
            .desired_width(f32::INFINITY),
    );

    let needle = state.font_filter.to_lowercase();
    let matches: Vec<&String> = state
        .font_families
        .iter()
        .filter(|name| needle.is_empty() || name.to_lowercase().contains(&needle))
        .collect();

    if matches.is_empty() {
        ui.label(
            egui::RichText::new("No font of that name is installed.")
                .small()
                .color(theme::disabled()),
        );
        return changed;
    }

    // Scrolled and height-limited: the list is as long as the machine's font
    // directory, and a combo that grows past the window cannot be reached.
    let mut chosen: Option<String> = None;
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .show(ui, |ui| {
            for name in matches {
                let selected = matches!(family, FontFamily::Named(current) if current == name);
                if ui.selectable_label(selected, name).clicked() {
                    chosen = Some(name.clone());
                }
            }
        });

    if let Some(name) = chosen {
        *family = FontFamily::Named(name);
        changed = true;
    }
    changed
}

/// A colour swatch that opens a picker, in the text crate's own colour type.
///
/// Alpha included: a shadow at full opacity is a black slab, and a background
/// that cannot be made translucent is unusable over footage.
fn colour_button(ui: &mut egui::Ui, colour: &mut bettercut_editor_core::text::Rgba) -> bool {
    let mut rgba = egui::Color32::from_rgba_unmultiplied(colour.r, colour.g, colour.b, colour.a);
    let changed = ui.color_edit_button_srgba(&mut rgba).changed();
    if changed {
        *colour = bettercut_editor_core::text::Rgba::new(rgba.r(), rgba.g(), rgba.b(), rgba.a());
    }
    changed
}

/// Dispatch whatever the rows asked for this frame.
///
/// Collected during the draw and applied after it, because the controls read
/// the project immutably while drawing and every one of these borrows it
/// mutably. Shared by the Video and Colours tabs so the two cannot drift in how
/// they handle a keyframe toggle or a reset.
fn apply_row_actions(
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
    change: Option<(bettercut_editor_core::ClipProperty, bool)>,
    toggle: Option<bettercut_editor_core::ClipProperty>,
    reset: Option<bettercut_editor_core::ClipProperty>,
) {
    if let Some(property) = toggle {
        match editor.toggle_keyframe(clip, property) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }

    if let Some(property) = reset {
        match editor.reset_clip_parameter(clip, property) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }

    if let Some((property, continuing)) = change {
        apply_clip_property(editor, state, clip, property, continuing);
    }
}

/// One animatable control: its keyframe button, then the control itself.
///
/// The button is first so the column of them lines up down the left edge and
/// reads as one thing — which of these move, and which are fixed.
fn keyed_row<R>(
    ui: &mut egui::Ui,
    look: &VideoLook,
    current: bettercut_editor_core::ClipProperty,
    toggle: &mut Option<bettercut_editor_core::ClipProperty>,
    reset: &mut Option<bettercut_editor_core::ClipProperty>,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal_wrapped(|ui| {
        // The control first — its label, slider and value — then whether it
        // is animated, then reset: the order the eye reads a row in.
        let result = control(ui);
        let state = look.row(current);
        let (glyph, hint) = match (state.animated, state.at_playhead) {
            (false, _) => (
                "○",
                "Animate this. A keyframe is added here, and another wherever \
                 you next change it.",
            ),
            // ◊ and ♦ rather than the geometric diamonds ◇ and ◆:
            // those two are in Hack only, and a button draws with the
            // proportional family, so they came out as empty boxes. See
            // tests/glyphs.rs, which now catches that class of bug.
            (true, false) => ("◊", "Add a keyframe at the playhead"),
            (true, true) => ("♦", "Remove the keyframe at the playhead"),
        };

        let colour = if state.animated {
            theme::keyframe()
        } else {
            theme::disabled()
        };
        let button = egui::Button::new(egui::RichText::new(glyph).color(colour))
            .frame(false)
            .min_size(egui::vec2(18.0, 18.0));

        // No playhead over the clip means no frame to key at, so the button is
        // shown disabled with the reason rather than hidden — a control that
        // vanishes is harder to understand than one that explains itself.
        let response = ui.add_enabled(look.source_time.is_some(), button);
        let response = if look.source_time.is_some() {
            response.on_hover_text(hint)
        } else {
            response.on_disabled_hover_text("Move the playhead over this clip to add a keyframe")
        };
        if response.clicked() {
            *toggle = Some(current);
        }

        // Reset last, at the far end of the row: it is the least-used control
        // here and putting it in the reading path would slow every other edit
        // down. Disabled rather than hidden when there is nothing to undo, so
        // the row does not change width as values change and the button does
        // not appear under a cursor that was aiming at the slider.
        let changed = !current.is_default() || look.row(current).animated;
        let button = egui::Button::new(egui::RichText::new("↺").color(if changed {
            theme::clip_text()
        } else {
            theme::disabled()
        }))
        .frame(false)
        .min_size(egui::vec2(18.0, 18.0));

        let response = ui.add_enabled(changed, button);
        if response
            .on_hover_text(format!(
                "Reset {} to its default, and remove its keyframes",
                current.kind().to_lowercase()
            ))
            .clicked()
        {
            *reset = Some(current);
        }

        result
    })
    .inner
}

/// The animation graph: one parameter's curve across the clip, with its keys
/// as points that can be dragged, added and deleted.
///
/// Numbers in a list say *that* something is animated; a curve says what the
/// animation does — where it holds, where it rushes, whether the ease is where
/// the user thinks it is. It is drawn in source time, which is what keys are
/// anchored to (§24), so it keeps its shape when the clip is trimmed or slipped.
fn keyframe_graph(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
    look: &VideoLook,
) {
    use bettercut_editor_core::foundation::MediaTime;

    let Some(video) = editor.video_clip(clip) else {
        return;
    };
    let source = video.source;
    let animated: Vec<AnimatedParameter> = AnimatedParameter::ALL
        .into_iter()
        .filter(|parameter| video.keyframes.is_animated(*parameter))
        .collect();
    let Some(&first) = animated.first() else {
        return;
    };
    let parameter = match state.graph_parameter {
        Some(chosen) if animated.contains(&chosen) => chosen,
        _ => first,
    };
    state.graph_parameter = Some(parameter);

    // Which parameter the graph is showing. One row of small buttons rather
    // than a dropdown: a clip rarely animates more than three or four things,
    // and seeing them all is how the user notices what else is keyed.
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for candidate in &animated {
            if ui
                .selectable_label(*candidate == parameter, candidate.label())
                .clicked()
            {
                state.graph_parameter = Some(*candidate);
                state.needs_repaint = true;
            }
        }
    });

    let keys = editor.keyframes_of(clip, parameter);
    if keys.is_empty() {
        return;
    }

    // The value axis: the parameter's own limits where it has them, widened if
    // a key sits outside, and otherwise the keys' own range with room around
    // it so a flat curve is not drawn on the frame.
    let (mut low, mut high) = keys.iter().fold((f32::MAX, f32::MIN), |(lo, hi), key| {
        (lo.min(key.value), hi.max(key.value))
    });
    if let Some((min, max)) = parameter.limits() {
        low = low.min(min);
        high = high.max(max);
    }
    if (high - low).abs() < 1e-4 {
        low -= 0.5;
        high += 0.5;
    }
    let pad = (high - low) * 0.08;
    let (low, high) = (low - pad, high + pad);

    let height = 132.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4, theme::timeline_background());

    let span = (source.end.ticks() - source.start.ticks()).max(1);
    let x_of = |time: MediaTime| {
        let through = (time.ticks() - source.start.ticks()) as f32 / span as f32;
        rect.left() + 6.0 + through.clamp(0.0, 1.0) * (rect.width() - 12.0)
    };
    let time_of = |x: f32| {
        let through = ((x - rect.left() - 6.0) / (rect.width() - 12.0)).clamp(0.0, 1.0);
        MediaTime::from_ticks(source.start.ticks() + (f64::from(through) * span as f64) as i64)
    };
    let y_of = |value: f32| {
        let through = ((value - low) / (high - low)).clamp(0.0, 1.0);
        rect.bottom() - 6.0 - through * (rect.height() - 12.0)
    };
    let value_of = |y: f32| {
        let through = ((rect.bottom() - 6.0 - y) / (rect.height() - 12.0)).clamp(0.0, 1.0);
        low + through * (high - low)
    };

    // The parameter's resting value, so a curve reads against "no change".
    let default = parameter.default_value();
    if default >= low && default <= high {
        let y = y_of(default);
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, theme::grid_line()),
        );
    }

    // The drag's own idea of where the key is, so the curve follows the pointer
    // rather than jumping on release.
    let dragged = state
        .graph_drag
        .filter(|drag| drag.clip == clip && drag.parameter == parameter && drag.moved);
    let shown: Vec<(MediaTime, f32)> = {
        let mut points: Vec<(MediaTime, f32)> = keys
            .iter()
            .map(|key| match dragged {
                Some(drag) if drag.from == key.time => (drag.to, drag.value),
                _ => (key.time, key.value),
            })
            .collect();
        points.sort_by_key(|(time, _)| time.ticks());
        points
    };

    // The curve between the keys, sampled: the interpolation is the point of
    // the graph, so it is read from the clip rather than drawn as straight
    // lines between points.
    let samples = 96;
    let mut curve: Vec<egui::Pos2> = Vec::with_capacity(samples);
    for step in 0..samples {
        let at =
            MediaTime::from_ticks(source.start.ticks() + span * step as i64 / (samples as i64 - 1));
        let value = match dragged {
            // While a key is moving, straight segments between the points: the
            // clip has not been told about the move yet.
            Some(_) => value_between(&shown, at),
            None => video
                .keyframes
                .value_at(parameter, at)
                .unwrap_or_else(|| value_between(&shown, at)),
        };
        curve.push(egui::pos2(x_of(at), y_of(value)));
    }
    painter.add(egui::Shape::line(
        curve,
        egui::Stroke::new(1.5, theme::keyframe()),
    ));

    // Where the playhead is, when it is over the clip.
    if let Some(at) = look.source_time
        && at >= source.start
        && at <= source.end
    {
        let x = x_of(at);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, theme::playhead()),
        );
    }

    for (time, value) in &shown {
        painter.circle_filled(
            egui::pos2(x_of(*time), y_of(*value)),
            4.0,
            theme::keyframe(),
        );
    }

    // The axis, so the numbers behind the shape are readable.
    painter.text(
        egui::pos2(rect.left() + 4.0, rect.top() + 2.0),
        egui::Align2::LEFT_TOP,
        format!("{high:.2}"),
        egui::FontId::proportional(10.0),
        theme::disabled(),
    );
    painter.text(
        egui::pos2(rect.left() + 4.0, rect.bottom() - 2.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{low:.2}"),
        egui::FontId::proportional(10.0),
        theme::disabled(),
    );

    let pointer = response.interact_pointer_pos().or_else(|| {
        ui.ctx()
            .pointer_latest_pos()
            .filter(|pos| rect.contains(*pos))
    });
    let nearest = |pos: egui::Pos2| -> Option<(MediaTime, f32)> {
        shown
            .iter()
            .copied()
            .map(|(time, value)| {
                let at = egui::pos2(x_of(time), y_of(value));
                (at.distance(pos), time, value)
            })
            .filter(|(distance, _, _)| *distance <= 9.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, time, value)| (time, value))
    };

    if response.drag_started()
        && let Some(pos) = pointer
        && let Some((time, value)) = nearest(pos)
    {
        state.graph_drag = Some(crate::state::KeyframeDrag {
            clip,
            parameter,
            from: time,
            to: time,
            value,
            moved: false,
        });
    }

    if response.dragged()
        && let Some(pos) = pointer
        && let Some(drag) = state.graph_drag.as_mut()
        && drag.clip == clip
    {
        drag.moved = true;
        drag.to = time_of(pos.x);
        drag.value = value_of(pos.y);
        state.needs_repaint = true;
    }

    if response.drag_stopped()
        && let Some(drag) = state.graph_drag.take()
        && drag.moved
        && let Err(err) = editor.move_keyframe(clip, drag.parameter, drag.from, drag.to, drag.value)
    {
        state.error(err.to_string());
    }

    // Double-click: on a key, delete it; anywhere else, add one there.
    if response.double_clicked()
        && let Some(pos) = pointer
    {
        state.graph_drag = None;
        let result = match nearest(pos) {
            Some((time, _)) => editor.remove_keyframe_at(clip, parameter, time),
            None => editor.set_keyframe(
                clip,
                parameter,
                bettercut_editor_core::timeline::Keyframe::new(
                    time_of(pos.x),
                    value_of(pos.y),
                    bettercut_editor_core::timeline::Interpolation::default(),
                ),
            ),
        };
        if let Err(err) = result {
            state.error(err.to_string());
        }
        state.needs_repaint = true;
    }

    ui.label(
        egui::RichText::new("Drag a point to move it. Double-click to add one, or to remove the one under the pointer.")
            .small()
            .color(theme::disabled()),
    );
}

/// The value at `at` along straight lines between `points` — what the graph
/// draws while a key is mid-drag and the clip has not been told yet.
fn value_between(
    points: &[(bettercut_editor_core::foundation::MediaTime, f32)],
    at: bettercut_editor_core::foundation::MediaTime,
) -> f32 {
    match points.first() {
        None => 0.0,
        Some((first, value)) if at <= *first => *value,
        _ => {
            let last = points.last().copied().unwrap_or((at, 0.0));
            if at >= last.0 {
                return last.1;
            }
            points
                .windows(2)
                .find(|pair| at >= pair[0].0 && at <= pair[1].0)
                .map_or(last.1, |pair| {
                    let (a, b) = (pair[0], pair[1]);
                    let span = (b.0.ticks() - a.0.ticks()).max(1) as f32;
                    let through = (at.ticks() - a.0.ticks()) as f32 / span;
                    a.1 + (b.1 - a.1) * through
                })
        }
    }
}

/// How many keys the clip has, and a way back to them (§24).
///
/// Without this, animation is invisible unless the playhead happens to be
/// sitting on a key: the buttons show ◇ everywhere and there is nothing saying
/// the clip is animated at all.
fn animation_summary(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
    look: &VideoLook,
) {
    let Some(video) = editor.video_clip(clip) else {
        return;
    };
    if video.keyframes.is_empty() {
        return;
    }

    let count = video.keyframes.len();
    let times = video.keyframes.times();
    let at = look.source_time;
    // Resolved before dispatching, because both borrow the editor.
    let previous = at.and_then(|at| times.iter().rev().find(|t| **t < at).copied());
    let next = at.and_then(|at| times.iter().find(|t| **t > at).copied());
    let start = look.timeline.start;
    let source_start = video.source.start;

    let mut jump_to: Option<MediaTime> = None;

    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(if count == 1 {
                "1 keyframe".to_owned()
            } else {
                format!("{count} keyframes")
            })
            .small()
            .color(theme::keyframe()),
        );
        if ui
            .add_enabled(previous.is_some(), egui::Button::new("◀").frame(false))
            .on_hover_text("Previous keyframe")
            .clicked()
        {
            jump_to = previous;
        }
        if ui
            .add_enabled(next.is_some(), egui::Button::new("▶").frame(false))
            .on_hover_text("Next keyframe")
            .clicked()
        {
            jump_to = next;
        }
    });

    if let Some(target) = jump_to {
        // Source time back to timeline time: the clip's own offset, in the
        // integer ticks §9 requires.
        let into_source = target.ticks() - source_start.ticks();
        editor.set_playhead(TimelineTime::from_ticks(start.ticks() + into_source));
        state.needs_repaint = true;
    }
}

/// Volume for the selected audio clip (§20a.4).
/// Low cut, high cut and presence for a sound clip. A slider at its "off" end
/// says so, rather than showing a frequency that does nothing.
fn eq_controls(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::ClipProperty;
    use bettercut_editor_core::timeline::{
        ClipEq, EQ_HIGH_CUT_MAX, EQ_HIGH_CUT_MIN, EQ_LOW_CUT_MAX, EQ_PRESENCE_MAX,
    };

    let Some(current) = editor.audio_clip(clip).map(|c| c.eq) else {
        return;
    };
    let mut next = current;
    let mut dragging = false;
    ui.label(egui::RichText::new("EQ").small().color(theme::disabled()));
    // By name first: most sound wants one of these, and the sliders below
    // are for the last few percent.
    ui.horizontal_wrapped(|ui| {
        for (name, hint, preset) in ClipEq::PRESETS {
            if ui
                .selectable_label(current == preset, name)
                .on_hover_text(hint)
                .clicked()
            {
                next = preset;
            }
        }
        if !current.is_flat()
            && ui
                .small_button("Flat")
                .on_hover_text("Take the EQ off")
                .clicked()
        {
            next = ClipEq::default();
        }
    });

    // Low cut: 0 (off) up to 400 Hz.
    let response = ui
        .add(theme::labeled(
            "low cut",
            egui::Slider::new(&mut next.low_cut, 0.0..=EQ_LOW_CUT_MAX).custom_formatter(|v, _| {
                if v < 20.0 {
                    "off".to_owned()
                } else {
                    format!("{v:.0} Hz")
                }
            }),
        ))
        .on_hover_text("Take out rumble and boom below this — 80 to 120 Hz suits most voices");
    dragging |= response.dragged();

    // High cut: shown from 2 kHz to 20 kHz, where the top end is off.
    let mut high = if next.high_cut <= 0.0 {
        EQ_HIGH_CUT_MAX
    } else {
        next.high_cut
    };
    let response = ui
        .add(theme::labeled(
            "high cut",
            egui::Slider::new(&mut high, EQ_HIGH_CUT_MIN..=EQ_HIGH_CUT_MAX)
                .logarithmic(true)
                .custom_formatter(|v, _| {
                    if v >= f64::from(EQ_HIGH_CUT_MAX) - 1.0 {
                        "off".to_owned()
                    } else {
                        format!("{:.1} kHz", v / 1_000.0)
                    }
                }),
        ))
        .on_hover_text("Soften hiss and harshness above this");
    dragging |= response.dragged();
    next.high_cut = if high >= EQ_HIGH_CUT_MAX - 1.0 {
        0.0
    } else {
        high
    };

    let response = ui
        .add(theme::labeled(
            "presence",
            egui::Slider::new(&mut next.presence, -EQ_PRESENCE_MAX..=EQ_PRESENCE_MAX)
                .suffix(" dB")
                .fixed_decimals(1),
        ))
        .on_hover_text("Bring a voice forward (up) or set it back (down), around 3 kHz");
    dragging |= response.dragged();

    // Mains hum: the one noise in a recording that is a *tone*, and so the one
    // that can be taken out cleanly rather than guessed at.
    ui.horizontal_wrapped(|ui| {
        ui.label("hum");
        for (label, hz) in [("off", 0.0_f32), ("50 Hz", 50.0), ("60 Hz", 60.0)] {
            if ui
                .add(egui::Button::selectable((next.hum - hz).abs() < 0.5, label))
                .on_hover_text(match hz {
                    0.0 => "Leave the low end alone",
                    50.0 => "Notch out 50 Hz mains hum and its harmonics — Europe, Asia, Africa",
                    _ => "Notch out 60 Hz mains hum and its harmonics — the Americas, Japan",
                })
                .clicked()
            {
                next.hum = hz;
            }
        }
    });

    if next != current {
        apply_clip_property(editor, state, clip, ClipProperty::Eq(next), dragging);
    }
    if !current.is_flat()
        && ui
            .small_button("Flat")
            .on_hover_text("Turn the lot off")
            .clicked()
    {
        apply_clip_property(
            editor,
            state,
            clip,
            ClipProperty::Eq(ClipEq::default()),
            false,
        );
    }
}

fn space_controls(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    use bettercut_editor_core::ClipProperty;
    use bettercut_editor_core::timeline::{ClipSpace, SpaceKind};

    let Some(current) = editor.audio_clip(clip).map(|c| c.space) else {
        return;
    };
    ui.label(
        egui::RichText::new("Echo & reverb")
            .small()
            .color(theme::disabled()),
    );
    let mut next = current;
    let mut dragging = false;
    ui.horizontal_wrapped(|ui| {
        for kind in SpaceKind::ALL {
            if ui
                .selectable_label(current.kind == kind, kind.label())
                .on_hover_text(kind.description())
                .clicked()
                && current.kind != kind
            {
                next.kind = kind;
                // A fresh choice starts at a blend that is heard.
                if kind != SpaceKind::Dry && current.mix <= 0.0 {
                    next.mix = 0.35;
                }
            }
        }
    });
    if current.kind != SpaceKind::Dry {
        let mut percent = next.mix * 100.0;
        let response = ui
            .add(theme::labeled(
                "amount",
                egui::Slider::new(&mut percent, 0.0..=100.0)
                    .suffix(" %")
                    .fixed_decimals(0),
            ))
            .on_hover_text("How much of the echo or room is blended with the sound");
        if response.changed() {
            next.mix = percent / 100.0;
            dragging = response.dragged();
        }
    }
    if next != current {
        let next = if next.kind == SpaceKind::Dry {
            ClipSpace::default()
        } else {
            next
        };
        apply_clip_property(editor, state, clip, ClipProperty::Space(next), dragging);
    }
}

fn clip_audio_properties(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
    gain: f32,
) {
    use bettercut_editor_core::ClipProperty;

    ui.add_space(4.0);
    let mut value = gain;
    // Up to 2x rather than the model's 4x ceiling: past that a clip is almost
    // certainly clipping, and the limiter's work is not a volume control.
    let response = ui.add(theme::labeled(
        "volume",
        egui::Slider::new(&mut value, 0.0..=2.0),
    ));
    if response.changed() {
        apply_clip_property(
            editor,
            state,
            clip,
            ClipProperty::Gain(value),
            response.dragged(),
        );
    }

    // The one-click version of the clean-up below, for a voice recorded
    // anywhere but a studio. It sets the sliders, which stay to adjust.
    if ui
        .button("Enhance Voice")
        .on_hover_text(
            "Clean up speech in one click: hiss and rumble down, words clearer \
             and more even, the hiss on an \"s\" softened. Sets the sliders \
             below; undo takes it all back.",
        )
        .clicked()
    {
        match editor.enhance_voice(clip) {
            Ok(()) => state.info("Voice enhanced — the sliders below show what changed"),
            Err(err) => state.error(err.to_string()),
        }
    }

    // Beside the volume: both are about how this recording sounds.
    let denoise = editor.audio_clip(clip).map_or(0.0, |c| c.denoise);
    let mut cleaned = denoise;
    let response = ui
        .add(theme::labeled(
            "voice clean-up",
            egui::Slider::new(
                &mut cleaned,
                0.0..=bettercut_editor_core::timeline::MAX_DENOISE,
            )
            .suffix("%"),
        ))
        .on_hover_text(
            "Take out rumble and pull the room's hiss down between words. \
             For speech — a steady sound with no pauses is treated as background.",
        );
    if response.changed() {
        apply_clip_property(
            editor,
            state,
            clip,
            ClipProperty::Denoise(cleaned),
            response.dragged(),
        );
    }

    // Between the words, the room turned down: the plain tool beside the
    // clean-up, for a room that is quiet enough to simply close on.
    let gated = editor.audio_clip(clip).map_or(0.0, |c| c.gate);
    let mut next_gate = gated;
    let response = ui
        .add(
            theme::labeled("noise gate", egui::Slider::new(&mut next_gate, 0.0..=100.0)
                .suffix("%")),
        )
        .on_hover_text(
            "Turn the room down between the words. Higher closes on louder rooms and closes further; it never goes all the way to silence",
        );
    if response.changed() {
        apply_clip_property(
            editor,
            state,
            clip,
            ClipProperty::Gate(next_gate),
            response.dragged(),
        );
    }

    // Loud and quiet words closer together.
    let levelled = editor.audio_clip(clip).map_or(0.0, |c| c.leveller);
    let mut next_level = levelled;
    let response = ui
        .add(theme::labeled(
            "leveller",
            egui::Slider::new(&mut next_level, 0.0..=100.0).suffix("%"),
        ))
        .on_hover_text("Even out a voice: loud words turned down, the whole lifted back up");
    if response.changed() {
        apply_clip_property(
            editor,
            state,
            clip,
            ClipProperty::Leveller(next_level),
            response.dragged(),
        );
    }

    // Where the clip sits between the speakers, on top of its lane's pan: a
    // number, or a line of keys read at the playhead so a sound can cross the
    // picture with its subject (`AnimatedParameter::Pan`).
    if let Some(audio) = editor.audio_clip(clip) {
        let playhead = editor.playhead();
        let riding = audio.keyframes.is_animated(AnimatedParameter::Pan);
        let mut pan = if riding {
            audio.pan_at(playhead)
        } else {
            audio.pan
        };
        let response = ui
            .add(theme::labeled(
                if riding { "pan (keyed)" } else { "pan" },
                egui::Slider::new(&mut pan, -1.0..=1.0)
                    .custom_formatter(|v, _| crate::context_menu::pan_label(v as f32)),
            ))
            .on_hover_text(if riding {
                "This clip's pan follows its keys; dragging writes the key at the playhead"
            } else {
                "Left to right, on top of the lane's own pan"
            });
        if response.changed() {
            if riding {
                match editor.key_pan_at_playhead(clip, pan, response.dragged()) {
                    Ok(()) => state.needs_repaint = true,
                    Err(err) => state.error(err.to_string()),
                }
            } else {
                apply_clip_property(
                    editor,
                    state,
                    clip,
                    ClipProperty::Pan(pan),
                    response.dragged(),
                );
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(if riding { "Key pan here" } else { "Start a pan line" })
                .on_hover_text(
                    "Put a pan key at the playhead; between keys the sound crosses from one to the next",
                )
                .clicked()
            {
                match editor.key_pan_at_playhead(clip, pan, false) {
                    Ok(()) => state.needs_repaint = true,
                    Err(err) => state.error(err.to_string()),
                }
            }
            if riding
                && ui
                    .button("Clear pan line")
                    .on_hover_text("Back to one pan for the whole clip")
                    .clicked()
            {
                match editor.set_pan_envelope(clip, &[], false) {
                    Ok(_) => state.needs_repaint = true,
                    Err(err) => state.error(err.to_string()),
                }
            }
        });
    }

    // The hiss on an "s", dipped only while it is happening.
    let de_essed = editor.audio_clip(clip).map_or(0.0, |c| c.de_ess);
    let mut next_ess = de_essed;
    let response = ui
        .add(theme::labeled(
            "de-esser",
            egui::Slider::new(&mut next_ess, 0.0..=100.0).suffix("%"),
        ))
        .on_hover_text(
            "Take the hiss off \"s\" and \"t\" without dulling the voice: the sibilant \
             band is turned down only while an ess is happening",
        );
    if response.changed() {
        apply_clip_property(
            editor,
            state,
            clip,
            ClipProperty::DeEss(next_ess),
            response.dragged(),
        );
    }

    // The tone of the recording: three controls, each with an "off" end.
    eq_controls(ui, editor, state, clip);

    // Where the sound is: dry, or an echo or a room around it.
    space_controls(ui, editor, state, clip);

    // The voice changer: up for a chipmunk, down for a deep voice.
    let pitch = editor.audio_clip(clip).map_or(0.0, |c| c.pitch);
    let mut next_pitch = pitch;
    let mut pitch_drag = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new("Voice")
                .small()
                .color(theme::disabled()),
        );
        for (label, semitones, hint) in [
            ("Normal", 0.0, "The voice as recorded"),
            ("Chipmunk", 7.0, "Higher and squeakier, at the same speed"),
            ("Deep", -5.0, "Lower and heavier, at the same speed"),
        ] {
            if ui
                .selectable_label((pitch - semitones).abs() < 0.05, label)
                .on_hover_text(hint)
                .clicked()
            {
                next_pitch = semitones;
            }
        }
    });
    // The robot is its own control, not a pitch: on and off here, how much
    // on its slider.
    let robot = editor.audio_clip(clip).map_or(0.0, |c| c.robot);
    let mut next_robot = robot;
    ui.horizontal_wrapped(|ui| {
        theme::row_label(ui, "");
        if ui
            .selectable_label(robot > 0.0, "Robot")
            .on_hover_text("A machine's voice: the words stay, the warmth goes")
            .clicked()
        {
            next_robot = if robot > 0.0 { 0.0 } else { 100.0 };
        }
    });
    let mut robot_drag = false;
    if robot > 0.0 {
        let response = ui.add(theme::labeled(
            "robot",
            egui::Slider::new(&mut next_robot, 0.0..=100.0).suffix("%"),
        ));
        robot_drag = response.dragged();
    }
    if next_robot != robot {
        apply_clip_property(
            editor,
            state,
            clip,
            bettercut_editor_core::ClipProperty::Robot(next_robot),
            robot_drag,
        );
    }
    let response = ui
        .add(theme::labeled(
            "pitch",
            egui::Slider::new(&mut next_pitch, -12.0..=12.0)
                .suffix(" st")
                .fixed_decimals(1),
        ))
        .on_hover_text("Move the pitch up or down in semitones without changing the speed");
    pitch_drag |= response.dragged();
    if (next_pitch - pitch).abs() > 1e-4 {
        apply_clip_property(
            editor,
            state,
            clip,
            bettercut_editor_core::ClipProperty::Pitch(next_pitch),
            pitch_drag,
        );
    }

    // A mic recorded on one side only: heard in both ears.
    let mode = editor
        .audio_clip(clip)
        .map_or(bettercut_editor_core::timeline::ChannelMode::Stereo, |c| {
            c.channels
        });
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new("Channels")
                .small()
                .color(theme::disabled()),
        );
        for choice in bettercut_editor_core::timeline::ChannelMode::ALL {
            if ui
                .selectable_label(mode == choice, choice.label())
                .on_hover_text(choice.description())
                .clicked()
                && mode != choice
            {
                apply_clip_property(
                    editor,
                    state,
                    clip,
                    bettercut_editor_core::ClipProperty::Channels(choice),
                    false,
                );
            }
        }
    });

    // Into the next sound, where one follows straight on: the cut blended
    // rather than jumped.
    // How wide: narrower towards mono for a take that has to survive a phone
    // speaker, wider for music that wants the room. The middle never moves.
    let width = editor.audio_clip(clip).map_or(1.0, |c| c.stereo_width);
    let mut next_width = width;
    let response = ui
        .add(
            theme::labeled("width", egui::Slider::new(&mut next_width, 0.0..=2.0)
                .custom_formatter(|v, _| match v {
                    v if v < 0.05 => "mono".to_owned(),
                    v if (v - 1.0).abs() < 0.05 => "as recorded".to_owned(),
                    v => format!("{:.0}%", v * 100.0),
                })),
        )
        .on_hover_text(
            "How wide the stereo image is: down to mono, or pushed out past how it was recorded. What is in the middle stays where it is",
        );
    if response.changed() {
        apply_clip_property(
            editor,
            state,
            clip,
            bettercut_editor_core::ClipProperty::StereoWidth(next_width),
            response.dragged(),
        );
    }

    if editor.next_touching_sound(clip).is_some() {
        let current = editor
            .audio_clip(clip)
            .map_or(TimelineTime::ZERO, |c| c.crossfade_out);
        let mut seconds = current.ticks() as f32 / 960_000.0;
        let max = bettercut_editor_core::timeline::MAX_CROSSFADE.ticks() as f32 / 960_000.0;
        let response = ui
            .add(theme::labeled(
                "crossfade into next",
                egui::Slider::new(&mut seconds, 0.0..=max).custom_formatter(|v, _| {
                    if v < 0.01 {
                        "off".to_owned()
                    } else {
                        format!("{v:.2} s")
                    }
                }),
            ))
            .on_hover_text(
                "Blend this sound into the next one across the cut. Needs a little of \
                 each recording beyond the cut to blend with.",
            );
        if response.changed() {
            let length = TimelineTime::from_ticks((seconds * 960_000.0).round() as i64);
            match editor.set_audio_crossfade(clip, length, response.dragged()) {
                Ok(_) => state.needs_repaint = true,
                Err(err) => state.error(err.to_string()),
            }
        }
    }

    // Fades, in tenths of a second: the unit a person thinks of a fade in,
    // turned into ticks at once and never used as a position.
    let Some((fade_in, fade_out)) = editor.audio_clip(clip).map(|c| (c.fade_in, c.fade_out)) else {
        return;
    };
    let tenths = |t: TimelineTime| (t.ticks() as f64 / 96_000.0).round() as i64;
    let mut values = [tenths(fade_in), tenths(fade_out)];
    let mut dragging = false;
    let mut changed = false;
    for (value, label) in values.iter_mut().zip(["fade in", "fade out"]) {
        let response = ui.add(theme::labeled(
            label,
            egui::Slider::new(value, 0..=100).custom_formatter(|v, _| {
                if v == 0.0 {
                    "none".to_owned()
                } else {
                    format!("{:.1} s", v / 10.0)
                }
            }),
        ));
        if response.changed() {
            changed = true;
            dragging |= response.dragged();
        }
    }
    if changed {
        let [fade_in, fade_out] = values.map(|v| TimelineTime::from_ticks(v * 96_000));
        match editor.set_clip_fades(clip, fade_in, fade_out, dragging) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }

    // How the fades curve, once there is a fade to shape.
    if fade_in > TimelineTime::ZERO || fade_out > TimelineTime::ZERO {
        let shape = editor
            .audio_clip(clip)
            .map(|c| c.fade_shape)
            .unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.label("fade shape");
            for option in bettercut_editor_core::timeline::FadeShape::ALL {
                if ui
                    .selectable_label(shape == option, option.label())
                    .on_hover_text(match option {
                        bettercut_editor_core::timeline::FadeShape::Smooth => {
                            "Even to the ear, all the way through"
                        }
                        bettercut_editor_core::timeline::FadeShape::Linear => {
                            "A straight line: seems to hold, then drop at the end"
                        }
                        bettercut_editor_core::timeline::FadeShape::Fast => {
                            "Most of the change straight away"
                        }
                        bettercut_editor_core::timeline::FadeShape::Slow => {
                            "Holds on, then goes at the very end"
                        }
                    })
                    .clicked()
                    && shape != option
                {
                    apply_clip_property(
                        editor,
                        state,
                        clip,
                        bettercut_editor_core::ClipProperty::FadeShape(option),
                        false,
                    );
                }
            }
        });
    }
}

fn apply_clip_property(
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
    property: bettercut_editor_core::ClipProperty,
    continuing: bool,
) {
    // Several clips of the same kind selected: the change goes on all of
    // them, as one step (`editor_core::many_clips`). Not when the clip being
    // shown is keyframed for this — its control is moving a key, which is a
    // thing only it has.
    let picture = editor.video_clip(clip).is_some();
    let sound = editor.audio_clip(clip).is_some();
    let together: Vec<bettercut_editor_core::foundation::ClipId> = if (picture || sound)
        && state.selected_clips.len() > 1
        && state.selected_clips.contains(&clip)
        && !editor.animates(clip, property)
    {
        state
            .selected_clips
            .iter()
            .copied()
            .filter(|other| {
                (picture && editor.video_clip(*other).is_some())
                    || (sound && editor.audio_clip(*other).is_some())
            })
            .collect()
    } else {
        Vec::new()
    };
    if together.len() > 1 {
        match editor.set_clips_property(&together, property, continuing) {
            Ok(_) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
        return;
    }

    // `set_clip_value`, not `set_clip_property`: an animated parameter is
    // edited by moving its keyframe, and the control does not need to know
    // which it is doing (§24).
    match editor.set_clip_value(clip, property, continuing) {
        Ok(()) => state.needs_repaint = true,
        Err(err) => state.error(err.to_string()),
    }
}

/// Put a clip's look back to default, as one undo step (§79).
fn reset_video_properties(
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
) {
    match editor.reset_clip_look(clip) {
        Ok(()) => {
            state.needs_repaint = true;
            state.info("Clip reset");
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Resolution and frame rate for the active sequence (§8, §36).
///
/// Both go through one command, because "make this a 1080p50 project" is one
/// decision and should be one undo step.
/// Re-scale every clip for the frame, and say what happened (§36).
///
/// The count matters: "nothing to do" and "done" look identical on a timeline
/// of clips that were already the right shape, and a button that appears to do
/// nothing is a button the user stops trusting.
fn reframe(editor: &mut Editor, state: &mut UiState, fill: bool) {
    match editor.reframe_clips(fill) {
        Ok((0, 0)) => state.info("Every clip already frames that way"),
        Ok((changed, 0)) => state.info(format!("Reframed {changed} clip(s)")),
        Ok((changed, animated)) => state.info(format!(
            "Reframed {changed} clip(s); left {animated} with a movement of its own alone"
        )),
        Err(err) => state.error(err.to_string()),
    }
}

/// Whether a size is that shape, to within rounding. The shared rule, so the
/// shape buttons and the export dialog's extra shapes agree on what a project
/// already is.
fn matches_aspect(size: Resolution, ratio: (u32, u32)) -> bool {
    bettercut_editor_core::reshape::matches_aspect(size, ratio)
}

/// Reshape to `ratio`, keeping the short edge — the same rule the reshaped
/// export copies use.
fn with_aspect(size: Resolution, ratio: (u32, u32)) -> Resolution {
    bettercut_editor_core::reshape::with_aspect(size, ratio)
}

/// Scale so the short edge is `short`, keeping the shape.
fn resize_short_edge(size: Resolution, short: u32) -> Resolution {
    let (w, h) = (u64::from(size.width.max(1)), u64::from(size.height.max(1)));
    let short = u64::from(short);
    let scaled = if w <= h {
        Resolution::new(short as u32, (short * h / w) as u32)
    } else {
        Resolution::new((short * w / h) as u32, short as u32)
    };
    even_size(scaled)
}

/// The name a saved frame is offered under: the project and the timecode, so
/// several stills from one edit sort in the order they appear in it. Anything a
/// file name cannot hold becomes a dash — timecodes are full of colons.
pub fn still_file_name(project: &str, at: TimelineTime) -> String {
    let name = if project.trim().is_empty() {
        "frame"
    } else {
        project.trim()
    };
    let clean: String = format!("{name} {}", at.format_timecode())
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c })
        .collect();
    format!("{clean}.png")
}

/// Both dimensions even: §36 requires it, and 4:2:0 chroma has no
/// representation for an odd one.
fn even_size(size: Resolution) -> Resolution {
    Resolution::new(size.width.max(2) & !1, size.height.max(2) & !1)
}

fn sequence_format(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    current: Resolution,
    rate: FrameRate,
) {
    let mut wanted = (current, rate);

    // Shape first, then how big. Two different questions: "is this a YouTube
    // video or a Reel" is decided once and fixes the framing everything gets
    // composed against, while "how many pixels" is a quality dial that can move
    // later without recomposing anything.
    ui.horizontal_wrapped(|ui| {
        ui.label("shape");
        for shape in bettercut_editor_core::SHAPES {
            let (name, ratio, hint) = (shape.label, shape.ratio, shape.hint);
            let selected = matches_aspect(current, ratio);
            if ui
                .selectable_label(selected, name)
                .on_hover_text(hint)
                .clicked()
                && !selected
            {
                // Keeps the short edge, which is the detail the user has been
                // working at: reshaping 1920×1080 to 9:16 gives 1080×1920, not
                // something smaller in both directions.
                wanted.0 = with_aspect(current, ratio);
            }
        }
    });

    // Changing the shape leaves every clip the shape it was, which is §36's
    // rule — the media is not touched — and also, for a whole edit moved from
    // landscape to vertical, a sequence of pillarboxed clips. These say it
    // once for all of them.
    //
    // Said out loud when it applies, so the buttons below read as the answer to
    // something rather than three options nobody asked for. A fact in grey, not
    // a warning: choosing Fit keeps the bars on purpose, and a hint that turned
    // red at a deliberate choice would be a nag.
    let showing_bars = editor.clips_showing_bars();
    if showing_bars > 0 {
        ui.label(
            egui::RichText::new(match showing_bars {
                1 => "1 clip shows bars at this shape".to_string(),
                n => format!("{n} clips show bars at this shape"),
            })
            .small()
            .color(theme::disabled()),
        );
    }
    ui.horizontal_wrapped(|ui| {
        ui.label("clips");
        if ui
            .button("Fill frame")
            .on_hover_text("Scale every clip to cover the frame, cropping what does not fit")
            .clicked()
        {
            reframe(editor, state, true);
        }
        if ui
            .button("Crop to frame")
            .on_hover_text(
                "Take the frame's shape out of every clip at full size. Unlike Fill, nothing is enlarged — a 1080p shot stays 1080p.",
            )
            .clicked()
        {
            match editor.auto_crop_clips() {
                Ok((0, 0)) => state.info("Every clip is already the frame's shape"),
                Ok((changed, 0)) => state.info(format!("Cropped {changed} clip(s) to the frame")),
                Ok((changed, kept)) => state.info(format!(
                    "Cropped {changed} clip(s); left {kept} you had cropped yourself alone"
                )),
                Err(err) => state.error(err.to_string()),
            }
        }
        if ui
            .button("Fit frame")
            .on_hover_text("Show every clip whole, with bars where the shapes differ")
            .clicked()
        {
            reframe(editor, state, false);
        }
    });

    ui.horizontal_wrapped(|ui| {
        ui.label("size");
        egui::ComboBox::from_id_salt("sequence_resolution")
            .selected_text(format!("{}×{}", current.width, current.height))
            .show_ui(ui, |ui| {
                // Sizes for the shape the sequence already has, named by their
                // short edge — "1080p" is 1080 lines, which is 1920×1080
                // landscape and 1080×1920 vertical. A custom size still
                // round-trips through the project file; this is a shortcut,
                // not a restriction.
                for (label, short) in [
                    ("2160p (4K)", 2160_u32),
                    ("1440p", 1440),
                    ("1080p", 1080),
                    ("720p", 720),
                    ("480p", 480),
                ] {
                    let size = resize_short_edge(current, short);
                    if ui
                        .selectable_label(
                            current == size,
                            format!("{label}  —  {}×{}", size.width, size.height),
                        )
                        .on_hover_text("Export size; the preview still scales down (§16)")
                        .clicked()
                    {
                        wanted.0 = size;
                    }
                }
            });
    });

    ui.horizontal_wrapped(|ui| {
        ui.label("rate");
        egui::ComboBox::from_id_salt("sequence_rate")
            .selected_text(format!("{rate} fps"))
            .show_ui(ui, |ui| {
                // Only the nine rates §9's timebase divides exactly. Anything
                // else would put every frame boundary slightly off.
                for value in FrameRate::SUPPORTED {
                    let hint = match value {
                        FrameRate::FILM_23_976 | FrameRate::NTSC_29_97 | FrameRate::NTSC_59_94 => {
                            "NTSC rate — exact, not rounded"
                        }
                        FrameRate::PAL_25 | FrameRate::PAL_50 => "PAL rate",
                        FrameRate::FILM_24 => "Cinema",
                        _ => "",
                    };
                    let response = ui.selectable_value(&mut wanted.1, value, format!("{value}"));
                    if !hint.is_empty() {
                        response.on_hover_text(hint);
                    }
                }
            });
    });

    // What the first frame is called: an hour for a broadcast delivery, or
    // whatever the programme's slate says. Positions stay from zero inside.
    ui.horizontal_wrapped(|ui| {
        ui.label("starts at");
        let start = editor.start_timecode().ticks();
        let per_second = bettercut_editor_core::foundation::TICKS_PER_SECOND;
        let mut hours = start / per_second / 3_600;
        let mut minutes = start / per_second / 60 % 60;
        let mut seconds = start / per_second % 60;
        let h = ui.add(egui::DragValue::new(&mut hours).range(0..=23).suffix(" h"));
        let m = ui.add(egui::DragValue::new(&mut minutes).range(0..=59).suffix(" m"));
        let s = ui.add(egui::DragValue::new(&mut seconds).range(0..=59).suffix(" s"));
        if (h.changed() || m.changed() || s.changed())
            && let Err(err) = editor.set_start_timecode(TimelineTime::from_seconds(
                hours * 3_600 + minutes * 60 + seconds,
            ))
        {
            state.error(err.to_string());
        }
    })
    .response
    .on_hover_text("What the first frame is called on the ruler, the readouts and a timecode burn-in. Nothing moves");

    if wanted != (current, rate) {
        match editor.set_sequence_format(wanted.0, wanted.1) {
            Ok(()) => {
                state.needs_repaint = true;
                // §9: positions are ticks, so nothing moves. Say so, because
                // "will this shift my cuts?" is the natural worry.
                state.info(format!(
                    "Sequence is now {}×{} @ {} fps — clips keep their exact positions",
                    wanted.0.width, wanted.0.height, wanted.1
                ));
            }
            Err(err) => state.error(err.to_string()),
        }
    }

    if clips_exist(editor) && rate != wanted.1 {
        ui.label(
            egui::RichText::new("Existing cuts keep their exact times and are not re-snapped.")
                .small()
                .color(theme::disabled()),
        );
    }
}

fn clips_exist(editor: &Editor) -> bool {
    editor.active_sequence().is_some_and(|s| s.clip_count() > 0)
}

/// The crash-recovery prompt (§39.4).
///
/// §39.5: *"Never overwrite the original project automatically."* So this
/// **offers** the recovered work and does nothing until the user chooses. It is
/// modal because the choice cannot be deferred sensibly — editing first and
/// deciding later would mean recovering over the top of new work.
pub fn recovery_prompt(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    let Some(session) = state.pending_recovery.as_ref() else {
        return;
    };

    let summary = session.summary();
    let mut decision: Option<bool> = None;

    egui::Modal::new(egui::Id::new("recovery")).show(ctx, |ui| {
        ui.set_min_width(420.0);
        ui.heading("Recover unsaved work?");
        ui.add_space(6.0);
        ui.label(summary);
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "bettercut did not shut down cleanly last time. This is what it \
                 was able to rebuild. Your saved project file has not been changed.",
            )
            .small()
            .color(theme::disabled()),
        );
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            if ui.button("Recover").clicked() {
                decision = Some(true);
            }
            if ui.button("Discard").clicked() {
                decision = Some(false);
            }
        });
    });

    let Some(accept) = decision else {
        return;
    };

    // `take` first: whichever way this goes, the prompt is finished with.
    let Some(session) = state.pending_recovery.take() else {
        return;
    };
    state.needs_repaint = true;

    if accept {
        let (recovered, _events) = Editor::from_project(session.accept());
        *editor = recovered;
        // Recovered work is by definition not saved anywhere yet.
        state.info("Recovered unsaved work — save it to keep it");
    } else {
        session.discard();
        state.info("Discarded the recovered work");
    }
}

/// Status bar: project state at a glance, plus the last message.
pub fn status_bar(ui: &mut egui::Ui, editor: &Editor, state: &mut UiState) {
    // A message that arrived with a new undo step is an edit's: only then is
    // Undo offered beside it.
    let depth = editor.undo_depth();
    let text = state.status.as_ref().map(|m| m.text.clone());
    if text != state.status_seen {
        state.status_from_edit = text.is_some() && depth > state.last_undo_depth;
        state.status_seen = text;
    }
    state.last_undo_depth = depth;
    ui.horizontal(|ui| {
        match &state.status {
            Some(message) if message.is_error => {
                ui.label(egui::RichText::new(&message.text).color(theme::error_text()));
            }
            Some(message) => {
                ui.label(egui::RichText::new(&message.text).color(theme::ok_text()));
                // The way back, right beside what was just done.
                if state.status_from_edit
                    && let Some(label) = editor.undo_label()
                    && ui
                        .small_button("Undo")
                        .on_hover_text(crate::keys::keys(&format!("Undo {label} (Ctrl+Z)")).into_owned())
                        .clicked()
                {
                    state.undo_request = true;
                }
            }
            None => {
                ui.label(
                    egui::RichText::new(crate::keys::keys(
                        "S split · Del delete · Shift+Del ripple · Ctrl+D duplicate · \
                         N snap · drag edges to trim · Ctrl+K any action",
                    ))
                    .color(theme::disabled()),
                );
            }
        }

        // How long the cut is, and how many clips are in it: the numbers a
        // person checks over and over while cutting to a length, and the
        // ones they otherwise open the export window to read.
        if let Some(sequence) = editor.active_sequence() {
            let duration = sequence.duration();
            let clips = sequence.clip_count();
            let picked = state.selected_clips.len();
            let span = (picked > 1)
                .then(|| state.selection_range(editor))
                .flatten();
            // In the flow, after the hint: only the file and its saved state
            // sit at the right edge, so the two never draw over each other.
            ui.separator();
            ui.horizontal(|ui| {
                // Several clips selected: how much of the cut they cover —
                // the number checked before a delete or a move.
                if let Some(span) = span {
                    ui.label(
                        egui::RichText::new(format!(
                            "{picked} selected · {}",
                            span.duration().format_timecode()
                        ))
                        .monospace()
                        .color(theme::selection()),
                    )
                    .on_hover_text("From the first selected clip's start to the last one's end");
                    ui.separator();
                }
                ui.label(
                    egui::RichText::new(format!(
                        "{} · {clips} clip{}",
                        duration.format_timecode(),
                        if clips == 1 { "" } else { "s" }
                    ))
                    .monospace()
                    .color(theme::disabled()),
                )
                .on_hover_text(
                    "How long this sequence runs, and how many clips are in it",
                );
            });
        }

        // §47a.4 / §20a: quiet when playback is keeping up, loud when it is
        // not. A counter shown permanently stops being read; one that appears
        // only when something is wrong is worth looking at.
        if let Some(stats) = state.playback
            && (stats.dropped_frames > 0 || stats.underruns > 0)
        {
            ui.separator();
            let mut parts = Vec::new();
            if stats.dropped_frames > 0 {
                parts.push(format!("{} dropped", stats.dropped_frames));
            }
            if stats.underruns > 0 {
                parts.push(format!("{} audio underrun(s)", stats.underruns));
            }
            ui.label(egui::RichText::new(parts.join(" · ")).color(theme::error_text()))
                .on_hover_text(
                    "Playback is not keeping up. Inspector, System has the full \
                     counters; lowering Proxies, Quality is the usual fix.",
                );
        }

        // §42: background work has to be visible while it runs. Sits next to
        // the hint text rather than in the corner, because "why is my fan on?"
        // is a question the user asks while looking at the middle of the app.
        if let Some((count, fraction)) = state.proxy_progress {
            ui.separator();
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(120.0)
                    .text(format!("{count} proxy")),
            )
            .on_hover_text(
                "Generating edit-friendly copies of your media.\n\
                 Editing works meanwhile; it just uses the originals.",
            );
        }

        // An export is the one background job the user is waiting on, so it
        // gets its own bar and a way to stop it (§42, §48).
        if let Some(fraction) = state.export_progress {
            ui.separator();
            let left = state
                .export_elapsed
                .and_then(|elapsed| export_time_left(elapsed, fraction));
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(140.0)
                    .text(format!("Exporting {:.0}%", fraction * 100.0)),
            )
            .on_hover_text("Rendering your timeline to a video file.");
            if let Some(left) = left {
                ui.label(egui::RichText::new(left).color(theme::ruler_text()));
            }
            if ui
                .button("Stop")
                .on_hover_text("Cancel the export. The partial file is removed.")
                .clicked()
            {
                state.export_stop_requested = true;
            }
            let waiting = state.export_queue.waiting.len();
            if ui
                .button(if waiting > 0 {
                    format!("Queue ({waiting} waiting)")
                } else {
                    "Queue".to_owned()
                })
                .on_hover_text("See the exports waiting their turn, and take any off")
                .clicked()
            {
                state.export_queue.open = !state.export_queue.open;
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let path = editor
                .path()
                .map_or_else(|| "unsaved".to_owned(), |p| p.display().to_string());
            ui.label(egui::RichText::new(path).small().color(theme::disabled()));
            if editor.is_dirty() {
                ui.separator();
                ui.label(
                    egui::RichText::new("unsaved changes")
                        .small()
                        .color(theme::selection()),
                );
            }
            // A newer version, found by the day's check (`crate::updates`).
            let update = state.update_found.lock().ok().and_then(|u| u.clone());
            if let Some(update) = update {
                ui.separator();
                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(format!("bettercut {} is out", update.version))
                                .small()
                                .color(theme::accent_text()),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Open its download page")
                    .clicked()
                {
                    state.open_url = Some(update.url);
                }
            }
            // An assistant working in this window, so its edits are no
            // surprise; a click opens how it is connected.
            if crate::assistant::is_connected(state.assistant_seen, std::time::Instant::now()) {
                ui.separator();
                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new("assistant connected")
                                .small()
                                .color(theme::accent_text()),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text("An AI assistant is editing in this window. Click for details.")
                    .clicked()
                {
                    state.assistant_open = true;
                }
            }

            // §38: autosave is mandatory, and an autosave that has quietly
            // stopped working is worse than none — the user believes they are
            // protected while they are not. A full disk or a read-only folder
            // is exactly how it happens, and neither announces itself.
            //
            // Loudest thing in the bar, because it is the only thing here that
            // means work is at risk.
            let failures = editor.autosave_failures();
            if failures > 0 {
                ui.separator();
                ui.label(
                    egui::RichText::new(autosave_warning(failures))
                        .small()
                        .strong()
                        .color(theme::error_text()),
                )
                .on_hover_text(
                    "Recovery data could not be written — a full disk or a folder that cannot be written to. Save the project somewhere else.",
                );
            }
        });
    });
}

/// What the status bar says when autosave is failing.
///
/// The count matters: one failure can be a file the antivirus held open for a
/// moment, while a hundred is a disk that is not coming back. Saying "not
/// saving" rather than a number of errors, because what the user needs to know
/// is what it means for them, not how many times it happened.
fn autosave_warning(failures: u32) -> String {
    match failures {
        1 => "autosave failed once".to_owned(),
        n => format!("NOT autosaving — {n} failures"),
    }
}

// ---- actions -------------------------------------------------------------

pub fn new_project(editor: &mut Editor, state: &mut UiState) {
    // §39/§50: never discard unsaved work silently. Until a proper prompt
    // exists, refuse and say why.
    // Unsaved work: ask, and come back here once it is answered.
    if editor.is_dirty() && !state.discard_ok {
        state.pending_switch = Some(crate::save_prompt::Switch::New);
        return;
    }
    let (fresh, _rx) = Editor::new_project("Untitled");
    *editor = fresh;
    reset_state(state);
    state.info("New project created");
}

/// Read a subtitle file onto the timeline (§27, Milestone 10).
pub fn import_captions(editor: &mut Editor, state: &mut UiState) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("subtitles", &["srt", "vtt"])
        .pick_file()
    else {
        return;
    };

    match editor.import_captions(&path) {
        Ok(count) => state.info(format!("Imported {count} captions")),
        Err(err) => state.error(format!("Could not import captions: {err}")),
    }
}

/// Write the caption lane back out.
pub fn export_captions(editor: &mut Editor, state: &mut UiState) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("SubRip", &["srt"])
        .add_filter("WebVTT", &["vtt"])
        .add_filter("Transcript (plain text)", &["txt"])
        .set_file_name("captions.srt")
        .save_file()
    else {
        return;
    };

    match editor.export_captions(&path) {
        Ok(count) => state.info(format!("Wrote {count} captions to {}", path.display())),
        Err(err) => state.error(format!("Could not export captions: {err}")),
    }
}

pub fn open_project(editor: &mut Editor, state: &mut UiState) {
    // Unsaved work: ask, and come back here once it is answered.
    if editor.is_dirty() && !state.discard_ok {
        state.pending_switch = Some(crate::save_prompt::Switch::Open);
        return;
    }
    let Some(path) = rfd::FileDialog::new()
        .add_filter("bettercut project", &["vproj"])
        .pick_file()
    else {
        return;
    };
    open_project_at(editor, state, &path);
}

/// Open the project at `path` — from the file dialog or the recent list — and
/// remember it. The same unsaved-work guard as the dialog route: a recent
/// project one click away must not make discarding an edit one click away.
pub fn open_project_at(editor: &mut Editor, state: &mut UiState, path: &std::path::Path) {
    // Unsaved work: ask, and come back here once it is answered.
    if editor.is_dirty() && !state.discard_ok {
        state.pending_switch = Some(crate::save_prompt::Switch::OpenPath(path.to_path_buf()));
        return;
    }
    if !path.exists() {
        // Picked from the list and still not there: it has gone for good, as
        // far as this list can tell.
        state.recent.forget(path);
        state.error(format!(
            "{} is not there any more, so it was taken off the recent list",
            path.display()
        ));
        return;
    }
    match Editor::open(path) {
        Ok((opened, _rx)) => {
            *editor = opened;
            // Bakes whose files have gone since the project was saved: the
            // cache cleared, the folder tidied, the project moved without its
            // renders (`editor_core::render_in_place`).
            editor.prune_renders();
            reset_state(state);
            state.recent.touch(path);
            state.info(format!("Opened {}", path.display()));
        }
        Err(err) => state.error(format!("Could not open project: {err}")),
    }
}

/// A fresh interface for a different project, keeping what belongs to the
/// person or the machine rather than the project: the recent list, the
/// prefs and everything saved beside them, the key bindings, the palette's
/// recent actions, the font list and the graphics card. Losing those here
/// emptied the font picker and stopped prefs being saved after the first
/// project was opened.
pub fn reset_state(state: &mut UiState) {
    let recent = std::mem::take(&mut state.recent);
    let prefs = std::mem::take(&mut state.prefs);
    let title_styles = std::mem::take(&mut state.title_styles);
    let export_presets = std::mem::take(&mut state.export_presets);
    let user_looks = std::mem::take(&mut state.user_looks);
    let keymap = std::mem::take(&mut state.keymap);
    let font_families = std::mem::take(&mut state.font_families);
    let gpu = state.gpu.take();
    let palette_recent = std::mem::take(&mut state.palette_recent);
    *state = UiState::default();
    state.recent = recent;
    state.prefs = prefs;
    state.title_styles = title_styles;
    state.export_presets = export_presets;
    state.user_looks = user_looks;
    state.keymap = keymap;
    state.font_families = font_families;
    state.gpu = gpu;
    state.palette_recent = palette_recent;
}

/// What a copy is called by default: the project's name with "copy" after it.
pub fn copy_file_name(project_name: &str) -> String {
    let name = project_name.trim();
    let name = if name.is_empty() { "Untitled" } else { name };
    format!("{name} copy.vproj")
}

/// Save under a new name, and keep working in the new file.
pub fn save_project_as(editor: &mut Editor, state: &mut UiState) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("bettercut project", &["vproj"])
        .set_file_name(format!("{}.vproj", editor.project().name))
        .save_file()
    else {
        return;
    };
    match editor.save_as(path) {
        Ok(()) => {
            if let Some(path) = editor.path() {
                state.recent.touch(path);
            }
            state.info("Project saved");
        }
        Err(err) => state.error(format!("Could not save: {err}")),
    }
}

/// Copy the project and its files into a folder the user picks.
///
/// The originals are left alone (§2), so this is safe to point at a drive that
/// turns out to be too small: what fits is copied and the rest is reported.
pub fn collect_files(editor: &mut Editor, state: &mut UiState) {
    let Some(folder) = rfd::FileDialog::new()
        .set_title("Collect the project and its files into…")
        .pick_folder()
    else {
        return;
    };
    match editor.collect_into(&folder) {
        Ok(done) if done.missing.is_empty() => state.info(format!(
            "Collected {} file(s) into {}",
            done.copied,
            folder.display()
        )),
        Ok(done) => state.error(format!(
            "Collected {} file(s); {} could not be copied: {}",
            done.copied,
            done.missing.len(),
            done.missing.join("; ")
        )),
        Err(err) => state.error(format!("Could not collect: {err}")),
    }
    state.needs_repaint = true;
}

/// Bytes as something a person reads: "4.2 GB".
fn readable_bytes(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("GB", 1 << 30), ("MB", 1 << 20), ("kB", 1 << 10)];
    for (unit, size) in UNITS {
        if bytes >= size {
            return format!("{:.1} {unit}", bytes as f64 / size as f64);
        }
    }
    format!("{bytes} bytes")
}

/// Write a copy of the project somewhere else, and stay in this one.
pub fn save_copy(editor: &mut Editor, state: &mut UiState) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("bettercut project", &["vproj"])
        .set_file_name(copy_file_name(&editor.project().name))
        .save_file()
    else {
        return;
    };
    match editor.save_copy(path) {
        Ok(written) => state.info(format!(
            "Copy saved as {}; still working in this project",
            written.file_name().map_or_else(
                || written.display().to_string(),
                |n| n.to_string_lossy().into_owned()
            )
        )),
        Err(err) => state.error(format!("Could not save a copy: {err}")),
    }
}

pub fn save_project(editor: &mut Editor, state: &mut UiState) {
    let result = if editor.path().is_some() {
        editor.save()
    } else {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("bettercut project", &["vproj"])
            .set_file_name("Untitled.vproj")
            .save_file()
        else {
            return;
        };
        editor.save_as(path)
    };

    match result {
        Ok(()) => {
            if let Some(path) = editor.path() {
                state.recent.touch(path);
            }
            state.info("Project saved");
        }
        Err(err) => state.error(format!("Could not save: {err}")),
    }
}

fn import_media(editor: &mut Editor, state: &mut UiState) {
    let Some(paths) = rfd::FileDialog::new()
        .add_filter(
            "media",
            &[
                "mp4", "mov", "mkv", "webm", "avi", "mp3", "wav", "m4a", "flac", "png", "jpg",
                "jpeg", "heic", "heif", "webp", "bmp", "gif", "tif", "tiff",
            ],
        )
        .pick_files()
    else {
        return;
    };

    import_paths(editor, state, &paths);
}

/// Ask for one frame of a numbered run and import the whole run as one clip
/// at the sequence's rate, so it lands on the grid it will play on.
fn import_image_sequence(editor: &mut Editor, state: &mut UiState) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter(
            "still images",
            &[
                "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "ppm", "dpx", "exr",
            ],
        )
        .set_title("Pick any frame of the sequence")
        .pick_file()
    else {
        return;
    };
    let rate = editor
        .active_sequence()
        .map_or(FrameRate::PAL_25, |s| s.frame_rate);
    match editor.import_image_sequence(&path, rate) {
        Ok(id) => {
            let frames = editor
                .project()
                .media_asset(id)
                .and_then(|asset| asset.sequence)
                .map_or(0, |sequence| sequence.count);
            state.info(format!(
                "Imported {frames} frames as one clip at {rate} fps"
            ));
            state.needs_repaint = true;
        }
        Err(err) => state.error(format!("Could not import an image sequence: {err}")),
    }
}

/// Import files from wherever they came from — the file picker, or dropped
/// onto the window — and say what happened. Returns what was imported, in the
/// order given.
pub fn import_paths(
    editor: &mut Editor,
    state: &mut UiState,
    paths: &[std::path::PathBuf],
) -> Vec<MediaId> {
    let total = paths.len();
    let mut imported = 0;
    let mut ids = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut adopted = None;

    for path in paths {
        match editor.import_file(path) {
            Ok(id) => {
                imported += 1;
                ids.push(id);
                // §8: match an empty sequence to the first real video, so 25 or
                // 50 fps footage does not land on a 30 fps grid and judder.
                if adopted.is_none() {
                    adopted = editor.adopt_format_from(id);
                }
            }
            Err(err) => {
                // §50: one unreadable file must not abort the whole import.
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                tracing::warn!(path = %path.display(), %err, "import failed");
                failures.push(name);
            }
        }
    }

    match failures.len() {
        0 => match adopted {
            // Say it rather than doing it silently: the sequence format is the
            // user's to control, and a change they did not make should be
            // visible and undoable.
            Some((resolution, rate)) => state.info(format!(
                "Imported {imported} file(s) — sequence set to {}x{} @ {rate} fps to match",
                resolution.width, resolution.height
            )),
            None => state.info(format!("Imported {imported} file(s)")),
        },
        n if n == total => state.error(format!("Could not read {}", failures.join(", "))),
        _ => state.error(format!(
            "Imported {imported} of {total}; could not read {}",
            failures.join(", ")
        )),
    }
    ids
}

/// §33's slideshow, with its three choices in front of the button that uses
/// them.
///
/// A popover rather than a dialog: there are three decisions, all of which have
/// good defaults, and a window to dismiss would make the common case — press
/// it, get a slideshow — a step longer. Each option shows its effect in words
/// rather than as a number to interpret.
fn slideshow_menu(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, photos: &[MediaId]) {
    use bettercut_editor_core::foundation::{TICKS_PER_SECOND, TimelineTime};
    use bettercut_editor_core::slideshow::{MAX_HOLD, MIN_HOLD};
    use bettercut_editor_core::timeline::{Movement, TransitionKind};

    let mut build = false;
    ui.menu_button(format!("Make Slideshow ({} photos)", photos.len()), |ui| {
        ui.set_min_width(220.0);
        let plan = &mut state.slideshow;

        // Whole seconds. A slideshow's rhythm is felt in seconds, and a slider
        // of fractions invites fiddling with a difference nobody sees.
        let mut seconds = plan.hold.ticks() / TICKS_PER_SECOND;
        ui.add(
            theme::labeled("seconds each", egui::Slider::new(
                &mut seconds,
                MIN_HOLD.ticks() / TICKS_PER_SECOND..=MAX_HOLD.ticks() / TICKS_PER_SECOND,
            )
            .integer()),
        );
        plan.hold = TimelineTime::from_seconds(seconds);

        ui.add_space(4.0);
        ui.label("between photos");
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(egui::Button::selectable(plan.transition.is_none(), "Cut"))
                .on_hover_text("Straight from one photo to the next")
                .clicked()
            {
                plan.transition = None;
            }
            for kind in TransitionKind::ALL {
                if ui
                    .add(egui::Button::selectable(
                        plan.transition == Some(kind),
                        kind.label(),
                    ))
                    .on_hover_text(kind.description())
                    .clicked()
                {
                    plan.transition = Some(kind);
                }
            }
        });

        ui.add_space(4.0);
        ui.label("movement");
        ui.horizontal_wrapped(|ui| {
            for movement in Movement::ALL {
                if ui
                    .add(egui::Button::selectable(
                        plan.movement == movement,
                        movement.label(),
                    ))
                    .clicked()
                {
                    plan.movement = movement;
                }
            }
        });

        ui.add_space(6.0);
        // What it will come to, so the choice of seconds means something
        // before it is committed to.
        let total = plan.total(photos.len()).ticks() / TICKS_PER_SECOND;
        ui.label(
            egui::RichText::new(format!("{} photos, {total} s in all", photos.len()))
                .small()
                .color(theme::disabled()),
        );

        if ui.button("Make Slideshow").clicked() {
            build = true;
            ui.close();
        }

        // One photo per beat instead of a set number of seconds each.
        let beats = editor.markers().len();
        if ui
            .add_enabled(beats >= 2, egui::Button::new("Cut to the Beat"))
            .on_hover_text("One photo from each marker to the next, so the pictures change on the music's beats")
            .on_disabled_hover_text("Mark the music's beats first: right-click a sound clip and choose Mark Beats")
            .clicked()
        {
            ui.close();
            match editor.photos_to_beats(photos, plan.movement) {
                Ok(clips) => {
                    state.info(format!("{} photos cut to the beat", clips.len()));
                    state.needs_repaint = true;
                }
                Err(err) => state.error(err.to_string()),
            }
        }
    });

    if build {
        // After whatever is already on the first picture track, rather than at
        // the playhead: the slideshow refuses to write over existing clips, and
        // a button that usually refuses is a button people learn to ignore.
        let at = editor
            .active_sequence()
            .and_then(|sequence| sequence.video_tracks.first())
            .map(|track| track.duration())
            .unwrap_or_default();
        match editor.build_slideshow(photos, state.slideshow, at) {
            Ok(applied) => {
                state.info(format!("Slideshow of {} photos", applied.clips.len()));
                state.needs_repaint = true;
            }
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// A synthetic 5-second asset, so the timeline, selection, undo, and save/load
/// can all be exercised before there is a decoder.
fn add_placeholder_clip(editor: &mut Editor, state: &mut UiState) {
    let index = editor.project().media.len() + 1;
    let asset = MediaAsset::new(
        MediaKind::Video,
        format!("placeholder-{index}.mp4"),
        MediaTime::from_seconds(5),
    )
    .with_video(1920, 1080, FrameRate::FPS_30);

    let id = editor.import_media(asset);
    place_on_timeline(editor, state, id);
}

/// Put an imported asset on the timeline (§12).
///
/// The assembling is the editor's (§54, §86): a video file is a picture clip
/// *and* a sound clip, which is a decision about the model rather than about
/// the interface, and this used to make a `VideoClip` for everything — so an
/// imported video arrived silent and an imported song arrived as a video clip
/// on a video track.
fn place_on_timeline(editor: &mut Editor, state: &mut UiState, media_id: MediaId) {
    // The part marked on the file, if one is (`UiState::mark_media`).
    let part = state.marked_part(media_id);
    match editor.place_media_range(media_id, part) {
        Ok(clips) => {
            let what = if part.is_some() {
                "Part added"
            } else {
                "Clip added"
            };
            state.info(if clips.len() > 1 {
                format!("{what}, with its sound")
            } else {
                what.to_owned()
            });
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// Put a file on the timeline *at the playhead*, rather than after everything
/// else: three-point editing (`editor_core::three_point`).
fn drop_at_playhead(
    editor: &mut Editor,
    state: &mut UiState,
    media_id: MediaId,
    kind: bettercut_editor_core::three_point::DropKind,
) {
    use bettercut_editor_core::three_point::DropKind;

    let part = state.marked_part(media_id);
    let at = editor.playhead();
    match editor.place_media_at(media_id, part, at, kind) {
        Ok(clips) => {
            let what = match kind {
                DropKind::Insert => "Inserted",
                DropKind::Overwrite => "Overwritten",
            };
            state.info(if clips.len() > 1 {
                format!("{what} at the playhead, with its sound")
            } else {
                format!("{what} at the playhead")
            });
            state.needs_repaint = true;
        }
        Err(err) => state.error(err.to_string()),
    }
}

/// A file's rating, as five stars to click.
///
/// Clicking the star a file already has takes the rating off again, which is
/// how every rating control in every media tool behaves — and the only way to
/// say "actually, not that one" without a second control for it.
fn stars_row(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, media: MediaId) {
    let rating = editor.media_rating(media);
    let mut chosen: Option<u8> = None;
    ui.horizontal(|ui| {
        for star in 1..=5_u8 {
            let lit = star <= rating;
            let response = ui
                .add(
                    egui::Button::new(
                        egui::RichText::new(if lit { "★" } else { "☆" })
                            .size(13.0)
                            .color(if lit {
                                theme::selection()
                            } else {
                                theme::disabled()
                            }),
                    )
                    .frame(false),
                )
                .on_hover_text(if lit && star == rating {
                    "Take the rating off".to_owned()
                } else {
                    format!("Rate this {star} of 5")
                });
            if response.clicked() {
                chosen = Some(if star == rating { 0 } else { star });
            }
        }
    });
    if let Some(stars) = chosen {
        match editor.set_media_rating(media, stars) {
            Ok(true) => state.needs_repaint = true,
            Ok(false) => {}
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// The part of a file marked in the browser, drawn along the bottom of its
/// thumbnail: a bar for the marked span, or a single line for an in-point
/// waiting for its out-point.
fn draw_marks(
    ui: &egui::Ui,
    state: &UiState,
    media: MediaId,
    rect: egui::Rect,
    duration: MediaTime,
) {
    let Some((from, to)) = state.media_marks.get(&media).copied() else {
        return;
    };
    if duration.is_zero() {
        return;
    }
    let x_of = |at: MediaTime| {
        let through = (at.ticks() as f64 / duration.ticks() as f64).clamp(0.0, 1.0) as f32;
        rect.left() + through * rect.width()
    };
    let band = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - 4.0),
        egui::pos2(rect.right(), rect.bottom()),
    );
    ui.painter()
        .rect_filled(band, 0, theme::timeline_background());
    match to {
        Some(to) => {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x_of(from), band.top()),
                    egui::pos2(x_of(to).max(x_of(from) + 2.0), band.bottom()),
                ),
                0,
                theme::selection(),
            );
        }
        None => {
            ui.painter().line_segment(
                [
                    egui::pos2(x_of(from), band.top() - 4.0),
                    egui::pos2(x_of(from), band.bottom()),
                ],
                egui::Stroke::new(2.0, theme::selection()),
            );
        }
    }
}

#[cfg(test)]
mod sequence_shape_tests {
    use super::*;

    /// A saved frame is offered under the project and the timecode, with the
    /// colons a timecode is full of made safe for a file name.
    #[test]
    fn a_still_is_named_for_the_project_and_the_instant() {
        let at = TimelineTime::from_millis(83_250);
        assert_eq!(still_file_name("Trip", at), "Trip 00-01-23.250.png");
        assert_eq!(still_file_name("Trip 6/7", at), "Trip 6-7 00-01-23.250.png");
        assert_eq!(still_file_name("  ", at), "frame 00-01-23.250.png");
        // Two stills of one edit sort in the order they appear in it.
        assert!(
            still_file_name("Trip", TimelineTime::from_seconds(9))
                < still_file_name("Trip", TimelineTime::from_seconds(61))
        );
    }

    /// Reshaping keeps the detail the user has been working at, so switching a
    /// 1080p landscape project to vertical gives 1080×1920 rather than a
    /// smaller frame in both directions.
    #[test]
    fn reshaping_keeps_the_short_edge() {
        let landscape = Resolution::HD_1080;
        assert_eq!(with_aspect(landscape, (9, 16)), Resolution::new(1080, 1920));
        assert_eq!(with_aspect(landscape, (1, 1)), Resolution::new(1080, 1080));
        assert_eq!(with_aspect(landscape, (16, 9)), Resolution::new(1920, 1080));

        // And back again, without having shrunk on the way.
        let vertical = with_aspect(landscape, (9, 16));
        assert_eq!(with_aspect(vertical, (16, 9)), landscape);
    }

    #[test]
    fn every_shape_of_every_size_is_even() {
        for shape in bettercut_editor_core::SHAPES {
            let (name, ratio) = (shape.label, shape.ratio);
            for start in [
                Resolution::HD_1080,
                Resolution::new(1080, 1920),
                Resolution::new(3840, 2160),
                Resolution::new(999, 501),
            ] {
                let shaped = with_aspect(start, ratio);
                assert!(
                    shaped.width.is_multiple_of(2) && shaped.height.is_multiple_of(2),
                    "{name} of {}×{} gave {}×{}",
                    start.width,
                    start.height,
                    shaped.width,
                    shaped.height
                );
            }
        }
    }

    /// The selected shape has to light up for the size the project is actually
    /// at, or every project looks like it has no shape chosen.
    #[test]
    fn common_sizes_report_their_shape() {
        assert!(matches_aspect(Resolution::HD_1080, (16, 9)));
        assert!(matches_aspect(Resolution::HD_720, (16, 9)));
        assert!(matches_aspect(Resolution::VERTICAL_1080, (9, 16)));
        assert!(matches_aspect(Resolution::new(1080, 1080), (1, 1)));
        assert!(matches_aspect(Resolution::new(3840, 2160), (16, 9)));

        assert!(!matches_aspect(Resolution::HD_1080, (9, 16)));
        assert!(!matches_aspect(Resolution::HD_1080, (1, 1)));
        // Exactly one shape claims each of the usual sizes.
        let claims = bettercut_editor_core::SHAPES
            .iter()
            .filter(|shape| matches_aspect(Resolution::HD_1080, shape.ratio))
            .count();
        assert_eq!(claims, 1, "1920×1080 matched {claims} shapes");
    }

    /// The same rule the export dialog uses, and the bug it had: "1080p" is the
    /// short edge, so a 4K sequence resizes to 1920×1080, not 1080×608.
    #[test]
    fn resizing_names_the_short_edge() {
        let uhd = Resolution::new(3840, 2160);
        assert_eq!(resize_short_edge(uhd, 1080), Resolution::HD_1080);
        assert_eq!(resize_short_edge(uhd, 720), Resolution::HD_720);
        assert_eq!(
            resize_short_edge(Resolution::VERTICAL_1080, 720),
            Resolution::new(720, 1280)
        );
    }
}

#[cfg(test)]
mod preview_pick_tests {
    use super::*;
    use bettercut_editor_core::media::{MediaAsset, MediaKind};
    use bettercut_editor_core::timeline::{SourceRange, VideoClip};
    use bettercut_editor_core::{ClipPayload, Editor};

    /// Two video tracks, each with a clip under the playhead.
    ///
    /// Returns the clips in *track* order — bottom first — which is the
    /// opposite of what a click should find, so the test can tell the two
    /// orders apart.
    fn stacked() -> (Editor, [bettercut_editor_core::foundation::ClipId; 2]) {
        let (mut editor, _rx) = Editor::new_project("Preview picking");
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            "C:/media/a.mp4",
            MediaTime::from_seconds(60),
        ));
        let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).expect("range");

        let bottom_track = editor.active_sequence().expect("sequence").video_tracks[0].id;
        let bottom = VideoClip::new(media, TimelineTime::ZERO, source).expect("clip");
        let bottom_id = bottom.id;
        editor
            .add_clip(bottom_track, ClipPayload::Video(Box::new(bottom)))
            .expect("add");

        editor.add_video_track("V2".to_owned()).expect("track");
        let top_track = editor.active_sequence().expect("sequence").video_tracks[1].id;
        let top = VideoClip::new(media, TimelineTime::ZERO, source).expect("clip");
        let top_id = top.id;
        editor
            .add_clip(top_track, ClipPayload::Video(Box::new(top)))
            .expect("add");

        (editor, [bottom_id, top_id])
    }

    fn canvas() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 180.0))
    }

    /// §26: a title is grabbable in the picture like anything else. Without
    /// this the only way to place one is the Inspector's number fields.
    #[test]
    fn a_title_gets_a_box_too() {
        let (mut editor, [_, _]) = stacked();
        let title = editor.add_text("Hello").expect("add text");
        editor.set_playhead(TimelineTime::ZERO);

        // 400×120, as the rasterizer would have reported after drawing it.
        let shown = visible_boxes(&editor, &|_| Some((400, 120)), canvas(), 16.0 / 9.0);

        let found = shown
            .iter()
            .find(|s| s.clip == title)
            .expect("the title has no box");
        assert!(found.box_on_canvas.area() > 0.0);
        assert_eq!(
            shown.first().map(|s| s.clip),
            Some(title),
            "the title is not first: a click would land on the shot behind it"
        );
    }

    /// A title that has never been drawn has no size to draw a box around. The
    /// next frame has the answer, so skipping is right and panicking is not.
    #[test]
    fn a_title_with_no_rendered_size_is_skipped() {
        let (mut editor, [_, _]) = stacked();
        let title = editor.add_text("Hello").expect("add text");

        let shown = visible_boxes(&editor, &|_| None, canvas(), 16.0 / 9.0);
        assert!(shown.iter().all(|s| s.clip != title));
    }

    /// The box goes around where the picture *is*, and a title is drawn at its
    /// own size rather than fitted — so a small bitmap gets a small box.
    #[test]
    fn a_titles_box_is_its_natural_size_not_the_whole_frame() {
        let (mut editor, [_, _]) = stacked();
        let title = editor.add_text("Hello").expect("add text");

        let shown = visible_boxes(&editor, &|_| Some((480, 270)), canvas(), 16.0 / 9.0);
        let found = shown.iter().find(|s| s.clip == title).expect("box");

        // A 480-wide bitmap in a 1920-wide sequence covers a quarter of it.
        let fraction = found.box_on_canvas.width() / canvas().width();
        assert!(
            (fraction - 0.25).abs() < 0.02,
            "the box covers {fraction} of the frame rather than a quarter"
        );
    }

    /// The transform carried on the box is the clip's *own*, because a gesture
    /// starts from it and a command writes it back. Seeded with the composited
    /// one, the first drag of a corner would collapse the title.
    #[test]
    fn a_titles_gesture_starts_from_its_own_scale() {
        let (mut editor, [_, _]) = stacked();
        let title = editor.add_text("Hello").expect("add text");

        let shown = visible_boxes(&editor, &|_| Some((480, 270)), canvas(), 16.0 / 9.0);
        let found = shown.iter().find(|s| s.clip == title).expect("box");
        assert_eq!(
            found.transform.scale.x, 1.0,
            "the gesture would start from the corrected scale"
        );
    }

    /// §22 stacks track 0 at the bottom, so a click has to find the *last*
    /// track first. Getting this backwards would silently pick whatever is
    /// hidden behind the picture the user is looking at.
    #[test]
    fn the_topmost_clip_is_listed_first() {
        let (editor, [bottom, top]) = stacked();
        let shown = visible_boxes(&editor, &|_| None, canvas(), 16.0 / 9.0);

        assert_eq!(shown.len(), 2);
        assert_eq!(shown[0].clip, top, "the upper track should come first");
        assert_eq!(shown[1].clip, bottom);
    }

    /// And a click in the middle finds the top one, not the one beneath it.
    #[test]
    fn a_click_lands_on_the_clip_that_is_drawn_over_the_others() {
        let (editor, [_, top]) = stacked();
        let shown = visible_boxes(&editor, &|_| None, canvas(), 16.0 / 9.0);

        let hit = topmost_at(&shown, canvas().center()).expect("something under the pointer");
        assert_eq!(hit.clip, top);
    }

    /// A hidden track is not on screen, so it must not be clickable either —
    /// otherwise clicking the picture selects something invisible.
    #[test]
    fn a_hidden_track_cannot_be_picked() {
        let (mut editor, [_, top]) = stacked();
        let top_track = editor.active_sequence().expect("sequence").video_tracks[1].id;
        editor
            .set_track_flag(top_track, bettercut_editor_core::TrackFlag::Enabled, false)
            .expect("hide");

        let shown = visible_boxes(&editor, &|_| None, canvas(), 16.0 / 9.0);
        assert_eq!(shown.len(), 1, "the hidden track should be gone");
        assert_ne!(shown[0].clip, top);
    }

    /// Clicking the canvas outside every picture is not a pick. The caller
    /// treats that as "clear the selection", which only makes sense if nothing
    /// is reported.
    #[test]
    fn a_click_outside_every_picture_finds_nothing() {
        let (editor, _) = stacked();
        let shown = visible_boxes(&editor, &|_| None, canvas(), 16.0 / 9.0);

        // Far outside the canvas entirely.
        assert!(topmost_at(&shown, egui::pos2(-500.0, -500.0)).is_none());
    }

    /// With the playhead past the clips there is nothing on screen, so there is
    /// nothing to select and nothing to draw handles around.
    #[test]
    fn nothing_is_shown_when_the_playhead_is_past_the_clips() {
        let (mut editor, _) = stacked();
        editor.set_playhead(TimelineTime::from_seconds(30));

        assert!(visible_boxes(&editor, &|_| None, canvas(), 16.0 / 9.0).is_empty());
    }
}

#[cfg(test)]
mod status_bar_tests {
    use super::autosave_warning;

    /// One failure is a hiccup — an antivirus holding the file for a moment.
    /// A run of them is a disk that is not coming back, and the two must not
    /// read the same: the first is worth a glance, the second means stop and
    /// save somewhere else.
    #[test]
    fn the_autosave_warning_distinguishes_a_hiccup_from_a_dead_disk() {
        assert_eq!(autosave_warning(1), "autosave failed once");

        let many = autosave_warning(40);
        assert!(many.contains("NOT autosaving"), "too quiet: {many}");
        assert!(many.contains("40"), "did not say how bad it is: {many}");
    }
}

#[cfg(test)]
mod copy_name_tests {
    use super::copy_file_name;

    #[test]
    fn a_copy_is_named_after_its_project() {
        assert_eq!(copy_file_name("Holiday cut"), "Holiday cut copy.vproj");
        assert_eq!(copy_file_name("   "), "Untitled copy.vproj");
    }
}

/// Whether `range` is baked, and the bake is still the edit
/// (`editor_core::render_in_place`).
fn range_is_baked(editor: &Editor, range: bettercut_editor_core::timeline::TimelineRange) -> bool {
    editor.active_sequence().is_some_and(|sequence| {
        sequence.renders.iter().any(|render| render.range == range)
            && bettercut_playback::rendered::usable(editor.project(), sequence, range.start)
                .is_some()
    })
}
