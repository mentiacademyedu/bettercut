//! The §58 panels: toolbar, media browser, preview, inspector, status bar.
//!
//! Every control here either reads through `editor.project()` or dispatches a
//! command. None of them mutate project data (§54).

use bettercut_editor_core::foundation::{FrameRate, MediaId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::project_format::PerformanceMode;
use bettercut_editor_core::timeline::{AnimatedParameter, Resolution, SourceRange, VideoClip};
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

                ui.horizontal(|ui| {
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
    let on_corner = selected
        .zip(pointer)
        .and_then(|(shown, at)| overlay::corner_at(shown.box_on_canvas, at));

    // The cursor says what is under it before it is pressed.
    if on_corner.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    } else if pointer.is_some_and(|at| visible.iter().any(|s| s.box_on_canvas.contains(at))) {
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
        let grabbed = match (selected, on_corner) {
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

    overlay::draw(painter, shown.box_on_canvas, dragging);

    if dragging
        && let Some(at) = pointer
        && let Some(drag) = &mut state.preview_drag
    {
        let property = overlay::property_for(drag.gesture, canvas, shown.box_on_canvas, at);
        let continuing = drag.started;
        drag.started = true;
        // §11: `continuing` after the first frame, so the whole drag is one
        // undo step — and §54, so this goes through a command rather than
        // touching the project.
        if let Err(err) = editor.set_clip_value(shown.clip, property, continuing) {
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
        let transform = clip.look_at(clip.source_time_at(playhead)).transform;
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
                overlay::layer_box(source_aspect, output_aspect, transform),
                canvas,
            ),
        });
    }
    shown
}

/// The frontmost clip whose picture covers `at`.
fn topmost_at(visible: &[ShownClip], at: egui::Pos2) -> Option<ShownClip> {
    visible
        .iter()
        .find(|shown| shown.box_on_canvas.contains(at))
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

    let Some(sequence) = editor.active_sequence() else {
        return;
    };

    ui.separator();
    ui.label(egui::RichText::new("Selection").strong());

    let selected: Vec<_> = state.selected_clips.iter().copied().collect();
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

    match (selected.len(), single) {
        (1, _) if text_selected => text_properties(ui, editor, state, selected[0]),
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
            inspector_tabs(ui, state, true, false);
            ui.add_space(4.0);

            master_properties(ui, editor, state);
        }
        (1, Some((id, video, audio))) => {
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
            inspector_tabs(ui, state, video.is_some(), audio.is_some());
            ui.add_space(4.0);

            match state.inspector_tab {
                InspectorTab::Video => {
                    if let Some(look) = video {
                        clip_video_properties(ui, editor, state, id, look);
                    } else {
                        unavailable(ui, "This clip has no picture.");
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
                    if let Some((_, gain, _)) = audio {
                        clip_audio_properties(ui, editor, state, id, gain);
                    } else {
                        unavailable(ui, "This clip has no sound.");
                    }
                }
                InspectorTab::Speed => unavailable(
                    ui,
                    "Speed changes are not built yet. Clips play at their                      recorded rate.",
                ),
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
    /// Where the picture is and what it looks like: transform, opacity, blur.
    #[default]
    Video,
    /// Brightness, contrast, saturation.
    Colours,
    Audio,
    Speed,
    /// The keyframes on this clip, and a way to move between them.
    Animation,
}

impl InspectorTab {
    pub const ALL: [Self; 5] = [
        Self::Video,
        Self::Colours,
        Self::Audio,
        Self::Speed,
        Self::Animation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
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
                InspectorTab::Video | InspectorTab::Colours | InspectorTab::Animation => has_video,
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
        InspectorTab::Video | InspectorTab::Colours | InspectorTab::Animation => has_video,
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
        }
        InspectorTab::Audio => unavailable(
            ui,
            "There is no whole-video volume yet. Select a clip to set its own.",
        ),
        InspectorTab::Speed => unavailable(
            ui,
            "Speed changes are not built yet. Clips play at their recorded rate.",
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

    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("0 saturation is black and white; 1.0 is untouched.")
            .small()
            .color(theme::DISABLED),
    );

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

    apply_row_actions(editor, state, clip, change, toggle, reset);

    clip_transition(ui, editor, state, clip);
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

/// Everything about one text overlay (§26).
///
/// The style is dispatched as a whole rather than field by field, because that
/// is how the model holds it and how the rasterizer consumes it. A per-field
/// command would buy a finer undo history for a control nobody adjusts one
/// field at a time — and would multiply the number of commands by twelve.
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

    ui.monospace(format!("start     {}", range.start.format_timecode()));
    ui.monospace(format!("duration  {}", range.duration().format_timecode()));
    ui.add_space(6.0);

    // Collected during the draw and dispatched after it, for the same reason
    // every other panel here does: the controls read the project while drawing
    // and each of these borrows it mutably.
    let mut change: Option<(TextProperty, bool)> = None;
    let mut remove = false;

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

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Font").strong());

    let mut restyled = false;

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
            .show_ui(ui, |ui| {
                for (family, label) in [
                    (FontFamily::SansSerif, "Sans"),
                    (FontFamily::Serif, "Serif"),
                    (FontFamily::Monospace, "Mono"),
                ] {
                    if ui.selectable_label(style.family == family, label).clicked() {
                        style.family = family;
                        restyled = true;
                    }
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
        egui::RichText::new("White text over an unknown shot is unreadable about half the time.")
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
            let response = ui.add(egui::Slider::new(&mut stroke.width, 0.0..=20.0).text("width"));
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
    if ui.button("Remove title").clicked() {
        remove = true;
    }

    if let Some((property, continuing)) = change {
        match editor.set_text_property(clip, property, continuing) {
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
        // vanishes is harder to understand than one that explains itself (§41).
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
/// The shapes a project is usually made in (§36).
const ASPECTS: [(&str, (u32, u32), &str); 5] = [
    (
        "16:9",
        (16, 9),
        "Landscape — YouTube, television, most cameras",
    ),
    ("9:16", (9, 16), "Vertical — Shorts, TikTok, Reels"),
    ("1:1", (1, 1), "Square — feed posts"),
    ("4:5", (4, 5), "Portrait — Instagram feed"),
    ("21:9", (21, 9), "Ultrawide — cinematic"),
];

/// Whether a size is that shape, to within rounding.
///
/// Compared as a ratio rather than by exact dimensions: 1920×1080 and 1280×720
/// are both 16:9, and someone who typed 1918×1080 has still chosen landscape.
fn matches_aspect(size: Resolution, ratio: (u32, u32)) -> bool {
    if size.height == 0 || ratio.1 == 0 {
        return false;
    }
    let actual = f64::from(size.width) / f64::from(size.height);
    let wanted = f64::from(ratio.0) / f64::from(ratio.1);
    (actual - wanted).abs() < 0.01
}

/// Reshape to `ratio`, keeping the short edge.
fn with_aspect(size: Resolution, ratio: (u32, u32)) -> Resolution {
    let short = u64::from(size.width.min(size.height).max(2));
    let (num, den) = (u64::from(ratio.0.max(1)), u64::from(ratio.1.max(1)));
    let (w, h) = if num >= den {
        (short * num / den, short)
    } else {
        (short, short * den / num)
    };
    even_size(Resolution::new(w as u32, h as u32))
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
        for (name, ratio, hint) in ASPECTS {
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

#[cfg(test)]
mod sequence_shape_tests {
    use super::*;

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
        for (name, ratio, _) in ASPECTS {
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
        let claims = ASPECTS
            .iter()
            .filter(|(_, ratio, _)| matches_aspect(Resolution::HD_1080, *ratio))
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
