//! The §58 panels: toolbar, media browser, preview, inspector, status bar.
//!
//! Every control here either reads through `editor.project()` or dispatches a
//! command. None of them mutate project data (§54).

use bettercut_editor_core::foundation::{FrameRate, MediaId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::project_format::PerformanceMode;
use bettercut_editor_core::timeline::{Resolution, SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor, SettingChange, TrackFlag};

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
        if ui.button("Open…").clicked() {
            open_project(editor, state);
        }
        if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
            save_project(editor, state);
        }

        ui.separator();

        // Plain words and ASCII, not arrow glyphs: egui's bundled font does not
        // carry ↶/↷/＋/－, and a missing glyph renders as an empty box. The
        // brief is controls that are easy to understand — the hover text says
        // exactly what will be undone.
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
            ui.label(
                egui::RichText::new(editor.playhead().format_timecode())
                    .monospace()
                    .size(15.0),
            );
        });
    });
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

    if ui
        .button("Add placeholder clip")
        .on_hover_text(
            "Adds a 5-second synthetic asset — useful for laying out a timeline \
             before the real footage is available.",
        )
        .clicked()
    {
        add_placeholder_clip(editor, state);
    }

    ui.separator();

    if editor.project().media.is_empty() {
        ui.label(egui::RichText::new("No media imported yet.").color(theme::DISABLED));
        return;
    }

    // Collect first so the list can be drawn while dispatching commands below.
    let assets: Vec<(MediaId, String, bool, bool, MediaTime)> = editor
        .project()
        .media
        .iter()
        .map(|m| {
            (
                m.id,
                m.file_name.clone(),
                m.missing,
                m.duration.is_zero(),
                m.duration,
            )
        })
        .collect();

    let mut place: Option<MediaId> = None;
    let mut relink: Option<MediaId> = None;

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

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, name, missing, no_duration, duration) in &assets {
            ui.group(|ui| {
                thumbnail(ui, state, *id, *missing);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(name).strong());
                    if *missing {
                        ui.label(egui::RichText::new("missing").color(theme::ERROR_TEXT));
                    }
                });

                if *no_duration {
                    // Still images have no duration, and so does a file whose
                    // container never declared one.
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

                let can_place = !*no_duration && !*missing;
                if ui
                    .add_enabled(can_place, egui::Button::new("Add to timeline"))
                    .on_disabled_hover_text(
                        "This file has no duration to place — it is a still image, \
                         or the file is missing from disk.",
                    )
                    .clicked()
                {
                    place = Some(*id);
                }
            });
        }
    });

    if let Some(id) = place {
        place_on_timeline(editor, state, id);
    }
    if let Some(id) = relink {
        relink_one(editor, state, id);
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
    editor: &Editor,
    _state: &UiState,
    preview: Option<&crate::Preview>,
) {
    let available = ui.available_size();
    let (rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0, theme::BACKGROUND);

    let Some(sequence) = editor.active_sequence() else {
        return;
    };

    // Letterbox a rect with the sequence's aspect ratio.
    let aspect = sequence.resolution.aspect_ratio().max(0.01);
    let mut size = egui::vec2(rect.width() - 24.0, (rect.width() - 24.0) / aspect);
    if size.y > rect.height() - 24.0 {
        size = egui::vec2((rect.height() - 24.0) * aspect, rect.height() - 24.0);
    }
    let canvas = egui::Rect::from_center_size(rect.center(), size);

    painter.rect_filled(canvas, 4, egui::Color32::BLACK);
    painter.rect_stroke(
        canvas,
        4,
        egui::Stroke::new(1.0, theme::GRID_LINE),
        egui::StrokeKind::Outside,
    );

    // The composited frame, painted directly. One line, no copy (§4.1).
    if let Some(preview) = preview
        && preview.has_content()
    {
        painter.image(
            preview.texture_id(),
            canvas,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        return;
    }

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

    let Some(sequence) = editor.active_sequence() else {
        return;
    };

    ui.separator();
    ui.label(egui::RichText::new("Selection").strong());

    let selected: Vec<_> = state.selected_clips.iter().copied().collect();
    if selected.is_empty() {
        ui.label(egui::RichText::new("Nothing selected").color(theme::DISABLED));
    } else if selected.len() == 1 {
        let id = selected[0];
        let found = sequence.video_tracks.iter().find_map(|t| {
            t.get(id).map(|c| {
                (
                    c.timeline.start,
                    c.timeline.duration(),
                    c.opacity,
                    c.media_id,
                )
            })
        });
        match found {
            Some((start, duration, opacity, media_id)) => {
                let name = editor
                    .project()
                    .media_asset(media_id)
                    .map_or_else(|| "(missing)".to_owned(), |m| m.file_name.clone());
                ui.monospace(format!("media     {name}"));
                ui.monospace(format!("start     {}", start.format_timecode()));
                ui.monospace(format!("duration  {}", duration.format_timecode()));
                ui.monospace(format!("opacity   {opacity:.2}"));
            }
            None => {
                ui.label(egui::RichText::new("Audio clip selected").color(theme::DISABLED));
            }
        }
    } else {
        ui.label(format!("{} clips selected", selected.len()));
    }

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
                        // §41 says the interface explains itself.
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
        ui.monospace(format!("processors {}", hardware.logical_processors));
        ui.monospace(format!(
            "~cores     {}",
            hardware.estimated_physical_cores()
        ));
        ui.monospace(format!(
            "mode       {:?}",
            editor.project().settings.performance_mode
        ));
        ui.monospace(format!("heavy jobs {}", hardware.max_heavy_jobs()));
        ui.monospace(format!(
            "ffmpeg     {} thread(s)/job",
            hardware.ffmpeg_threads_per_job()
        ));
        ui.monospace(format!(
            "frame cache {} MB",
            hardware.frame_cache_bytes() / (1024 * 1024)
        ));

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

/// Resolution and frame rate for the active sequence (§8, §36).
///
/// Both go through one command, because "make this a 1080p50 project" is one
/// decision and should be one undo step.
fn sequence_format(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut UiState,
    current: Resolution,
    rate: FrameRate,
) {
    let mut wanted = (current, rate);

    ui.horizontal(|ui| {
        ui.label("size");
        egui::ComboBox::from_id_salt("sequence_resolution")
            .selected_text(format!("{}×{}", current.width, current.height))
            .show_ui(ui, |ui| {
                // §36's preset shapes plus the two common landscape sizes. A
                // custom size still round-trips through the project file; this
                // is a shortcut, not a restriction.
                let presets = [
                    (Resolution::HD_1080, "1920×1080", "Landscape 16:9"),
                    (Resolution::HD_720, "1280×720", "Landscape 16:9, smaller"),
                    (
                        Resolution::VERTICAL_1080,
                        "1080×1920",
                        "Vertical 9:16 — Shorts, TikTok, Reels",
                    ),
                    (
                        Resolution::new(1080, 1080),
                        "1080×1080",
                        "Square 1:1 — feed posts",
                    ),
                    (
                        Resolution::new(3840, 2160),
                        "3840×2160",
                        "4K UHD. Export size; preview still scales down (§16)",
                    ),
                ];
                for (value, label, hint) in presets {
                    if ui
                        .selectable_value(&mut wanted.0, value, label)
                        .on_hover_text(hint)
                        .changed()
                    {
                        wanted.0 = value;
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
pub fn status_bar(ui: &mut egui::Ui, editor: &Editor, state: &UiState) {
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
        });
    });
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
    *state = UiState::default();
    state.info("New project created");
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

    match Editor::open(&path) {
        Ok((opened, _rx)) => {
            *editor = opened;
            *state = UiState::default();
            state.info(format!("Opened {}", path.display()));
        }
        Err(err) => state.error(format!("Could not open project: {err}")),
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
        Ok(()) => state.info("Project saved"),
        Err(err) => state.error(format!("Could not save: {err}")),
    }
}

fn import_media(editor: &mut Editor, state: &mut UiState) {
    let Some(paths) = rfd::FileDialog::new()
        .add_filter(
            "media",
            &[
                "mp4", "mov", "mkv", "webm", "avi", "mp3", "wav", "m4a", "flac", "png", "jpg",
                "jpeg",
            ],
        )
        .pick_files()
    else {
        return;
    };

    let total = paths.len();
    let mut imported = 0;
    let mut failures: Vec<String> = Vec::new();
    let mut adopted = None;

    for path in paths {
        match editor.import_file(&path) {
            Ok(id) => {
                imported += 1;
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

/// Append an asset to the first video track, after everything already there.
fn place_on_timeline(editor: &mut Editor, state: &mut UiState, media_id: MediaId) {
    let Some(sequence) = editor.active_sequence() else {
        state.error("No sequence to add to");
        return;
    };
    let Some(track) = sequence.video_tracks.first() else {
        state.error("No video track — add one first");
        return;
    };
    let (track_id, start) = (track.id, track.duration());

    let Some(asset) = editor.project().media_asset(media_id) else {
        state.error("Media is no longer in the project");
        return;
    };
    let duration = asset.duration;

    let source = match SourceRange::new(MediaTime::ZERO, duration) {
        Ok(range) => range,
        Err(err) => {
            state.error(format!("Cannot place media: {err}"));
            return;
        }
    };

    let clip = match VideoClip::new(media_id, start, source) {
        Ok(clip) => clip,
        Err(err) => {
            state.error(format!("Cannot place media: {err}"));
            return;
        }
    };

    match editor.add_clip(track_id, ClipPayload::Video(Box::new(clip))) {
        Ok(()) => state.info("Clip added"),
        Err(err) => state.error(err.to_string()),
    }
}
