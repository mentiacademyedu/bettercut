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
                        .color(theme::DISABLED),
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
                    egui::RichText::new(format!("{name} (not found)")).color(theme::DISABLED)
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
        if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
            save_project(editor, state);
        }
        ui.menu_button("Save…", |ui| {
            if ui
                .button("Save As…")
                .on_hover_text("Save under a new name and keep working in that file (Ctrl+Shift+S)")
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
        });
        if ui
            .button("Export…")
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
        .on_hover_text("A rectangle or an ellipse on the title lane, placed like a title");
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

        // §31: a whole edit from a few clips. Beside the other things that put
        // something on the timeline.
        if ui
            .button("Templates")
            .on_hover_text("Start from a ready-made edit and fill in your clips")
            .clicked()
        {
            state.template_dialog.open();
        }

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
            state.zoom_to_fit(duration, ui.available_width().max(400.0));
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

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                .button("History")
                .on_hover_text("Every step of the edit; click one to go back to it (Ctrl+H)")
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
            ui.label(
                egui::RichText::new(editor.playhead().format_timecode())
                    .monospace()
                    .size(15.0),
            );
        });
    });
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
    ui.heading("Media");
    ui.add_space(4.0);

    if ui
        .button("Import…")
        .on_hover_text("Add a file to the project's media library")
        .clicked()
    {
        import_media(editor, state);
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

    ui.separator();

    if editor.project().media.is_empty() {
        ui.label(
            egui::RichText::new(
                "No media yet. Drag files onto the window, or press Import. Dropped on the timeline, they are added to it too.",
            )
            .color(theme::DISABLED),
        );
        return;
    }

    // Finding one file among dozens: words typed match anywhere in the name,
    // and the kind narrows it further. Unused files can go in one step.
    ui.add(
        egui::TextEdit::singleline(&mut state.media_search)
            .hint_text("Search files")
            .desired_width(f32::INFINITY),
    );
    ui.horizontal_wrapped(|ui| {
        for filter in crate::state::MediaFilter::ALL {
            if ui
                .selectable_label(state.media_kind == filter, filter.label())
                .clicked()
            {
                state.media_kind = filter;
            }
        }
    });
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
    let total = editor.project().media.len();

    // Collect first so the list can be drawn while dispatching commands below.
    let assets: Vec<(MediaId, String, bool, bool, MediaTime, bool)> = editor
        .project()
        .media
        .iter()
        .filter(|m| {
            state.media_kind.accepts(m.kind)
                && crate::state::media_name_matches(&m.file_name, &state.media_search)
        })
        .map(|m| {
            (
                m.id,
                m.file_name.clone(),
                m.missing,
                // A photo has no duration and needs none.
                !m.is_still() && m.duration.is_zero(),
                m.duration,
                m.is_still(),
            )
        })
        .collect();

    // §33's auto slideshow, offered only when there is something to make one
    // from. A button that is always there and usually refuses teaches people to
    // ignore it.
    let photos: Vec<MediaId> = assets
        .iter()
        .filter(|(_, _, missing, _, _, still)| *still && !*missing)
        .map(|(id, ..)| *id)
        .collect();
    if photos.len() >= 2 {
        slideshow_menu(ui, editor, state, &photos);
        ui.separator();
    }

    let mut place: Option<MediaId> = None;
    let mut relink: Option<MediaId> = None;
    let mut remove: Option<MediaId> = None;

    // One banner rather than one button per broken asset: media usually moves a
    // folder at a time, so the folder scan is the action that actually fixes
    // the project (§66 "locate folder").
    let missing_count = assets.iter().filter(|a| a.2).count();
    if missing_count > 0 {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(format!("{missing_count} file(s) missing"))
                    .color(theme::ERROR_TEXT),
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

    if assets.len() < total {
        ui.label(
            egui::RichText::new(if assets.is_empty() {
                "No file matches".to_owned()
            } else {
                format!("{} of {total} files", assets.len())
            })
            .small()
            .color(theme::DISABLED),
        );
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, name, missing, no_duration, duration, still) in &assets {
            ui.group(|ui| {
                thumbnail(ui, state, *id, *missing);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(name).strong());
                    if *missing {
                        ui.label(egui::RichText::new("missing").color(theme::ERROR_TEXT));
                    }
                });

                if *still {
                    ui.label(egui::RichText::new("Photo").small().color(theme::DISABLED));
                } else if *no_duration {
                    // A file whose container never declared a duration.
                    ui.label(
                        egui::RichText::new("no duration")
                            .small()
                            .color(theme::DISABLED),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(
                            TimelineTime::from_ticks(duration.ticks()).format_timecode(),
                        )
                        .small()
                        .monospace()
                        .color(theme::DISABLED),
                    );
                }

                // §66: a missing file needs a way back, right where the problem
                // is visible. Without it the project is simply broken.
                if *missing && ui.button("Locate…").clicked() {
                    relink = Some(*id);
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
        }
    });

    if let Some(id) = place {
        place_on_timeline(editor, state, id);
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
    painter.rect_filled(rect, 0, theme::BACKGROUND);

    let Some(sequence) = editor.active_sequence() else {
        return;
    };
    let output_aspect = sequence.resolution.aspect_ratio().max(0.01);

    // Letterbox a rect with the sequence's aspect ratio.
    let mut size = egui::vec2(rect.width() - 24.0, (rect.width() - 24.0) / output_aspect);
    if size.y > rect.height() - 24.0 {
        size = egui::vec2((rect.height() - 24.0) * output_aspect, rect.height() - 24.0);
    }
    let canvas = egui::Rect::from_center_size(rect.center(), size);

    painter.rect_filled(canvas, 4, egui::Color32::BLACK);
    painter.rect_stroke(
        canvas,
        4,
        egui::Stroke::new(1.0, theme::GRID_LINE),
        egui::StrokeKind::Outside,
    );

    let has_content = preview.is_some_and(crate::Preview::has_content);

    // The composited frame, painted directly. One line, no copy (§4.1).
    if let Some(preview) = preview
        && has_content
    {
        painter.image(
            preview.texture_id(),
            canvas,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        painter.text(
            canvas.center(),
            egui::Align2::CENTER_CENTER,
            format!(
                "{}×{} · {} fps",
                sequence.resolution.width, sequence.resolution.height, sequence.frame_rate
            ),
            egui::FontId::proportional(14.0),
            theme::DISABLED,
        );
        painter.text(
            egui::Pos2::new(canvas.center().x, canvas.center().y + 22.0),
            egui::Align2::CENTER_CENTER,
            "Move the playhead over a clip to see it here",
            egui::FontId::proportional(11.0),
            theme::DISABLED,
        );
    }

    // The eyedropper, before the handles: while it is armed the picture is
    // a colour chart, not a thing to drag, and letting a transform handle take
    // the click first would move the clip instead of keying it.
    if let Some(clip) = state.picking_key {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        painter.rect_stroke(
            canvas,
            4,
            egui::Stroke::new(2.0, theme::SELECTION),
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
        let property = match drag.gesture {
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
            gesture => overlay::property_for(gesture, canvas, shown.box_on_canvas, at),
        };
        let continuing = drag.started;
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
    }

    if response.drag_stopped() {
        state.preview_drag = None;
    }
}

/// A clip on screen right now, and where its picture is.
#[derive(Clone, Copy)]
struct ShownClip {
    clip: bettercut_editor_core::foundation::ClipId,
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

        shown.push(ShownClip {
            clip: clip.id,
            transform,
            box_on_canvas: overlay::to_canvas(
                overlay::clip_box(source_aspect, output_aspect, look.crop, transform),
                canvas,
            ),
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
    ui.heading("Inspector");
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

    ui.label(egui::RichText::new("Sequence").strong());
    ui.monospace(format!("name      {name}"));
    ui.monospace(format!("frame     {ticks} ticks"));
    ui.monospace(format!("duration  {}", duration.format_timecode()));
    ui.monospace(format!("clips     {clips}"));

    sequence_format(ui, editor, state, resolution, rate);

    if editor.active_sequence().is_none() {
        return;
    }

    ui.separator();
    ui.label(egui::RichText::new("Selection").strong());

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
                .color(theme::DISABLED),
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
                .map_or_else(|| "(missing)".to_owned(), |m| m.file_name.clone());
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
                                    .color(theme::DISABLED),
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
                    .color(theme::DISABLED),
            );
        }
    }

    let Some(sequence) = editor.active_sequence() else {
        return;
    };

    ui.separator();
    ui.label(egui::RichText::new("Tracks").strong());

    // Read the switch states, then dispatch after the borrow ends.
    let tracks: Vec<(TrackId, String, bool, bool)> = sequence
        .video_tracks
        .iter()
        .map(|t| (t.id, t.name.clone(), t.enabled, t.locked))
        .chain(
            sequence
                .audio_tracks
                .iter()
                .map(|t| (t.id, t.name.clone(), t.enabled, t.locked)),
        )
        .collect();

    let mut toggles: Vec<(TrackId, TrackFlag, bool)> = Vec::new();

    for (id, name, enabled, locked) in &tracks {
        ui.horizontal(|ui| {
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

            ui.horizontal(|ui| {
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
                    .color(theme::DISABLED),
            );

            if let Some(change) = change
                && let Err(err) =
                    editor.dispatch(bettercut_editor_core::Command::ChangeSetting { change })
            {
                state.error(err.to_string());
            }
        });

    ui.separator();
    ui.collapsing("System", |ui| {
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
                    .color(theme::DISABLED),
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
                ui.monospace(egui::RichText::new(dropped).color(theme::ERROR_TEXT))
                    .on_hover_text("§47a.4: frames too late to show. The machine is behind.");
            } else {
                ui.monospace(dropped);
            }

            let underruns = format!("underruns  {}", stats.underruns);
            if stats.underruns > 0 {
                ui.monospace(egui::RichText::new(underruns).color(theme::ERROR_TEXT))
                    .on_hover_text("§20a: the audio device ran dry. This is audible.");
            } else {
                ui.monospace(underruns);
            }

            if stats.limited_samples > 0 {
                ui.monospace(
                    egui::RichText::new(format!("clipped    {}", stats.limited_samples))
                        .color(theme::ERROR_TEXT),
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
                        .color(theme::ERROR_TEXT),
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
        let mut seek_to: Option<TimelineTime> = None;
        let mut toggle_play = false;
        let playhead = editor.playhead();
        let duration = editor
            .active_sequence()
            .map_or(TimelineTime::ZERO, |s| s.duration());
        let at_start = playhead == TimelineTime::ZERO;

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
        // J and L's speeds, which the Play button alone cannot show.
        let rate = preview.as_ref().map_or(0, |p| p.shuttle_rate());
        if rate != 0 && rate != 1 {
            ui.label(
                egui::RichText::new(crate::shuttle::describe(rate))
                    .monospace()
                    .color(theme::PLAYHEAD),
            )
            .on_hover_text("J / K / L: reverse, stop, forward — press again to go faster");
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

        ui.separator();
        ui.label(
            egui::RichText::new(playhead.format_timecode())
                .monospace()
                .size(14.0),
        );
        ui.label(
            egui::RichText::new(format!("/ {}", duration.format_timecode()))
                .monospace()
                .small()
                .color(theme::DISABLED),
        );

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

        // §20a: what is going to the device, right now.
        if let Some(stats) = state.playback {
            ui.separator();
            draw_meter(ui, stats.peaks, stats.limited_samples > 0);
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
fn draw_meter(ui: &mut egui::Ui, (left, right): (f32, f32), clipping: bool) {
    const WIDTH: f32 = 86.0;
    const BAR: f32 = 5.0;

    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(WIDTH, BAR * 2.0 + 3.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    for (index, peak) in [left, right].into_iter().enumerate() {
        let top = rect.top() + index as f32 * (BAR + 3.0);
        let track = egui::Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(WIDTH, BAR));
        painter.rect_filled(track, 1.0, theme::DISABLED.gamma_multiply(0.35));

        let filled = WIDTH * meter_fraction(peak);
        if filled > 0.5 {
            let colour = if clipping || peak >= 1.0 {
                theme::PLAYHEAD
            } else if peak > 0.7 {
                theme::SELECTION
            } else {
                theme::AUDIO_CLIP_TOP
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

/// One asset's poster image, or a placeholder of the same size.
///
/// The placeholder matters: without it the row height changes the moment a
/// thumbnail finishes generating, and the whole list jumps under the pointer.
fn thumbnail(ui: &mut egui::Ui, state: &mut UiState, media: MediaId, missing: bool) {
    let width = ui.available_width().min(180.0);
    // 16:9 is only a guess for the placeholder — a real thumbnail draws at its
    // own aspect, letterboxed into this box rather than stretched.
    let size = egui::vec2(width, width * 9.0 / 16.0);

    let texture = state.thumbnails.texture(ui.ctx(), media).cloned();
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());

    match texture {
        Some(handle) if !missing => {
            let image = handle.size_vec2();
            let scale = (rect.width() / image.x).min(rect.height() / image.y);
            let drawn = egui::Rect::from_center_size(rect.center(), image * scale);
            ui.painter()
                .rect_filled(rect, 4, theme::TIMELINE_BACKGROUND);
            ui.painter().image(
                handle.id(),
                drawn,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        _ => {
            ui.painter()
                .rect_filled(rect, 4, theme::TIMELINE_BACKGROUND);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if missing { "missing" } else { "…" },
                egui::FontId::proportional(12.0),
                theme::DISABLED,
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
                |ui| ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).text("opacity")),
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
                    ui.horizontal(|ui| {
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
                ui.add(
                    egui::Slider::new(&mut scale, 0.05..=4.0)
                        .logarithmic(true)
                        .text("scale"),
                )
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
                ui.add(egui::Slider::new(&mut rotation, -180.0..=180.0).text("rotation"))
            });
            if response.changed() {
                change = Some((ClipProperty::Rotation(rotation), response.dragged()));
            }

            let mut blur = master.blur;
            let response = master_row(ui, ClipProperty::Blur(master.blur), &mut reset, |ui| {
                ui.add(
                    egui::Slider::new(&mut blur, 0.0..=bettercut_editor_core::timeline::MAX_BLUR)
                        .text("blur")
                        .suffix("%"),
                )
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
                    .color(theme::DISABLED),
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
                ui.add(egui::Slider::new(&mut brightness, 0.0..=2.0).text("brightness"))
            });
            if response.changed() {
                change = Some((ClipProperty::Brightness(brightness), response.dragged()));
            }

            let mut contrast = master.color.contrast;
            let current = ClipProperty::Contrast(master.color.contrast);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(egui::Slider::new(&mut contrast, 0.0..=2.0).text("contrast"))
            });
            if response.changed() {
                change = Some((ClipProperty::Contrast(contrast), response.dragged()));
            }

            let mut saturation = master.color.saturation;
            let current = ClipProperty::Saturation(master.color.saturation);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(egui::Slider::new(&mut saturation, 0.0..=2.0).text("saturation"))
            });
            if response.changed() {
                change = Some((ClipProperty::Saturation(saturation), response.dragged()));
            }

            let mut temperature = master.color.temperature;
            let current = ClipProperty::Temperature(master.color.temperature);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(
                    egui::Slider::new(&mut temperature, -1.0..=1.0)
                        .text("temperature")
                        .custom_formatter(|v, _| warmth_label(v, "cool", "warm")),
                )
            });
            if response.changed() {
                change = Some((ClipProperty::Temperature(temperature), response.dragged()));
            }

            let mut tint = master.color.tint;
            let current = ClipProperty::Tint(master.color.tint);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(
                    egui::Slider::new(&mut tint, -1.0..=1.0)
                        .text("tint")
                        .custom_formatter(|v, _| warmth_label(v, "green", "magenta")),
                )
            });
            if response.changed() {
                change = Some((ClipProperty::Tint(tint), response.dragged()));
            }

            // Here and not on a clip: a vignette frames the frame.
            let mut vignette = master.vignette * 100.0;
            let current = ClipProperty::Vignette(master.vignette);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(
                    egui::Slider::new(&mut vignette, 0.0..=100.0)
                        .text("vignette")
                        .suffix("%"),
                )
                .on_hover_text("Darken the edges of the frame, leaving the middle as it is")
            });
            if response.changed() {
                change = Some((ClipProperty::Vignette(vignette / 100.0), response.dragged()));
            }

            let mut grain = master.grain * 100.0;
            let current = ClipProperty::Grain(master.grain);
            let response = master_row(ui, current, &mut reset, |ui| {
                ui.add(
                    egui::Slider::new(&mut grain, 0.0..=100.0)
                        .text("grain")
                        .suffix("%"),
                )
                .on_hover_text("Film grain over the whole picture, moving every frame")
            });
            if response.changed() {
                change = Some((ClipProperty::Grain(grain / 100.0), response.dragged()));
            }

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
                ui.add(
                    egui::Slider::new(
                        &mut volume,
                        0.0..=bettercut_editor_core::timeline::sequence::MAX_MASTER_VOLUME,
                    )
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                    .text("volume"),
                )
            });
            if response.changed() {
                change = Some((ClipProperty::Gain(volume), response.dragged()));
            }
            ui.label(
                egui::RichText::new(
                    "Applies to the export too. Select a clip to set its own volume.",
                )
                .small()
                .color(theme::DISABLED),
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
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        let result = control(ui);

        let changed = !current.is_default();
        let button = egui::Button::new(egui::RichText::new("\u{21ba}").color(if changed {
            theme::CLIP_TEXT
        } else {
            theme::DISABLED
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
    ui.label(egui::RichText::new(message).color(theme::DISABLED));
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
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            if ui
                .add_enabled(
                    !animated,
                    egui::Slider::new(&mut strength, 0.0..=1.0)
                        .text("strength")
                        .fixed_decimals(2),
                )
                .on_hover_text("How much of the look to apply")
                .changed()
            {
                chosen = Some(ColorAdjust::IDENTITY.lerp(full, strength));
            }
        });
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
        |ui| ui.add(egui::Slider::new(&mut brightness, 0.0..=2.0).text("brightness")),
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
        |ui| ui.add(egui::Slider::new(&mut contrast, 0.0..=2.0).text("contrast")),
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
        |ui| ui.add(egui::Slider::new(&mut saturation, 0.0..=2.0).text("saturation")),
    );
    if response.changed() {
        change = Some((ClipProperty::Saturation(saturation), response.dragged()));
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
            ui.add(
                egui::Slider::new(&mut temperature, -1.0..=1.0)
                    .text("temperature")
                    .custom_formatter(|v, _| warmth_label(v, "cool", "warm")),
            )
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
            ui.add(
                egui::Slider::new(&mut tint, -1.0..=1.0)
                    .text("tint")
                    .custom_formatter(|v, _| warmth_label(v, "green", "magenta")),
            )
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
        .color(theme::DISABLED),
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
            .color(theme::DISABLED),
        );
        return;
    }

    animation_summary(ui, editor, state, clip, look);

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
                ui.label(egui::RichText::new("on").small().color(theme::MARKER));
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
        ui.horizontal(|ui| {
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
        ui.horizontal(|ui| {
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
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label("blend");
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
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label("mask");
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

            ui.horizontal(|ui| {
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
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label("size");
                    moved |= ui
                        .add(
                            egui::DragValue::new(&mut next.size[0])
                                .speed(0.005)
                                .prefix("w "),
                        )
                        .changed();
                    moved |= ui
                        .add(
                            egui::DragValue::new(&mut next.size[1])
                                .speed(0.005)
                                .prefix("h "),
                        )
                        .changed();
                });
            }

            ui.horizontal(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(
                        egui::Slider::new(&mut next.feather, 0.0..=1.0)
                            .text("feather")
                            .fixed_decimals(2),
                    )
                    .on_hover_text("How far the edge takes to fade out")
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(
                        egui::Slider::new(&mut next.rotation_degrees, -180.0..=180.0)
                            .text("angle")
                            .suffix("°"),
                    )
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
        let key = editor.video_clip(clip).and_then(|clip| clip.chroma_key);
        let mut on = key.is_some();
        ui.horizontal(|ui| {
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

            ui.horizontal(|ui| {
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
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(
                        egui::Slider::new(&mut next.tolerance, 0.0..=max)
                            .text("tolerance")
                            .fixed_decimals(2),
                    )
                    .on_hover_text("How far from that colour still counts as screen")
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(
                        egui::Slider::new(&mut next.softness, 0.0..=max)
                            .text("softness")
                            .fixed_decimals(2),
                    )
                    .on_hover_text("How quickly the edge gives way — what saves hair")
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                moved |= ui
                    .add(
                        egui::Slider::new(&mut next.spill, 0.0..=1.0)
                            .text("spill")
                            .fixed_decimals(2),
                    )
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
        let (split, glitch) = editor
            .video_clip(clip)
            .map_or((0.0, 0.0), |clip| (clip.rgb_split, clip.glitch));
        let max = bettercut_editor_core::timeline::MAX_GLITCH;
        let (mut next_split, mut next_glitch) = (split, glitch);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let response = ui
                .add(
                    egui::Slider::new(&mut next_split, 0.0..=max)
                        .text("RGB split")
                        .suffix("%"),
                )
                .on_hover_text("Pull the red and blue apart, like a cheap lens or an old tape");
            if response.changed() {
                change = Some((ClipProperty::RgbSplit(next_split), response.dragged()));
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let response = ui
                .add(
                    egui::Slider::new(&mut next_glitch, 0.0..=max)
                        .text("glitch")
                        .suffix("%"),
                )
                .on_hover_text("Throw bands of the picture sideways, different ones every frame");
            if response.changed() {
                change = Some((ClipProperty::Glitch(next_glitch), response.dragged()));
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
        |ui| ui.add(egui::Slider::new(&mut value, 0.0..=1.0).text("opacity")),
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
            ui.add(
                egui::Slider::new(&mut scale, 0.05..=4.0)
                    .logarithmic(true)
                    .text("scale"),
            )
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
        ui.horizontal(|ui| {
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
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label("behind");
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
                    })
                    .clicked()
                    && current != option
                {
                    change = Some((ClipProperty::Backdrop(option), false));
                }
            }
        });
    }

    // A slow zoom, written as keyframes (§24). Beside the scale it animates,
    // because that is what it changes — and what to adjust afterwards.
    let current = editor.movement_of(clip);
    let mut movement = None;
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label("movement");
        for option in Movement::ALL {
            if ui
                .add(egui::Button::selectable(
                    current == Some(option),
                    match option {
                        Movement::None => "None",
                        Movement::ZoomIn => "Zoom in",
                        Movement::ZoomOut => "Zoom out",
                    },
                ))
                .on_hover_text(match option {
                    Movement::None => "No movement; any zoom keyframes are removed",
                    Movement::ZoomIn => "Grow slowly across the clip",
                    Movement::ZoomOut => "Start close and pull back",
                })
                .clicked()
            {
                movement = Some(option);
            }
        }
    });
    if let Some(movement) = movement {
        match editor.set_movement(clip, movement) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }

    // Camera shake, also written as keys: beside the movement because both are
    // generated motion, and both are adjusted afterwards in the same place.
    let shake = editor.shake_of(clip);
    let mut shake_choice: Option<Option<bettercut_editor_core::timeline::ShakeStrength>> = None;
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label("shake");
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

    let mut rotation = transform.rotation_degrees;
    let response = keyed_row(
        ui,
        &look,
        ClipProperty::Rotation(transform.rotation_degrees),
        &mut toggle,
        &mut reset,
        |ui| ui.add(egui::Slider::new(&mut rotation, -180.0..=180.0).text("rotation")),
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
                    let response = ui.add(
                        egui::Slider::new(&mut percent, 0.0..=95.0)
                            .text(label)
                            .suffix("%"),
                    );
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

    // Dragging the edges on the picture, which is how a crop is actually made;
    // the sliders above are for the exact number afterwards.
    ui.horizontal(|ui| {
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
            }
            state.needs_repaint = true;
        }
    });

    // Beside the rotation, because both answer "which way is this shot facing".
    // Toggles rather than a -1 in the scale: mirroring is not a size, and a
    // clip covers the same part of the frame either way.
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label("mirror");
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
            ui.add(
                egui::Slider::new(&mut amount, 0.0..=bettercut_editor_core::timeline::MAX_BLUR)
                    .text("blur")
                    .suffix("%"),
            )
        },
    );
    if response.changed() {
        change = Some((ClipProperty::Blur(amount), response.dragged()));
    }
    if amount > 0.0 {
        // §45 rates blur Medium and §44 says to avoid expensive realtime
        // effects on weak hardware. Saying so where the slider is beats
        // leaving the user to wonder why playback got choppy.
        ui.label(
            egui::RichText::new("Blur costs more than the controls above; playback may drop.")
                .small()
                .color(theme::DISABLED),
        );
    }

    // Beside the blur, as its opposite. Not keyed — a sharpen that eases in is
    // not something anyone asks for — so a plain row with its own reset.
    let sharpen = editor.video_clip(clip).map_or(0.0, |c| c.sharpen);
    let mut sharpened = sharpen;
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        let response = ui
            .add(
                egui::Slider::new(
                    &mut sharpened,
                    0.0..=bettercut_editor_core::timeline::MAX_SHARPEN,
                )
                .text("sharpen")
                .suffix("%"),
            )
            .on_hover_text("Bring out edges and fine detail, leaving flat areas as they are");
        if response.changed() {
            change = Some((ClipProperty::Sharpen(sharpened), response.dragged()));
        }
        let touched = !ClipProperty::Sharpen(sharpen).is_default();
        if ui
            .add_enabled(
                touched,
                egui::Button::new(egui::RichText::new("\u{21ba}").color(if touched {
                    theme::CLIP_TEXT
                } else {
                    theme::DISABLED
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

    clip_lut(ui, editor, state, clip);

    clip_transition(ui, editor, state, clip);
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

    ui.horizontal(|ui| {
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
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            let response = ui.add(
                egui::Slider::new(&mut strength, 0.0..=100.0)
                    .text("LUT strength")
                    .suffix("%"),
            );
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
            .color(theme::DISABLED),
    );
    ui.add_space(2.0);

    let mut chosen: Option<Option<TransitionKind>> = None;
    ui.horizontal(|ui| {
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
        let response = ui.add(
            egui::Slider::new(
                &mut seconds,
                MIN_TRANSITION.ticks() as f64 / 960_000.0..=limit.ticks() as f64 / 960_000.0,
            )
            .text("seconds")
            .fixed_decimals(2),
        );
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
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Speed").strong());
        ui.label(
            egui::RichText::new(format!("{:.2}×", speed.as_f64()))
                .monospace()
                .color(theme::KEYFRAME),
        );
    });
    ui.label(
        egui::RichText::new(format!(
            "Plays {} of footage in {}",
            TimelineTime::from_ticks(existing.source.duration().ticks()).format_timecode(),
            length.format_timecode()
        ))
        .small()
        .color(theme::DISABLED),
    );

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
    let response = ui.add(
        egui::Slider::new(
            &mut factor,
            bettercut_editor_core::timeline::MIN_SPEED.as_f64()
                ..=bettercut_editor_core::timeline::MAX_SPEED.as_f64(),
        )
        .logarithmic(true)
        .fixed_decimals(2)
        .suffix("×")
        .text("rate"),
    );
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
        .color(theme::DISABLED),
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
            .color(theme::DISABLED),
    );

    if let Some(speed) = chosen {
        match editor.set_clip_speed(clip, speed, dragging) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }
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
        .color(theme::DISABLED),
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
    note(ui.add(egui::Slider::new(&mut look.color.brightness, 0.0..=2.0).text("brightness")));
    note(ui.add(egui::Slider::new(&mut look.color.contrast, 0.0..=2.0).text("contrast")));
    note(ui.add(egui::Slider::new(&mut look.color.saturation, 0.0..=2.0).text("saturation")));
    note(
        ui.add(
            egui::Slider::new(&mut look.color.temperature, -1.0..=1.0)
                .text("temperature")
                .custom_formatter(|v, _| warmth_label(v, "cool", "warm")),
        ),
    );
    note(
        ui.add(
            egui::Slider::new(&mut look.color.tint, -1.0..=1.0)
                .text("tint")
                .custom_formatter(|v, _| warmth_label(v, "green", "magenta")),
        ),
    );

    ui.add_space(4.0);
    note(
        ui.add(
            egui::Slider::new(
                &mut look.blur,
                0.0..=bettercut_editor_core::timeline::MAX_BLUR,
            )
            .text("blur")
            .suffix("%"),
        ),
    );
    let mut vignette = look.vignette * 100.0;
    let response = ui
        .add(
            egui::Slider::new(&mut vignette, 0.0..=100.0)
                .text("vignette")
                .suffix("%"),
        )
        .on_hover_text("Darken the edges of the frame while this adjustment runs");
    if response.changed() {
        look.vignette = vignette / 100.0;
    }
    note(response);
    let mut grain = look.grain * 100.0;
    let response = ui
        .add(
            egui::Slider::new(&mut grain, 0.0..=100.0)
                .text("grain")
                .suffix("%"),
        )
        .on_hover_text("Film grain over the frame while this adjustment runs");
    if response.changed() {
        look.grain = grain / 100.0;
    }
    note(response);
    // As a percentage, because "how much of this grade" is read as a share.
    let mut strength = look.strength * 100.0;
    let response = ui.add(
        egui::Slider::new(&mut strength, 0.0..=100.0)
            .text("strength")
            .suffix("%"),
    );
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
        ui.horizontal(|ui| {
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
            let response = ui.add(
                egui::Slider::new(value, 8.0..=3840.0)
                    .logarithmic(true)
                    .text(label)
                    .suffix(" px"),
            );
            if response.changed() {
                dragging |= response.dragged();
                reshaped = true;
            }
        };
        size(ui, &mut shape.width, "width");
        size(ui, &mut shape.height, "height");
        if shape.kind == bettercut_editor_core::text::ShapeKind::RoundedRectangle {
            let response =
                ui.add(egui::Slider::new(&mut shape.corner_radius, 0.0..=400.0).text("corners"));
            if response.changed() {
                dragging |= response.dragged();
                reshaped = true;
            }
        }
        ui.horizontal(|ui| {
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
            ui.horizontal(|ui| {
                if colour_button(ui, &mut outline.color) {
                    reshaped = true;
                }
                let response =
                    ui.add(egui::Slider::new(&mut outline.width, 1.0..=120.0).text("width"));
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
            ui.horizontal(|ui| {
                for direction in bettercut_editor_core::timeline::CountDirection::ALL {
                    if ui
                        .selectable_label(counter.direction == direction, direction.label())
                        .clicked()
                    {
                        counter.direction = direction;
                    }
                }
            });
            ui.horizontal(|ui| {
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
            ui.horizontal(|ui| {
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
        }

        // A whole look in one click: the style and where it sits, which for a lower
        // third is half of what it is.
        ui.add_space(8.0);
        ui.horizontal(|ui| {
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

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Font").strong());

        ui.horizontal(|ui| {
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

        ui.horizontal(|ui| {
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
        let mut row = |ui: &mut egui::Ui, widget: egui::Slider<'_>| {
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
            egui::Slider::new(
                &mut style.size,
                bettercut_editor_core::text::MIN_SIZE..=bettercut_editor_core::text::MAX_SIZE,
            )
            .logarithmic(true)
            .text("size"),
        );
        restyled |= row(
            ui,
            egui::Slider::new(&mut style.line_height, 0.5..=3.0).text("line spacing"),
        );
        restyled |= row(
            ui,
            egui::Slider::new(&mut style.letter_spacing, -0.2..=1.0).text("letter spacing"),
        );

        ui.horizontal(|ui| {
            ui.label("colour");
            if colour_button(ui, &mut style.color) {
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

        ui.horizontal(|ui| {
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
            .color(theme::DISABLED),
        );

        let mut outlined = style.stroke.is_some();
        if ui.checkbox(&mut outlined, "outline").changed() {
            style.stroke = outlined.then(Stroke::default);
            restyled = true;
        }
        if let Some(stroke) = &mut style.stroke {
            ui.horizontal(|ui| {
                if colour_button(ui, &mut stroke.color) {
                    restyled = true;
                }
                let response =
                    ui.add(egui::Slider::new(&mut stroke.width, 0.0..=20.0).text("width"));
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
            ui.horizontal(|ui| {
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
            let response = ui.add(egui::Slider::new(&mut shadow.blur, 0.0..=60.0).text("blur"));
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
            ui.horizontal(|ui| {
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
    ui.horizontal(|ui| {
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
    let response = ui.add(
        egui::Slider::new(&mut scale, 0.05..=4.0)
            .logarithmic(true)
            .text("scale"),
    );
    if response.changed() {
        change = Some((
            TextProperty::Scale { x: scale, y: scale },
            response.dragged(),
        ));
    }

    let mut rotation = transform.rotation_degrees;
    let response = ui.add(egui::Slider::new(&mut rotation, -180.0..=180.0).text("rotation"));
    if response.changed() {
        change = Some((TextProperty::Rotation(rotation), response.dragged()));
    }

    let mut value = opacity;
    let response = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).text("opacity"));
    if response.changed() {
        change = Some((TextProperty::Opacity(value), response.dragged()));
    }

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Animation").strong());
    if text_animation(ui, &mut animation) {
        change = Some((TextProperty::Animation(animation), false));
    }

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
        ui.horizontal(|ui| {
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
                .color(theme::DISABLED),
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
    ui.horizontal(|ui| {
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
            theme::KEYFRAME
        } else {
            theme::DISABLED
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

        let result = control(ui);

        // Reset last, at the far end of the row: it is the least-used control
        // here and putting it in the reading path would slow every other edit
        // down. Disabled rather than hidden when there is nothing to undo, so
        // the row does not change width as values change and the button does
        // not appear under a cursor that was aiming at the slider.
        let changed = !current.is_default() || look.row(current).animated;
        let button = egui::Button::new(egui::RichText::new("↺").color(if changed {
            theme::CLIP_TEXT
        } else {
            theme::DISABLED
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
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(if count == 1 {
                "1 keyframe".to_owned()
            } else {
                format!("{count} keyframes")
            })
            .small()
            .color(theme::KEYFRAME),
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
    let response = ui.add(egui::Slider::new(&mut value, 0.0..=2.0).text("volume"));
    if response.changed() {
        apply_clip_property(
            editor,
            state,
            clip,
            ClipProperty::Gain(value),
            response.dragged(),
        );
    }

    // Beside the volume: both are about how this recording sounds.
    let denoise = editor.audio_clip(clip).map_or(0.0, |c| c.denoise);
    let mut cleaned = denoise;
    let response = ui
        .add(
            egui::Slider::new(
                &mut cleaned,
                0.0..=bettercut_editor_core::timeline::MAX_DENOISE,
            )
            .text("voice clean-up")
            .suffix("%"),
        )
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
        let response = ui.add(
            egui::Slider::new(value, 0..=100)
                .custom_formatter(|v, _| {
                    if v == 0.0 {
                        "none".to_owned()
                    } else {
                        format!("{:.1} s", v / 10.0)
                    }
                })
                .text(label),
        );
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
}

fn apply_clip_property(
    editor: &mut Editor,
    state: &mut UiState,
    clip: bettercut_editor_core::foundation::ClipId,
    property: bettercut_editor_core::ClipProperty,
    continuing: bool,
) {
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
    ui.horizontal(|ui| {
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
            .color(theme::DISABLED),
        );
    }
    ui.horizontal(|ui| {
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

    ui.horizontal(|ui| {
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

    ui.horizontal(|ui| {
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
                .color(theme::DISABLED),
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
            .color(theme::DISABLED),
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
    ui.horizontal(|ui| {
        match &state.status {
            Some(message) if message.is_error => {
                ui.label(egui::RichText::new(&message.text).color(theme::ERROR_TEXT));
            }
            Some(message) => {
                ui.label(egui::RichText::new(&message.text).color(theme::OK_TEXT));
            }
            None => {
                ui.label(
                    egui::RichText::new(
                        "S split · Del delete · Shift+Del ripple · Ctrl+D duplicate · \
                         N snap · drag edges to trim",
                    )
                    .color(theme::DISABLED),
                );
            }
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
            ui.label(egui::RichText::new(parts.join(" · ")).color(theme::ERROR_TEXT))
                .on_hover_text(
                    "Playback is not keeping up. Inspector → System has the full \
                     counters; lowering Proxies → Quality is the usual fix.",
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
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(140.0)
                    .text(format!("Exporting {:.0}%", fraction * 100.0)),
            )
            .on_hover_text("Rendering your timeline to a video file.");
            if ui
                .button("Stop")
                .on_hover_text("Cancel the export. The partial file is removed.")
                .clicked()
            {
                state.export_stop_requested = true;
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let path = editor
                .path()
                .map_or_else(|| "unsaved".to_owned(), |p| p.display().to_string());
            ui.label(egui::RichText::new(path).small().color(theme::DISABLED));
            ui.separator();
            ui.label(
                egui::RichText::new(format!("{} clips", editor.project().clip_count()))
                    .small()
                    .color(theme::DISABLED),
            );
            if editor.is_dirty() {
                ui.separator();
                ui.label(
                    egui::RichText::new("unsaved changes")
                        .small()
                        .color(theme::SELECTION),
                );
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
                        .color(theme::ERROR_TEXT),
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
    if editor.is_dirty() {
        state.error("Save the current project first — unsaved changes would be lost.");
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
    if editor.is_dirty() {
        state.error("Save the current project first — unsaved changes would be lost.");
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
    if editor.is_dirty() {
        state.error("Save the current project first — unsaved changes would be lost.");
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
            reset_state(state);
            state.recent.touch(path);
            state.info(format!("Opened {}", path.display()));
        }
        Err(err) => state.error(format!("Could not open project: {err}")),
    }
}

/// A fresh interface for a different project, keeping what belongs to the
/// person rather than the project: the recent list.
fn reset_state(state: &mut UiState) {
    let recent = std::mem::take(&mut state.recent);
    *state = UiState::default();
    state.recent = recent;
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
                "jpeg", "webp", "bmp", "gif", "tif", "tiff",
            ],
        )
        .pick_files()
    else {
        return;
    };

    import_paths(editor, state, &paths);
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
            egui::Slider::new(
                &mut seconds,
                MIN_HOLD.ticks() / TICKS_PER_SECOND..=MAX_HOLD.ticks() / TICKS_PER_SECOND,
            )
            .text("seconds each")
            .integer(),
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
        ui.horizontal(|ui| {
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
                .color(theme::DISABLED),
        );

        if ui.button("Make Slideshow").clicked() {
            build = true;
            ui.close();
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
    match editor.place_media(media_id) {
        Ok(clips) if clips.len() > 1 => state.info("Clip added, with its sound"),
        Ok(_) => state.info("Clip added"),
        Err(err) => state.error(err.to_string()),
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
