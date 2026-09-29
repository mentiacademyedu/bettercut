//! The timeline canvas (§53).
//!
//! > **The timeline is a custom-drawn canvas, not a widget tree.**
//!
//! Everything below — ruler, lanes, clips, playhead, selection, drag previews —
//! is drawn with one `Painter` inside one allocated rect. A project with 10,000
//! clips creates zero extra widgets; it draws only the ~30 that intersect the
//! viewport, found with `Track::clips_in_range` (§54's range query).
//!
//! Rules from §53 that are easy to break later:
//!
//! * **A timeline repaint must never trigger a decode.** Waveforms are drawn
//!   from cached peaks and nothing else; the peaks are loaded once, before the
//!   draw pass, and a clip whose analysis has not finished simply draws as a
//!   plain block. Nothing here ever opens a media file.
//! * Pixel↔tick conversion is integer arithmetic (§74). `ticks_per_pixel` comes
//!   from a fixed ladder, never from a float ratio.
//!
//! ## Drags commit once
//!
//! A drag updates a *preview* every frame and dispatches exactly one command on
//! release. Dispatching per frame would put sixty entries on the undo stack for
//! one gesture; §11's history is meant to hold user intentions, not mouse
//! samples.

use bettercut_editor_core::foundation::Rational;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::project_format::Project;
use bettercut_editor_core::timeline::{
    Interpolation, Sequence, TimelineRange, TrackKind, Transition, TransitionKind, snap,
};
use bettercut_editor_core::{Editor, TrimEdge};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};

use crate::state::{ContextTarget, DragMode, DragState, UiState};
use crate::theme;

/// How close to a clip edge counts as grabbing the trim handle.
const TRIM_HANDLE_PIXELS: f32 = 7.0;

/// Maps timeline ticks to screen x, and back.
#[derive(Clone, Copy)]
struct Viewport {
    /// Screen x of tick `scroll_ticks` — the left edge of the lane area.
    origin_x: f32,
    scroll_ticks: i64,
    ticks_per_pixel: i64,
}

impl Viewport {
    fn x_of(&self, t: TimelineTime) -> f32 {
        self.origin_x + ((t.ticks() - self.scroll_ticks) / self.ticks_per_pixel) as f32
    }

    fn tick_of(&self, x: f32) -> TimelineTime {
        let pixels = (x - self.origin_x) as i64;
        TimelineTime::from_ticks(
            self.scroll_ticks
                .saturating_add(pixels.saturating_mul(self.ticks_per_pixel))
                .max(0),
        )
    }

    /// Like `tick_of`, but allows negative results so a drag can be clamped
    /// deliberately rather than silently sticking at zero.
    fn raw_tick_of(&self, x: f32) -> i64 {
        let pixels = (x - self.origin_x) as i64;
        self.scroll_ticks
            .saturating_add(pixels.saturating_mul(self.ticks_per_pixel))
    }

    /// The time span currently on screen — the §53 viewport query bound.
    fn visible_range(&self, width: f32) -> TimelineRange {
        let start = TimelineTime::from_ticks(self.scroll_ticks);
        let span = (width.max(1.0) as i64).saturating_mul(self.ticks_per_pixel);
        let end =
            TimelineTime::from_ticks(self.scroll_ticks.saturating_add(span).saturating_add(1));
        TimelineRange { start, end }
    }
}

/// Where a track was drawn this frame. Needed after the draw pass to resolve
/// which track a pointer is over and where to paint a drag preview.
#[derive(Clone, Copy)]
struct LaneLayout {
    track: TrackId,
    kind: TrackKind,
    rect: Rect,
}

/// A clip the pointer is currently over.
#[derive(Clone, Copy)]
struct ClipHit {
    clip: ClipId,
    track: TrackId,
    rect: Rect,
    range: TimelineRange,
}

/// A volume point the pointer is over, found by the draw pass.
#[derive(Clone, Copy)]
struct EnvelopeHit {
    clip: ClipId,
    index: usize,
    rect: Rect,
}

/// A point of a *track's* volume line the pointer is over.
#[derive(Clone, Copy)]
struct TrackEnvelopeHit {
    track: TrackId,
    index: usize,
    rect: Rect,
}

/// A clip's colour tag as the colour its strip is drawn in.
fn label_color(label: bettercut_editor_core::timeline::ColorLabel) -> Option<Color32> {
    label.rgb().map(|[r, g, b]| Color32::from_rgb(r, g, b))
}

/// A fade handle the pointer is over, found by the draw pass.
#[derive(Clone, Copy)]
struct FadeHit {
    clip: ClipId,
    edge: crate::state::FadeEdge,
    range: TimelineRange,
}

/// Where a sound clip's two fade handles sit: on its top edge, at the end of
/// each fade — or just inside the corner when there is none, so there is always
/// something to grab.
///
/// Public for the tests, which must press exactly where the timeline drew them.
pub fn fade_handle_positions(
    clip_rect: Rect,
    range: TimelineRange,
    (rise, fall): (i64, i64),
    origin_x: f32,
    scroll_ticks: i64,
    ticks_per_pixel: i64,
) -> [Pos2; 2] {
    let x_of = |t: i64| origin_x + ((t - scroll_ticks) / ticks_per_pixel.max(1)) as f32;
    let y = clip_rect.top() + FADE_HANDLE_SIZE;
    let inset = FADE_HANDLE_SIZE;
    let left = clip_rect.left() + inset;
    let right = (clip_rect.right() - inset).max(left);
    [
        Pos2::new(x_of(range.start.ticks() + rise).clamp(left, right), y),
        Pos2::new(x_of(range.end.ticks() - fall).clamp(left, right), y),
    ]
}

/// Side of a fade handle's square, and how close the pointer must be to grab
/// it — a little more than the square, because a 7 px target is hard to hit.
const FADE_HANDLE_SIZE: f32 = 7.0;
const FADE_HANDLE_REACH: f32 = 8.0;

/// Pointer state for this frame, plus what the draw pass hit-tested.
struct Interaction {
    pointer: Option<Pos2>,
    clicked: bool,
    additive: bool,
    /// Ctrl+Alt held: a drag on a clip slips it rather than moving it.
    slip: bool,
    /// Ctrl+Shift held: a drag on a clip slides it between its neighbours.
    slide: bool,
    hit: Option<ClipHit>,
    /// Takes precedence over `hit`: a press on a volume point moves the point,
    /// not the clip under it.
    envelope: Option<EnvelopeHit>,
    /// The pointer is on a clip's volume *line* but not on one of its points,
    /// which is where a new point can be put.
    envelope_line: Option<(ClipId, Rect)>,
    /// The same two, for the line a whole lane rides on. Checked after the
    /// clip's: where both are under the pointer, the one drawn on the clip is
    /// the one being aimed at, because it is the one inside a smaller shape.
    track_envelope: Option<TrackEnvelopeHit>,
    track_envelope_line: Option<(TrackId, Rect)>,
    /// A fade handle under the pointer. Takes precedence over everything else
    /// on the clip: it is the smallest target, and the one aimed at.
    fade: Option<FadeHit>,
}

pub fn draw(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    let full = ui.available_rect_before_wrap();
    let (whole, response) = ui.allocate_exact_size(full.size(), Sense::click_and_drag());

    let painter = ui.painter_at(whole);
    painter.rect_filled(whole, 0, theme::timeline_background());

    // The overview strip takes a band off the bottom; everything below works
    // in what is left, so the ruler, the lanes and every hit test are unaware
    // of it.
    let overview = Rect::from_min_max(
        Pos2::new(
            whole.left(),
            (whole.bottom() - theme::OVERVIEW_HEIGHT).max(whole.top()),
        ),
        whole.max,
    );
    let rect = Rect::from_min_max(whole.min, Pos2::new(whole.right(), overview.top()));

    if editor.project().active().is_none() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No sequence",
            FontId::proportional(14.0),
            theme::disabled(),
        );
        return;
    }

    // An empty edit: the timeline is the biggest thing on screen and has
    // nothing to draw, so it says what would fill it.
    if editor
        .active_sequence()
        .is_some_and(|s| s.clip_spans().next().is_none())
        && state.drag.is_none()
    {
        painter.text(
            Pos2::new(
                rect.center().x,
                rect.top() + theme::RULER_HEIGHT + rect.height() / 3.0,
            ),
            Align2::CENTER_CENTER,
            "Drop a file here, or import one and press + beside it",
            FontId::proportional(14.0),
            theme::disabled(),
        );
    }

    select_whole_groups(editor, state);

    let lane_width = (rect.width() - theme::TRACK_HEADER_WIDTH).max(1.0);

    // While playing, the view keeps up with the playhead (§53). Not while the
    // user is dragging a clip or a band: the ground moving under a drag is the
    // one thing worse than losing sight of the playhead.
    if state.playback.is_some_and(|p| p.playing) && state.drag.is_none() && state.marquee.is_none()
    {
        state.follow_playhead(editor.playhead(), lane_width);
    }

    let viewport = Viewport {
        origin_x: rect.left() + theme::TRACK_HEADER_WIDTH,
        scroll_ticks: state.scroll_ticks,
        ticks_per_pixel: state.ticks_per_pixel(),
    };

    handle_scroll_and_zoom(ui, &response, state);

    // Load the peaks the visible audio clips need *before* the draw pass, which
    // borrows the project immutably and cannot also borrow `state` mutably.
    // Collecting the ids first ends that borrow.
    let audio_media: Vec<bettercut_editor_core::foundation::MediaId> = editor
        .project()
        .active()
        .map(|sequence| {
            let mut ids: Vec<_> = sequence
                .audio_tracks
                .iter()
                .flat_map(|t| t.clips().iter().map(|c| c.media_id))
                .collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        })
        .unwrap_or_default();

    let waveforms: std::collections::HashMap<_, _> = audio_media
        .into_iter()
        .filter_map(|id| state.waveforms.get(id).map(|w| (id, w)))
        .collect();

    // Same reason as the waveforms: loading a texture needs `&mut state`, and
    // the draw pass holds the project immutably.
    let video_media: Vec<bettercut_editor_core::foundation::MediaId> = editor
        .project()
        .active()
        .map(|sequence| {
            let mut ids: Vec<_> = sequence
                .video_tracks
                .iter()
                .flat_map(|t| t.clips().iter().map(|c| c.media_id))
                .collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        })
        .unwrap_or_default();

    let strips: std::collections::HashMap<_, _> = video_media
        .into_iter()
        .filter_map(|id| {
            state
                .thumbnails
                .filmstrip(ui.ctx(), id)
                .map(|strip| (id, strip))
        })
        .collect();

    let mut interaction = Interaction {
        pointer: response
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_latest_pos()),
        clicked: response.clicked(),
        additive: ui.input(|i| i.modifiers.command),
        slip: ui.input(|i| i.modifiers.command && i.modifiers.alt),
        slide: ui.input(|i| i.modifiers.command && i.modifiers.shift),
        hit: None,
        envelope: None,
        envelope_line: None,
        track_envelope: None,
        track_envelope_line: None,
        fade: None,
    };

    // The draw pass borrows the project immutably.
    let pointer = response.hover_pos();
    let lanes = {
        let project = editor.project();
        let Some(sequence) = project.active() else {
            return;
        };
        let playhead = editor.playhead();

        draw_ruler(
            &painter,
            rect,
            viewport,
            state,
            lane_width,
            editor.start_timecode(),
            pointer,
        );
        let lanes = draw_lanes(
            &painter,
            rect,
            viewport,
            state,
            project,
            sequence,
            lane_width,
            &mut interaction,
            &waveforms,
            &strips,
        );
        draw_in_out(
            &painter,
            rect,
            viewport,
            sequence.mark_in,
            sequence.mark_out,
        );
        draw_render_bar(&painter, rect, viewport, project, sequence);
        draw_markers(&painter, rect, viewport, &sequence.markers);
        draw_clip_marks(&painter, viewport, sequence, &lanes);
        if let Some(dialog) = &state.silence {
            draw_suggested_cuts(&painter, rect, viewport, dialog.ranges());
        }
        if let Some(dialog) = &state.highlights {
            draw_suggested_keeps(&painter, rect, viewport, dialog.ranges());
        }
        if let Some(dialog) = &state.scenes {
            draw_suggested_splits(&painter, rect, viewport, dialog.cuts());
        }
        draw_drag_preview(&painter, viewport, state, &lanes);
        draw_marquee(&painter, state);
        draw_playhead(&painter, rect, viewport, playhead);

        if sequence.clip_count() == 0 {
            painter.text(
                Pos2::new(
                    viewport.origin_x + lane_width / 2.0,
                    rect.top() + theme::RULER_HEIGHT + 36.0,
                ),
                Align2::CENTER_CENTER,
                "Drag files here, press “Add to timeline” in Media, or start from Templates",
                FontId::proportional(13.0),
                theme::disabled(),
            );
            // Where everything else is, for someone who has not found it yet.
            painter.text(
                Pos2::new(
                    viewport.origin_x + lane_width / 2.0,
                    rect.top() + theme::RULER_HEIGHT + 56.0,
                ),
                Align2::CENTER_CENTER,
                crate::keys::keys("Ctrl+K finds any action by name"),
                FontId::proportional(12.0),
                theme::disabled(),
            );
        }

        lanes
    };

    // A clip's note, where the pointer is resting on it — not while dragging,
    // where a tooltip would sit over the thing being placed.
    if state.drag.is_none()
        && state.context.is_none()
        && let Some(hit) = interaction.hit
        && let Some(note) = editor.clip_note(hit.clip)
    {
        response.clone().on_hover_text_at_pointer(note.to_owned());
    }

    draw_overview(&painter, overview, editor, state, lane_width);
    if overview_interaction(ui, &response, overview, editor, state, lane_width) {
        crate::context_menu::show(&response, editor, state);
        return;
    }

    // Now the project borrow is released and the editor can be mutated.
    apply_interaction(
        ui,
        &response,
        rect,
        viewport,
        &interaction,
        &lanes,
        editor,
        state,
    );

    crate::context_menu::show(&response, editor, state);
}

/// The whole edit in one strip, with the visible part marked on it (§58).
///
/// A long edit at a working zoom shows a few seconds at a time, and scrolling
/// then means guessing how far along the timeline is. The strip says where the
/// view is, where the clips are, and clicking it goes there.
fn draw_overview(
    painter: &egui::Painter,
    rect: Rect,
    editor: &Editor,
    state: &UiState,
    lane_width: f32,
) {
    painter.rect_filled(rect, 0, theme::track_header());
    painter.line_segment(
        [
            Pos2::new(rect.left(), rect.top()),
            Pos2::new(rect.right(), rect.top()),
        ],
        Stroke::new(1.0, theme::grid_line()),
    );

    let Some(sequence) = editor.project().active() else {
        return;
    };
    let visible = (lane_width as i64)
        .saturating_mul(state.ticks_per_pixel())
        .max(1);
    // Long enough to hold the edit and the view, so the marker never runs off
    // the end when the view is scrolled past the last clip.
    let total = sequence
        .duration()
        .ticks()
        .max(state.scroll_ticks.saturating_add(visible))
        .max(1);
    let left = rect.left() + 4.0;
    let width = (rect.width() - 8.0).max(1.0);
    let x_of = |ticks: i64| left + (ticks.clamp(0, total) as f64 / total as f64) as f32 * width;

    // Two rows: picture above, sound below, so the shape of the edit reads the
    // same way it does on the lanes.
    let top = rect.top() + 6.0;
    let row = ((rect.height() - 12.0) / 2.0).max(2.0);
    for span in sequence.clip_spans() {
        use bettercut_editor_core::timeline::TrackKind;
        let (y, colour) = match span.kind {
            TrackKind::Video => (top, theme::video_clip()),
            TrackKind::Audio => (top + row, theme::audio_clip()),
            TrackKind::Text => (top, theme::text_clip()),
            TrackKind::Adjustment => (top, theme::adjustment_clip()),
        };
        let x0 = x_of(span.timeline.start.ticks());
        let x1 = x_of(span.timeline.end.ticks()).max(x0 + 1.0);
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, y + row - 1.0)),
            0,
            colour,
        );
    }

    // Where the playhead is, and which part of all that is on screen.
    let playhead = x_of(editor.playhead().ticks());
    painter.line_segment(
        [
            Pos2::new(playhead, rect.top() + 2.0),
            Pos2::new(playhead, rect.bottom() - 2.0),
        ],
        Stroke::new(1.0, theme::playhead()),
    );
    let view = Rect::from_min_max(
        Pos2::new(x_of(state.scroll_ticks), rect.top() + 2.0),
        Pos2::new(
            x_of(state.scroll_ticks.saturating_add(visible)).max(x_of(state.scroll_ticks) + 2.0),
            rect.bottom() - 2.0,
        ),
    );
    painter.rect_filled(view, 2, theme::selection().gamma_multiply(0.15));
    painter.rect_stroke(
        view,
        2,
        Stroke::new(1.0, theme::selection()),
        StrokeKind::Inside,
    );
}

/// A press or drag on the overview strip moves the view there.
///
/// Returns true while the strip owns the gesture, so the timeline underneath
/// leaves it alone — otherwise the same drag would also scrub the playhead.
fn overview_interaction(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    editor: &Editor,
    state: &mut UiState,
    lane_width: f32,
) -> bool {
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| ui.ctx().pointer_latest_pos());
    let Some(pos) = pointer else {
        state.overview_drag = false;
        return false;
    };
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    if pressed && rect.contains(pos) {
        state.overview_drag = true;
    }
    if !state.overview_drag {
        return false;
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        state.overview_drag = false;
        return true;
    }

    let visible = (lane_width as i64)
        .saturating_mul(state.ticks_per_pixel())
        .max(1);
    let total = editor
        .project()
        .active()
        .map_or(1, |sequence| sequence.duration().ticks())
        .max(state.scroll_ticks.saturating_add(visible))
        .max(1);
    let left = rect.left() + 4.0;
    let width = (rect.width() - 8.0).max(1.0);
    let through = f64::from(((pos.x - left) / width).clamp(0.0, 1.0));
    state.center_view_on(
        TimelineTime::from_ticks((through * total as f64) as i64),
        lane_width,
    );
    true
}

fn handle_scroll_and_zoom(ui: &egui::Ui, response: &egui::Response, state: &mut UiState) {
    if !response.hovered() {
        return;
    }
    let (scroll, modifiers) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers));

    // Ctrl/Cmd + wheel zooms; plain wheel pans. Both are what a user expects,
    // and neither requires hunting for a modifier.
    if modifiers.command && scroll.y != 0.0 {
        if scroll.y > 0.0 {
            state.zoom_in();
        } else {
            state.zoom_out();
        }
    } else if scroll.x != 0.0 || scroll.y != 0.0 {
        let delta = if scroll.x != 0.0 { scroll.x } else { scroll.y };
        state.scroll_by_pixels(-delta);
    }
}

// ---- interaction ---------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn apply_interaction(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    viewport: Viewport,
    interaction: &Interaction,
    lanes: &[LaneLayout],
    editor: &mut Editor,
    state: &mut UiState,
) {
    // Releasing ends any drag. A press that never moved is a click, so it
    // falls through to the selection logic below rather than being swallowed.
    if (response.drag_stopped() || ui.input(|i| i.pointer.primary_released()))
        && let Some(drag) = state.drag.take()
    {
        state.needs_repaint = true;
        if drag.moved {
            commit_drag(drag, editor, state);
            return;
        }
    }

    let Some(pos) = interaction.pointer else {
        return;
    };

    // Right-click: remember what is under the pointer before the menu opens.
    if response.secondary_clicked() && rect.contains(pos) {
        capture_context(pos, rect, viewport, interaction, lanes, state);
        return;
    }

    // Capture the grab on *press*, while the pointer is still over the clip.
    // Waiting for egui's `drag_started` loses the clip whenever the pointer
    // travels far enough in one frame to leave it.
    // A double-click on a point takes it out; on the line between points, puts
    // one in. Checked before the drag, because the press that begins the second
    // click of a double-click would otherwise start one.
    if ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    }) {
        if let Some(hit) = interaction.envelope {
            state.envelope_drag = None;
            remove_envelope_point(hit, editor, state);
            return;
        }
        if let Some((clip, _)) = interaction.envelope_line {
            state.envelope_drag = None;
            add_envelope_point(clip, pos, viewport, editor, state);
            return;
        }
        if let Some(hit) = interaction.track_envelope {
            state.track_envelope_drag = None;
            remove_track_volume_point(hit, editor, state);
            return;
        }
        if let Some((track, _)) = interaction.track_envelope_line {
            state.track_envelope_drag = None;
            add_track_volume_point(track, pos, viewport, editor, state);
            return;
        }
        // On the clip itself: select it and open the inspector where its
        // settings are — the grade for a picture, the sound for a sound.
        if let Some(hit) = interaction.hit {
            state.clear_selection();
            state.selected_clips.insert(hit.clip);
            if editor.audio_clip(hit.clip).is_some() {
                state.inspector_tab = crate::panels::InspectorTab::Audio;
            } else if editor.video_clip(hit.clip).is_some() {
                state.inspector_tab = crate::panels::InspectorTab::Colours;
            }
            state.drag = None;
            state.needs_repaint = true;
            return;
        }
    }

    // A fade handle, before anything else under the pointer: grabbing a
    // corner of a clip is aiming at the corner, not the clip.
    if interaction.fade.is_some() || state.fade_drag.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if state.drag.is_none()
        && state.envelope_drag.is_none()
        && state.fade_drag.is_none()
        && ui.input(|i| i.pointer.primary_pressed())
        && let Some(hit) = interaction.fade
        && let Some((rise, fall)) = editor
            .audio_clip(hit.clip)
            .map(|clip| (clip.fade_in.ticks(), clip.fade_out.ticks()))
            .or_else(|| editor.video_clip(hit.clip).map(|clip| clip.motion.ramps()))
    {
        let fade_ends = match hit.edge {
            crate::state::FadeEdge::In => hit.range.start.ticks() + rise,
            crate::state::FadeEdge::Out => hit.range.end.ticks() - fall,
        };
        state.fade_drag = Some(crate::state::FadeDrag {
            clip: hit.clip,
            edge: hit.edge,
            range: hit.range,
            moved: false,
            grab_offset: viewport.raw_tick_of(pos.x) - fade_ends,
        });
        return;
    }
    if state.fade_drag.is_some() {
        if response.dragged() {
            drag_fade(pos, viewport, editor, state);
        }
        if ui.input(|i| i.pointer.primary_released()) {
            state.fade_drag = None;
        }
        return;
    }

    // A press on a volume point moves the point, not the clip under it (§24).
    if state.drag.is_none()
        && state.envelope_drag.is_none()
        && ui.input(|i| i.pointer.primary_pressed())
        && let Some(hit) = interaction.envelope
        && let Some(clip) = editor.audio_clip(hit.clip)
    {
        state.envelope_drag = Some(crate::state::EnvelopeDrag {
            clip: hit.clip,
            index: hit.index,
            points: envelope_points_of(clip),
            rect: hit.rect,
            moved: false,
        });
        return;
    }

    if state.envelope_drag.is_some() {
        if response.dragged() {
            drag_envelope_point(pos, viewport, editor, state);
        }
        if ui.input(|i| i.pointer.primary_released()) {
            state.envelope_drag = None;
        }
        return;
    }

    // And the same for a point of a whole lane's line.
    if state.drag.is_none()
        && state.envelope_drag.is_none()
        && state.track_envelope_drag.is_none()
        && ui.input(|i| i.pointer.primary_pressed())
        && let Some(hit) = interaction.track_envelope
    {
        state.track_envelope_drag = Some(crate::state::TrackEnvelopeDrag {
            track: hit.track,
            index: hit.index,
            points: editor.track_volume(hit.track).to_vec(),
            rect: hit.rect,
            moved: false,
        });
        return;
    }

    if state.track_envelope_drag.is_some() {
        if response.dragged() {
            drag_track_volume_point(pos, viewport, editor, state);
        }
        if ui.input(|i| i.pointer.primary_released()) {
            state.track_envelope_drag = None;
        }
        return;
    }

    if state.drag.is_none()
        && ui.input(|i| i.pointer.primary_pressed())
        && pos.x >= viewport.origin_x
        && pos.y >= rect.top() + theme::RULER_HEIGHT
        && let Some(hit) = interaction.hit
    {
        begin_drag(
            hit,
            pos,
            Held {
                additive: interaction.additive,
                slip: interaction.slip,
                slide: interaction.slide,
                alone: ui.input(|i| i.modifiers.shift),
            },
            viewport,
            state,
        );
        if let Some(drag) = state.drag.as_mut() {
            // A split edit moves this edge and nothing else, so the partner
            // is not drawn as coming with it.
            drag.partners = if drag.alone {
                Vec::new()
            } else {
                linked_partners(editor, hit.clip)
            };
            // Ctrl+Alt on an edge that meets another clip rolls that cut.
            if drag.mode == DragMode::Slip {
                let near_start = pos.x - hit.rect.left() <= TRIM_HANDLE_PIXELS;
                let near_end = hit.rect.right() - pos.x <= TRIM_HANDLE_PIXELS;
                let left = if near_end {
                    editor
                        .roll_partner(hit.clip, TrimEdge::End)
                        .map(|_| hit.clip)
                } else if near_start {
                    editor.roll_partner(hit.clip, TrimEdge::Start)
                } else {
                    None
                };
                if let Some(left) = left {
                    let cut = if near_end {
                        hit.range.end
                    } else {
                        hit.range.start
                    };
                    drag.mode = DragMode::Roll;
                    drag.roll = Some(crate::state::RollDrag { left, cut, to: cut });
                }
            }
        }
        return;
    }

    if state.drag.is_some() && response.dragged() {
        update_drag(ui, pos, viewport, lanes, editor, state);
        return;
    }

    // A press on empty lane space starts a rubber band (§10 "Multi-select").
    // Captured on press like a clip drag, and only *becomes* a marquee once the
    // pointer moves — a press that never moves is a click and must still put
    // the playhead where the user clicked.
    if state.marquee.is_none()
        && state.drag.is_none()
        && ui.input(|i| i.pointer.primary_pressed())
        && rect.contains(pos)
        && pos.x >= viewport.origin_x
        && pos.y >= rect.top() + theme::RULER_HEIGHT
        && interaction.hit.is_none()
    {
        state.marquee = Some(crate::state::Marquee {
            origin: pos,
            current: pos,
            additive: interaction.additive,
            moved: false,
        });
        return;
    }

    if state.marquee.is_some() {
        if response.dragged() {
            if let Some(marquee) = state.marquee.as_mut() {
                // A few pixels of travel is a shaky click, not a selection.
                if (pos - marquee.origin).length() > 3.0 {
                    marquee.moved = true;
                }
                marquee.current = pos;
            }
            state.needs_repaint = true;
            return;
        }

        // Released. `drag_stopped` alone is unreliable when the pointer leaves
        // the widget mid-drag, so the button state is checked directly.
        if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            state.needs_repaint = true;
            if let Some(marquee) = state.marquee.take()
                && marquee.moved
            {
                select_within(marquee, viewport, lanes, editor, state);
                return;
            }
            // Fall through: a press that never moved is an ordinary click.
        }
    }

    // The track-header column is not part of the timeline, so a *click* there
    // must not move the playhead. A drag already under way is different: the
    // pointer crossing back over the headers is how a user drags the playhead
    // to the start, and bailing out here left tick 0 sitting on a single
    // boundary pixel — reachable only by landing on it exactly, and not
    // reachable at all once the view had scrolled. That is the whole reason
    // the start of a sequence could only be got at with the -10s button.
    let dragging = response.dragged();
    if pos.x < viewport.origin_x && !dragging {
        if response.clicked() {
            press_header_button(pos, rect, lanes, editor, state);
        }
        return; // track-header column
    }

    // `pos` may be `pointer_latest_pos`, which is the pointer anywhere in the
    // window — including the preview and the toolbar, which sit *above* this
    // widget. Testing only `pos.y < top + RULER_HEIGHT` was therefore true for
    // the entire upper half of the app, and simply moving the mouse there
    // scrubbed the playhead. Anything positional must first confirm the pointer
    // is actually inside the timeline.
    let in_ruler = rect.contains(pos) && pos.y < rect.top() + theme::RULER_HEIGHT;

    // Dragging outside a clip scrubs; so does the ruler, but only while the
    // button is held. Hovering the ruler must not move the playhead.
    //
    // **Down *on this widget*, not down anywhere.** `pos` falls back to the
    // pointer's position anywhere in the window, so a global button check meant
    // that dragging the picture in the preview scrubbed the timeline the moment
    // the cursor crossed into the ruler — the playhead jumping to wherever the
    // mouse happened to be, mid-gesture, for a drag that had nothing to do with
    // the timeline.
    let holding = response.is_pointer_button_down_on();
    if dragging || (in_ruler && holding) {
        // Dragging past an edge scrolls the view, so a scrub can reach
        // material that is currently off screen without letting go. Without
        // this the reachable range is whatever happens to be visible, which
        // is most obvious at the start: scrolled forward, there is no way
        // back to zero.
        //
        // The viewport was resolved at the top of this frame, so the scroll
        // lands on the next one. That is what makes holding at the edge scroll
        // continuously rather than jumping once.
        if pos.x < viewport.origin_x {
            state.scroll_by_pixels(pos.x - viewport.origin_x);
        } else if pos.x > rect.right() {
            state.scroll_by_pixels(pos.x - rect.right());
        }

        // `tick_of` clamps at zero, so dragging off the left edge settles on
        // the start of the sequence rather than running negative.
        editor.set_playhead(viewport.tick_of(pos.x));
        state.needs_repaint = true;
        return;
    }

    if !interaction.clicked {
        return;
    }

    match interaction.hit {
        Some(hit) => {
            if interaction.additive {
                state.toggle_selection(hit.clip);
            } else {
                state.select_only(hit.clip);
            }
        }
        None => {
            // Clicking empty canvas deselects and moves the playhead there.
            state.clear_selection();
            editor.set_playhead(viewport.tick_of(pos.x));
            state.needs_repaint = true;
        }
    }
}

/// Work out what a right-click landed on, and prepare the selection for it.
///
/// Right-clicking a clip that is not selected selects it, which is what every
/// other editor does — otherwise "Delete" in the menu would silently act on
/// some other clip the user had selected earlier. A clip already in a
/// multi-selection leaves that selection alone, so "Delete" still means all of
/// them.
fn capture_context(
    pos: Pos2,
    rect: Rect,
    viewport: Viewport,
    interaction: &Interaction,
    lanes: &[LaneLayout],
    state: &mut UiState,
) {
    state.needs_repaint = true;

    if let Some(hit) = interaction.hit {
        if !state.selected_clips.contains(&hit.clip) {
            state.select_only(hit.clip);
        }
        state.context = Some(ContextTarget::Clip {
            clip: hit.clip,
            track: hit.track,
        });
        return;
    }

    // The header column, if the click was left of the lanes and below the ruler.
    if pos.x < viewport.origin_x
        && pos.y >= rect.top() + theme::RULER_HEIGHT
        && let Some(lane) = lanes
            .iter()
            .find(|l| pos.y >= l.rect.top() && pos.y <= l.rect.bottom())
    {
        state.selected_track = Some(lane.track);
        state.context = Some(ContextTarget::TrackHeader { track: lane.track });
        return;
    }

    state.context = Some(ContextTarget::Empty {
        at: viewport.tick_of(pos.x.max(viewport.origin_x)),
        track: lanes
            .iter()
            .find(|l| pos.y >= l.rect.top() && pos.y <= l.rect.bottom())
            .map(|l| l.track),
    });
}

/// A click in the header column, which may be on a mute or solo button.
///
/// Uses the same [`header_buttons`] the drawing does, so the targets are where
/// they look.
fn press_header_button(
    pos: Pos2,
    rect: Rect,
    lanes: &[LaneLayout],
    editor: &mut Editor,
    state: &mut UiState,
) {
    use bettercut_editor_core::TrackFlag;

    if pos.y < rect.top() + theme::RULER_HEIGHT {
        return;
    }
    let Some(lane) = lanes
        .iter()
        .find(|l| pos.y >= l.rect.top() && pos.y <= l.rect.bottom())
    else {
        return;
    };

    let header = Rect::from_min_size(
        lane.rect.min,
        vec2(theme::TRACK_HEADER_WIDTH, lane.rect.height()),
    );
    let [target, mute, solo] = header_buttons(header);
    // Targeting is not a flag the user sets twice over: setting it takes it
    // off the other lanes of the kind, which is its own call (§10).
    if target.contains(pos) {
        let now = editor.track_flag(lane.track, TrackFlag::Targeted);
        if let Err(err) = editor.set_target_track(lane.track, !now) {
            state.error(err.to_string());
        }
        state.needs_repaint = true;
        return;
    }
    let (flag, now) = if mute.contains(pos) {
        (
            TrackFlag::Enabled,
            editor.track_flag(lane.track, TrackFlag::Enabled),
        )
    } else if solo.contains(pos) {
        (
            TrackFlag::Solo,
            editor.track_flag(lane.track, TrackFlag::Solo),
        )
    } else {
        return;
    };

    if let Err(err) = editor.set_track_flag(lane.track, flag, !now) {
        state.error(err.to_string());
    }
    state.needs_repaint = true;
}

/// Select every clip the rubber band touches (§10).
///
/// Works in time and track space rather than against the pixel rects the draw
/// pass produced: a clip scrolled off the left edge is still *inside* the band
/// if the band covers its span, and testing screen rectangles would miss it.
fn select_within(
    marquee: crate::state::Marquee,
    viewport: Viewport,
    lanes: &[LaneLayout],
    editor: &Editor,
    state: &mut UiState,
) {
    let rect = marquee.rect();
    let start = viewport.tick_of(rect.left());
    // At least one tick wide, or a vertical band would select nothing.
    let end = TimelineTime::from_ticks(
        viewport
            .tick_of(rect.right())
            .ticks()
            .max(start.ticks() + 1),
    );
    let band = TimelineRange { start, end };

    if !marquee.additive {
        state.clear_selection();
    }

    let Some(sequence) = editor.active_sequence() else {
        return;
    };

    let mut selected = 0;
    for lane in lanes
        .iter()
        .filter(|l| l.rect.top() < rect.bottom() && l.rect.bottom() > rect.top())
    {
        match lane.kind {
            TrackKind::Video => {
                if let Some(track) = sequence.video_tracks.iter().find(|t| t.id == lane.track) {
                    for clip in track.clips_in_range(band) {
                        state.selected_clips.insert(clip.id);
                        selected += 1;
                    }
                }
            }
            TrackKind::Audio => {
                if let Some(track) = sequence.audio_tracks.iter().find(|t| t.id == lane.track) {
                    for clip in track.clips_in_range(band) {
                        state.selected_clips.insert(clip.id);
                        selected += 1;
                    }
                }
            }
            TrackKind::Text => {
                if let Some(track) = sequence.text_tracks.iter().find(|t| t.id == lane.track) {
                    for clip in track.clips_in_range(band) {
                        state.selected_clips.insert(clip.id);
                        selected += 1;
                    }
                }
            }
            TrackKind::Adjustment => {
                if let Some(track) = sequence.adjustment_track(lane.track) {
                    for clip in track.clips_in_range(band) {
                        state.selected_clips.insert(clip.id);
                        selected += 1;
                    }
                }
            }
        }
    }

    state.needs_repaint = true;
    if selected > 0 {
        state.info(format!("Selected {selected} clip(s)"));
    }
}

/// `clip`'s linked partners and where each of them is now (§12).
fn linked_partners(editor: &Editor, clip: ClipId) -> Vec<(TrackId, TimelineRange)> {
    let Some(sequence) = editor.active_sequence() else {
        return Vec::new();
    };
    // Its sound, and the rest of its group with theirs: whatever the editor
    // will move with it on release.
    editor
        .moves_with(clip)
        .into_iter()
        .filter(|partner| *partner != clip)
        .filter_map(|partner| {
            let span = sequence.clip_span(partner)?;
            Some((span.track, span.timeline))
        })
        .collect()
}

/// A group is one thing to select: any member selected brings the rest.
fn select_whole_groups(editor: &Editor, state: &mut UiState) {
    let mut added = Vec::new();
    for clip in &state.selected_clips {
        if let Some(group) = editor.group_of(*clip) {
            added.extend(
                group
                    .into_iter()
                    .filter(|c| !state.selected_clips.contains(c)),
            );
        }
    }
    if !added.is_empty() {
        state.selected_clips.extend(added);
        state.needs_repaint = true;
    }
}

/// The band a lane's volume line is drawn in: along the top of the lane, clear
/// of the clips' own envelopes.
///
/// The two are different lines about different things — this one is the lane's
/// level, that one is the clip's — and drawn over each other they would read
/// as one shape that jumps at every clip edge.
fn track_volume_band(lane: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(lane.left(), lane.top() + 2.0),
        Pos2::new(lane.right(), lane.top() + (lane.height() * 0.45).max(14.0)),
    )
}

fn track_volume_y(band: Rect, gain: f32) -> f32 {
    band.bottom() - gain.clamp(0.0, 1.0) * band.height()
}

/// The level a height means, the inverse of [`track_volume_y`].
fn track_volume_gain(band: Rect, y: f32) -> f32 {
    ((band.bottom() - y) / band.height().max(1.0)).clamp(0.0, 1.0)
}

/// Where each point of a lane's line sits on screen, shared by the painter and
/// the hit test so a dot is grabbable exactly where it is drawn.
fn track_volume_dots(
    volume: &bettercut_editor_core::timeline::TrackVolume,
    band: Rect,
    viewport: Viewport,
) -> Vec<(usize, Pos2)> {
    volume
        .points()
        .iter()
        .enumerate()
        .map(|(index, point)| {
            (
                index,
                Pos2::new(viewport.x_of(point.at), track_volume_y(band, point.gain)),
            )
        })
        .collect()
}

/// A lane's volume line across its whole lane, and what the pointer is over.
///
/// Sampled from the same `gain_at` the mixer asks, so the line drawn is the
/// level heard rather than a drawing of what the points mean (§46).
fn draw_track_volume(
    painter: &egui::Painter,
    lane: Rect,
    viewport: Viewport,
    track: &bettercut_editor_core::timeline::AudioTrack,
    interaction: &mut Interaction,
) {
    if track.volume.is_empty() {
        return;
    }
    let band = track_volume_band(lane);
    let visible = Rect::from_min_max(Pos2::new(viewport.origin_x, band.top()), band.max);
    let clipped = painter.with_clip_rect(visible);

    // One point every three pixels: finer than the eye resolves at timeline
    // scale, and a fraction of the shapes a point per pixel would emit.
    let mut points = Vec::new();
    let mut x = visible.left();
    while x <= visible.right() {
        let at = TimelineTime::from_ticks(viewport.raw_tick_of(x).max(0));
        let gain = track.volume.gain_at(at).unwrap_or(track.gain);
        points.push(Pos2::new(x, track_volume_y(band, gain)));
        x += 3.0;
    }
    if points.len() >= 2 {
        clipped.add(egui::Shape::line(
            points,
            Stroke::new(1.5, theme::track_automation()),
        ));
    }
    for (_, at) in track_volume_dots(&track.volume, band, viewport) {
        clipped.circle_filled(at, 3.5, theme::track_automation());
    }

    // What is under the pointer, for the press handler.
    let Some(pos) = interaction.pointer.filter(|pos| visible.contains(*pos)) else {
        return;
    };
    for (index, dot) in track_volume_dots(&track.volume, band, viewport) {
        if dot.distance(pos) <= 7.0 {
            interaction.track_envelope = Some(TrackEnvelopeHit {
                track: track.id,
                index,
                rect: band,
            });
            return;
        }
    }
    let at = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x).max(0));
    let gain = track.volume.gain_at(at).unwrap_or(track.gain);
    if (track_volume_y(band, gain) - pos.y).abs() <= 7.0 {
        interaction.track_envelope_line = Some((track.id, band));
    }
}

/// Take one point out of a lane's line.
///
/// Any of them, unlike a clip envelope: a line of one point is a level the
/// lane rides at, which is a useful thing to keep, and an empty line is the
/// lane back at its static level, which the menu also offers.
fn remove_track_volume_point(
    hit: TrackEnvelopeHit,
    editor: &mut Editor,
    state: &mut crate::state::UiState,
) {
    let mut points = editor.track_volume(hit.track).to_vec();
    if hit.index >= points.len() {
        return;
    }
    points.remove(hit.index);
    if let Err(err) = editor.set_track_volume(hit.track, points, false) {
        state.error(err.to_string());
    }
    state.needs_repaint = true;
}

/// Put a new point on a lane's line where the pointer is, at the level the
/// line already has there — somewhere to take hold of, without the shape
/// jumping.
fn add_track_volume_point(
    track: TrackId,
    pos: Pos2,
    viewport: Viewport,
    editor: &mut Editor,
    state: &mut crate::state::UiState,
) {
    let at = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x).max(0));
    // Right on top of one that is already there is not a new point.
    if editor
        .track_volume(track)
        .iter()
        .any(|point| (point.at - at).ticks().abs() < TimelineTime::from_millis(20).ticks())
    {
        return;
    }
    if let Err(err) = editor.add_track_volume_point(track, at) {
        state.error(err.to_string());
    }
    state.needs_repaint = true;
}

/// Move the grabbed point of a lane's line to the pointer and write the line.
///
/// Held between its neighbours, as a clip envelope point is: a line whose
/// points crossed would be reordered underneath the drag, and the point would
/// appear to jump to somewhere the pointer is not.
fn drag_track_volume_point(
    pos: Pos2,
    viewport: Viewport,
    editor: &mut Editor,
    state: &mut crate::state::UiState,
) {
    let Some(drag) = state.track_envelope_drag.as_mut() else {
        return;
    };
    if drag.points.get(drag.index).is_none() {
        state.track_envelope_drag = None;
        return;
    }
    let band = drag.rect;
    let wanted = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x).max(0));
    let low = drag
        .index
        .checked_sub(1)
        .and_then(|before| drag.points.get(before))
        .map_or(TimelineTime::ZERO, |point| {
            point.at + TimelineTime::from_millis(1)
        });
    let high = drag
        .points
        .get(drag.index + 1)
        .map(|point| point.at - TimelineTime::from_millis(1));
    let at = match high {
        Some(high) => wanted.clamp(low, high.max(low)),
        None => wanted.max(low),
    };
    drag.points[drag.index] =
        bettercut_editor_core::timeline::VolumePoint::new(at, track_volume_gain(band, pos.y));

    let points = drag.points.clone();
    let continuing = drag.moved;
    drag.moved = true;
    let track = drag.track;
    if let Err(err) = editor.set_track_volume(track, points, continuing) {
        state.error(err.to_string());
        state.track_envelope_drag = None;
    }
    state.needs_repaint = true;
}

/// A clip's envelope as timeline instants and levels, which is the form the
/// editor takes and the form a drag edits.
fn envelope_points_of(
    clip: &bettercut_editor_core::timeline::AudioClip,
) -> Vec<(TimelineTime, f32)> {
    clip.keyframes
        .track(bettercut_editor_core::timeline::AnimatedParameter::Gain)
        .map(|track| {
            track
                .keys()
                .iter()
                .map(|key| {
                    let into_source = (key.time.ticks() - clip.source.start.ticks()).max(0);
                    let at = clip.timeline.start
                        + TimelineTime::from_ticks(
                            bettercut_editor_core::timeline::timeline_ticks_for(
                                bettercut_editor_core::foundation::MediaTime::from_ticks(
                                    into_source,
                                ),
                                clip.speed,
                            ),
                        );
                    (at, key.value)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Take one point out of an envelope (§24).
///
/// The last two are kept: an envelope of one point is a level, not a shape, and
/// clearing the whole thing is what the clip menu's Clear Ducking is for —
/// having it happen as the side effect of a double-click would be a surprise.
fn remove_envelope_point(hit: EnvelopeHit, editor: &mut Editor, state: &mut UiState) {
    let Some(clip) = editor.audio_clip(hit.clip) else {
        return;
    };
    let mut points = envelope_points_of(clip);
    if points.len() <= 2 || hit.index >= points.len() {
        state.info("An envelope needs at least two points — use Clear Ducking to remove it");
        state.needs_repaint = true;
        return;
    }
    points.remove(hit.index);
    if let Err(err) = editor.set_gain_envelope(hit.clip, &points, false) {
        state.error(err.to_string());
    }
    state.needs_repaint = true;
}

/// Put a new point on the line where the pointer is (§24).
///
/// At the level the line already has there, so the shape does not jump: a new
/// point is somewhere to *take hold of*, and moving it is a separate decision.
fn add_envelope_point(
    clip: ClipId,
    pos: Pos2,
    viewport: Viewport,
    editor: &mut Editor,
    state: &mut UiState,
) {
    let Some(audio) = editor.audio_clip(clip) else {
        return;
    };
    let at = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x))
        .clamp(audio.timeline.start, audio.timeline.end);
    let level = audio.gain_at(at);

    let mut points = envelope_points_of(audio);
    // Right on top of an existing point is not a new point.
    if points
        .iter()
        .any(|(existing, _)| (*existing - at).ticks().abs() < TimelineTime::from_millis(20).ticks())
    {
        return;
    }
    // Appended rather than inserted in order: the envelope is sorted where it
    // is written (`Keyframes::replace`), and a second place that sorts is a
    // second place that can disagree about how.
    points.push((at, level));

    if let Err(err) = editor.set_gain_envelope(clip, &points, false) {
        state.error(err.to_string());
    }
    state.needs_repaint = true;
}

/// Set the dragged fade to reach the pointer.
///
/// Measured from the clip's own edge and put on the frame grid (§76), held
/// between nothing and the whole clip less the other fade — two fades that
/// overlap are not two fades.
fn drag_fade(pos: Pos2, viewport: Viewport, editor: &mut Editor, state: &mut UiState) {
    use crate::state::FadeEdge;

    let Some(drag) = state.fade_drag.as_mut() else {
        return;
    };
    let picture = editor.video_clip(drag.clip).is_some();
    let Some((fade_in, fade_out)) = editor
        .audio_clip(drag.clip)
        .map(|clip| (clip.fade_in, clip.fade_out))
        .or_else(|| {
            editor.video_clip(drag.clip).map(|clip| {
                let (rise, fall) = clip.motion.ramps();
                (
                    TimelineTime::from_ticks(rise),
                    TimelineTime::from_ticks(fall),
                )
            })
        })
    else {
        state.fade_drag = None;
        return;
    };
    let length = drag.range.end.ticks() - drag.range.start.ticks();
    let at = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x) - drag.grab_offset);
    let at = editor
        .active_sequence()
        .map_or(at, |sequence| sequence.snap_to_frame(at));

    let (wanted_in, wanted_out) = match drag.edge {
        FadeEdge::In => {
            let most = (length - fade_out.ticks()).max(0);
            let reach = (at.ticks() - drag.range.start.ticks()).clamp(0, most);
            (TimelineTime::from_ticks(reach), fade_out)
        }
        FadeEdge::Out => {
            let most = (length - fade_in.ticks()).max(0);
            let reach = (drag.range.end.ticks() - at.ticks()).clamp(0, most);
            (fade_in, TimelineTime::from_ticks(reach))
        }
    };
    if (wanted_in, wanted_out) == (fade_in, fade_out) {
        return;
    }
    let continuing = drag.moved;
    drag.moved = true;
    let result = if picture {
        editor.set_clip_ramps(drag.clip, wanted_in, wanted_out, continuing)
    } else {
        editor.set_clip_fades(drag.clip, wanted_in, wanted_out, continuing)
    };
    if let Err(err) = result {
        state.error(err.to_string());
        state.fade_drag = None;
    }
}

/// Move the grabbed point to the pointer and write the whole envelope.
///
/// Both axes: a duck is adjusted as much by moving *when* it happens as by how
/// deep it is. The point is held between its neighbours, because an envelope
/// whose keys crossed would be reordered underneath the drag and the point
/// would appear to jump to somewhere the pointer is not.
fn drag_envelope_point(pos: Pos2, viewport: Viewport, editor: &mut Editor, state: &mut UiState) {
    let Some(drag) = state.envelope_drag.as_mut() else {
        return;
    };
    let Some(clip) = editor.audio_clip(drag.clip) else {
        state.envelope_drag = None;
        return;
    };
    let span = clip.timeline;
    let rect = drag.rect;

    if drag.points.get(drag.index).is_none() {
        state.envelope_drag = None;
        return;
    }

    let wanted = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x)).clamp(span.start, span.end);
    let low = drag
        .index
        .checked_sub(1)
        .and_then(|before| drag.points.get(before))
        .map_or(span.start, |(at, _)| *at + TimelineTime::from_millis(1));
    let high = drag
        .points
        .get(drag.index + 1)
        .map_or(span.end, |(at, _)| *at - TimelineTime::from_millis(1));

    drag.points[drag.index] = (wanted.clamp(low, high.max(low)), envelope_gain(rect, pos.y));

    let points = drag.points.clone();
    let continuing = drag.moved;
    drag.moved = true;
    let clip = drag.clip;
    if let Err(err) = editor.set_gain_envelope(clip, &points, continuing) {
        state.error(err.to_string());
        state.envelope_drag = None;
    }
    state.needs_repaint = true;
}

/// What was held down when a clip was grabbed: the modifiers that decide what
/// kind of drag this is.
#[derive(Debug, Clone, Copy, Default)]
struct Held {
    additive: bool,
    slip: bool,
    slide: bool,
    /// Shift on an edge: the J and L cuts.
    alone: bool,
}

fn begin_drag(hit: ClipHit, pos: Pos2, held: Held, viewport: Viewport, state: &mut UiState) {
    let Held {
        additive,
        slip,
        slide,
        alone,
    } = held;
    // Ctrl+Alt slips, wherever the clip was grabbed. Otherwise near an edge
    // means trim and anywhere else means move. The handle is a fixed pixel
    // width so it stays grabbable at every zoom level.
    let mode = if slide {
        DragMode::Slide
    } else if slip {
        DragMode::Slip
    } else if pos.x - hit.rect.left() <= TRIM_HANDLE_PIXELS {
        DragMode::TrimStart
    } else if hit.rect.right() - pos.x <= TRIM_HANDLE_PIXELS {
        DragMode::TrimEnd
    } else {
        DragMode::Move
    };

    // Shift on an edge trims this clip alone: a split edit, and the one
    // deliberate exception to §12's linked trimming.
    let alone = alone && matches!(mode, DragMode::TrimStart | DragMode::TrimEnd);

    state.drag = Some(DragState {
        clip: hit.clip,
        source_track: hit.track,
        mode,
        grab_offset: viewport.raw_tick_of(pos.x) - hit.range.start.ticks(),
        original: hit.range,
        preview: hit.range,
        moved: false,
        target_track: hit.track,
        target_invalid: false,
        snapped_to: None,
        partners: Vec::new(),
        slip: 0,
        roll: None,
        insert: false,
        alone,
    });

    // Dragging an unselected clip selects it, so the inspector follows.
    //
    // Except when Ctrl is held: that press is the start of a Ctrl+click, and
    // the release below toggles the clip. Selecting it here meant the toggle
    // immediately removed it again — Ctrl+click could never add a second clip
    // to the selection, it only ever cleared it.
    if (!additive || slip || slide) && !state.selected_clips.contains(&hit.clip) {
        state.select_only(hit.clip);
    }
    state.needs_repaint = true;
}

fn update_drag(
    ui: &egui::Ui,
    pos: Pos2,
    viewport: Viewport,
    lanes: &[LaneLayout],
    editor: &Editor,
    state: &mut UiState,
) {
    // Alt bypasses snapping for this drag — the standard way to place
    // something deliberately between grid points.
    let bypass_snap = ui.input(|i| i.modifiers.alt);
    let tolerance = if state.snapping && !bypass_snap {
        state.snap_tolerance()
    } else {
        TimelineTime::ZERO
    };

    let Some(sequence) = editor.project().active() else {
        return;
    };
    let Some(drag) = state.drag.as_mut() else {
        return;
    };
    drag.moved = true;

    let targets = snap::collect_targets(sequence, editor.playhead(), &[drag.clip]);

    match drag.mode {
        DragMode::Move => {
            let wanted =
                TimelineTime::from_ticks((viewport.raw_tick_of(pos.x) - drag.grab_offset).max(0));
            let duration = drag.original.duration();
            let (start, hit) = snap::snap_move(wanted, duration, &targets, tolerance);

            drag.preview = TimelineRange {
                start,
                end: start + duration,
            };
            drag.snapped_to = hit;
            drag.insert = ui.input(|i| i.modifiers.shift);

            // Which lane is the pointer over?
            if let Some(lane) = lanes
                .iter()
                .find(|l| pos.y >= l.rect.top() && pos.y <= l.rect.bottom())
            {
                drag.target_track = lane.track;
                let source_kind = sequence.track_kind(drag.source_track);
                drag.target_invalid = source_kind != Some(lane.kind);
            }
        }
        DragMode::TrimStart => {
            let wanted = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x).max(0));
            let (start, hit) = snap::snap(wanted, &targets, tolerance);
            // Never let a trim invert the clip; leave at least one pixel.
            let limit = drag.original.end - TimelineTime::from_ticks(viewport.ticks_per_pixel);
            drag.preview = TimelineRange {
                start: start.min(limit),
                end: drag.original.end,
            };
            drag.snapped_to = hit;
        }
        DragMode::TrimEnd => {
            let wanted = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x).max(0));
            let (end, hit) = snap::snap(wanted, &targets, tolerance);
            let limit = drag.original.start + TimelineTime::from_ticks(viewport.ticks_per_pixel);
            drag.preview = TimelineRange {
                start: drag.original.start,
                end: end.max(limit),
            };
            drag.snapped_to = hit;
        }
        DragMode::Slip => {
            // The footage is pulled along under a clip that stays put: drag
            // right and the clip shows what came earlier in the file, as if
            // sliding a strip of film through a fixed window.
            let travelled =
                viewport.raw_tick_of(pos.x) - drag.original.start.ticks() - drag.grab_offset;
            drag.slip = slip_offset(editor, drag.clip, travelled);
            drag.snapped_to = None;
        }
        DragMode::Slide => {
            // The clip keeps its length: only where it starts changes, and
            // only as far as its neighbours allow.
            let wanted = viewport.raw_tick_of(pos.x) - drag.grab_offset;
            let by = wanted - drag.original.start.ticks();
            let (earliest, latest) = editor.slide_room(drag.clip).unwrap_or((0, 0));
            let by = by.clamp(earliest, latest);
            let duration = drag.original.duration();
            let start = TimelineTime::from_ticks(drag.original.start.ticks() + by);
            drag.preview = TimelineRange {
                start,
                end: start + duration,
            };
            drag.snapped_to = None;
        }
        DragMode::Roll => {
            if let Some(roll) = drag.roll.as_mut() {
                let wanted = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x).max(0));
                let (to, hit) = snap::snap(wanted, &targets, tolerance);
                let (earliest, latest) =
                    editor.roll_room(roll.left).unwrap_or((roll.cut, roll.cut));
                roll.to = to.clamp(earliest, latest);
                drag.snapped_to = hit.filter(|_| roll.to == to);
            }
        }
    }

    state.needs_repaint = true;
}

/// How far a slip drag that has `travelled` timeline ticks moves `clip`'s
/// source, held inside its file: the opposite way to the pointer, scaled by the
/// clip's speed so the footage keeps pace with the mouse.
///
/// Zero for a clip with nothing to slip, so the drag shows nothing happening
/// and the release explains why.
fn slip_offset(editor: &Editor, clip: ClipId, travelled: i64) -> i64 {
    let Some((earlier, later)) = editor.slip_room(clip) else {
        return 0;
    };
    let ranges = editor
        .video_clip(clip)
        .map(|c| (c.timeline, c.source, c.reversed))
        .or_else(|| {
            editor
                .audio_clip(clip)
                .map(|c| (c.timeline, c.source, c.reversed))
        });
    let rate = ranges.map_or(1.0, |(timeline, source, reversed)| {
        let on_timeline = timeline.duration().ticks().max(1);
        let rate = source.duration().ticks() as f64 / on_timeline as f64;
        // Played backwards, what came earlier on screen is later in the file.
        if reversed { -rate } else { rate }
    });
    ((-travelled as f64 * rate).round() as i64).clamp(earlier, later)
}

/// Turn the finished drag into exactly one command (§11).
fn commit_drag(drag: DragState, editor: &mut Editor, state: &mut UiState) {
    // A drag that changed nothing must not create an undo entry.
    if drag.preview == drag.original
        && drag.target_track == drag.source_track
        && drag.slip == 0
        && drag.roll.is_none_or(|roll| roll.to == roll.cut)
    {
        return;
    }
    if drag.mode == DragMode::Slip {
        if let Err(err) = editor.slip_clip(drag.clip, drag.slip) {
            state.error(err.to_string());
        }
        return;
    }
    if drag.mode == DragMode::Slide {
        let by = drag.preview.start.ticks() - drag.original.start.ticks();
        if by != 0
            && let Err(err) = editor.slide_clip(drag.clip, by)
        {
            state.error(err.to_string());
        }
        return;
    }
    if drag.mode == DragMode::Roll {
        if let Some(roll) = drag.roll
            && let Err(err) = editor.roll_edit(roll.left, roll.to)
        {
            state.error(err.to_string());
        }
        return;
    }

    let result = match drag.mode {
        DragMode::Move if drag.insert => {
            editor.insert_clip_at(drag.clip, drag.target_track, drag.preview.start)
        }
        DragMode::Move => editor.move_clip(
            drag.source_track,
            drag.target_track,
            drag.clip,
            drag.preview.start,
        ),
        DragMode::TrimStart if drag.alone => {
            editor.split_edit(drag.clip, TrimEdge::Start, drag.preview.start)
        }
        DragMode::TrimEnd if drag.alone => {
            editor.split_edit(drag.clip, TrimEdge::End, drag.preview.end)
        }
        DragMode::TrimStart => editor.trim_clip(
            drag.source_track,
            drag.clip,
            TrimEdge::Start,
            drag.preview.start,
        ),
        DragMode::TrimEnd => editor.trim_clip(
            drag.source_track,
            drag.clip,
            TrimEdge::End,
            drag.preview.end,
        ),
        DragMode::Slip | DragMode::Roll | DragMode::Slide => return,
    };

    match result {
        // A magnetic main track closes up behind whatever moved or shrank.
        Ok(()) => crate::shortcuts::close_up_if_magnetic(editor, state),
        // The clip stays where it was; say why rather than silently snapping
        // back, which reads as the drag having been ignored.
        Err(err) => state.error(err.to_string()),
    }
}

// ---- drawing -------------------------------------------------------------

fn draw_ruler(
    painter: &egui::Painter,
    rect: Rect,
    viewport: Viewport,
    state: &UiState,
    lane_width: f32,
    start: TimelineTime,
    pointer: Option<Pos2>,
) {
    let ruler = Rect::from_min_size(rect.min, vec2(rect.width(), theme::RULER_HEIGHT));
    painter.rect_filled(ruler, 0, theme::track_header());

    // The time under the pointer, as the ruler's own readout: scrubbing is
    // one thing, knowing where a cut would land before clicking is another.
    if let Some(pointer) =
        pointer.filter(|p| p.x >= viewport.origin_x && p.x <= rect.right() && rect.contains(*p))
    {
        let at = viewport.tick_of(pointer.x).max(TimelineTime::ZERO);
        let text = (start + at).format_timecode();
        let anchor_at = Pos2::new(pointer.x.min(rect.right() - 4.0), ruler.bottom() - 3.0);
        let align = if pointer.x > rect.right() - 90.0 {
            Align2::RIGHT_BOTTOM
        } else {
            Align2::LEFT_BOTTOM
        };
        // Measured first, so the label sits on its own patch rather than on
        // top of the ticks it is meant to explain.
        let galley = painter.layout_no_wrap(text, FontId::monospace(11.0), theme::ruler_text());
        let nudge = vec2(
            if align == Align2::LEFT_BOTTOM {
                4.0
            } else {
                -4.0
            },
            0.0,
        );
        let at = anchor_at + nudge;
        let box_at = align.anchor_size(at, galley.size());
        painter.rect_filled(box_at.expand(2.0), 2, theme::track_header());
        painter.galley(box_at.min, galley, theme::ruler_text());
    }

    // Pick the finest label interval that still leaves ~70 px between labels.
    // Without this the ruler becomes an unreadable smear when zoomed out.
    const CANDIDATES: [i64; 12] = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600];
    let px_per_second = state.pixels_per_second();
    let step_seconds = CANDIDATES
        .iter()
        .copied()
        .find(|&s| s as f32 * px_per_second >= 70.0)
        .unwrap_or(3600);

    let step_ticks = TimelineTime::from_seconds(step_seconds).ticks();
    let span = (lane_width as i64).saturating_mul(viewport.ticks_per_pixel);
    let last = viewport.scroll_ticks.saturating_add(span);
    let mut ticks = (viewport.scroll_ticks / step_ticks) * step_ticks;

    while ticks <= last {
        let t = TimelineTime::from_ticks(ticks);
        let x = viewport.x_of(t);
        if x >= viewport.origin_x - 1.0 {
            painter.line_segment(
                [
                    Pos2::new(x, ruler.bottom() - 6.0),
                    Pos2::new(x, ruler.bottom()),
                ],
                Stroke::new(1.0, theme::ruler_text()),
            );
            painter.text(
                Pos2::new(x + 4.0, ruler.top() + 4.0),
                Align2::LEFT_TOP,
                (t + start).format_timecode(),
                FontId::monospace(11.0),
                theme::ruler_text(),
            );
            painter.line_segment(
                [Pos2::new(x, ruler.bottom()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, theme::grid_line()),
            );
        }
        match ticks.checked_add(step_ticks) {
            Some(next) => ticks = next,
            None => break,
        }
    }

    // Zoom readout, so the scale is never a mystery.
    painter.text(
        Pos2::new(rect.left() + 8.0, ruler.center().y),
        Align2::LEFT_CENTER,
        format!("{px_per_second:.0} px/s"),
        FontId::monospace(11.0),
        theme::disabled(),
    );
}

#[allow(clippy::too_many_arguments)] // a canvas draw genuinely needs all of it
fn draw_lanes(
    painter: &egui::Painter,
    rect: Rect,
    viewport: Viewport,
    state: &UiState,
    project: &Project,
    sequence: &Sequence,
    lane_width: f32,
    interaction: &mut Interaction,
    waveforms: &std::collections::HashMap<
        bettercut_editor_core::foundation::MediaId,
        std::sync::Arc<bettercut_cache::Waveform>,
    >,
    strips: &std::collections::HashMap<
        bettercut_editor_core::foundation::MediaId,
        (egui::TextureHandle, u32),
    >,
) -> Vec<LaneLayout> {
    let visible = viewport.visible_range(lane_width);
    let mut lanes = Vec::with_capacity(sequence.track_count());

    // §12: the links the selection carries, so each clip can say whether it
    // will move with it. Collected once rather than asked per clip, which
    // would make drawing the timeline quadratic in the clip count.
    let selected_links: Vec<bettercut_editor_core::foundation::LinkId> = sequence
        .video_tracks
        .iter()
        .flat_map(|t| t.clips())
        .filter(|c| state.selected_clips.contains(&c.id))
        .filter_map(|c| c.link)
        .chain(
            sequence
                .audio_tracks
                .iter()
                .flat_map(|t| t.clips())
                .filter(|c| state.selected_clips.contains(&c.id))
                .filter_map(|c| c.link),
        )
        .collect();
    // Grouped clips carry a badge, so a group reads as one before anything is
    // dragged.
    let grouped: std::collections::HashSet<ClipId> =
        sequence.groups.iter().flatten().copied().collect();
    let noted: std::collections::HashSet<ClipId> =
        sequence.notes.iter().map(|note| note.clip).collect();
    let badges_with_group = |mut badges: Vec<&'static str>, id: ClipId| {
        if grouped.contains(&id) {
            badges.push("group");
        }
        if sequence.clip_soloed(id) {
            badges.push("solo");
        }
        if noted.contains(&id) {
            badges.push("note");
        }
        badges
    };
    let partner = |link: Option<bettercut_editor_core::foundation::LinkId>, id: ClipId| {
        !state.selected_clips.contains(&id) && link.is_some_and(|l| selected_links.contains(&l))
    };
    // Solo, per clip: what another clip's solo silences draws dimmed, as a
    // clip on a hidden lane does, so the timeline says why it is not playing.
    let picture_soloed = sequence.picture_clip_soloed();
    let sound_soloed = sequence.sound_clip_soloed();
    let mut y = rect.top() + theme::RULER_HEIGHT + theme::TRACK_GAP;

    // §26's titles first, because they composite over every video track and the
    // lane order on screen is the compositing order upside down.
    for track in &sequence.text_tracks {
        let lane_height = state.lane_height_for(track.id);
        let lane = Rect::from_min_size(Pos2::new(rect.left(), y), vec2(rect.width(), lane_height));
        draw_lane_background(painter, lane, viewport, 0);
        draw_track_header(painter, lane, track, None, None);

        for clip in track.clips_in_range(visible) {
            draw_clip(
                painter,
                lane,
                viewport,
                ClipVisual {
                    range: clip.timeline,
                    // The words themselves, so a lane of titles can be read
                    // without clicking each one.
                    label: if clip.is_blank() {
                        "(empty)"
                    } else {
                        &clip.text
                    },
                    selected: state.selected_clips.contains(&clip.id),
                    track_enabled: track.enabled
                        && sequence.clip_plays(clip.id, true, picture_soloed),
                    dragging: state.drag.as_ref().is_some_and(|d| d.clip == clip.id),
                    body: theme::text_clip(),
                    top: theme::text_clip_top(),
                    waveform: None,
                    volume: None,
                    filmstrip: None,
                    colour: None,
                    duration_of_media: MediaTime::ZERO,
                    keyframes: None,
                    transition: None,
                    effects: badges_with_group(Vec::new(), clip.id),
                    frozen: false,
                    speed: Rational::ONE,
                    linked_to_selection: false,
                    ramps: (
                        clip.animation.intro.map_or(0, |m| m.duration.ticks()),
                        clip.animation.outro.map_or(0, |m| m.duration.ticks()),
                    ),
                    // A title's ramps are its entrance and exit, set in the
                    // Inspector with a preset; there is no fade to drag.
                    fade_handles: false,
                    label_color: label_color(clip.color_label)
                        .or_else(|| label_color(track.color_label)),
                },
                clip.id,
                track.id,
                interaction,
            );
        }

        lanes.push(LaneLayout {
            track: track.id,
            kind: TrackKind::Text,
            rect: lane,
        });
        y += lane_height + theme::TRACK_GAP;
    }

    // Adjustment lanes next: they composite over the video and under the
    // titles, and the lanes on screen are the compositing order upside down.
    // Topmost lane first, because a later lane grades what an earlier one left.
    for track in sequence.adjustment_tracks.iter().rev() {
        let lane_height = state.lane_height_for(track.id);
        let lane = Rect::from_min_size(Pos2::new(rect.left(), y), vec2(rect.width(), lane_height));
        draw_lane_background(painter, lane, viewport, 0);
        draw_track_header(painter, lane, track, None, None);

        for clip in track.clips_in_range(visible) {
            draw_clip(
                painter,
                lane,
                viewport,
                ClipVisual {
                    range: clip.timeline,
                    // What it does, rather than a name it does not have: an
                    // adjustment left at its defaults does nothing yet, and
                    // saying so is the hint to open the Inspector.
                    label: if clip.look.is_identity() {
                        "Adjustment (no change)"
                    } else {
                        "Adjustment"
                    },
                    selected: state.selected_clips.contains(&clip.id),
                    track_enabled: track.enabled
                        && sequence.clip_plays(clip.id, true, picture_soloed),
                    dragging: state.drag.as_ref().is_some_and(|d| d.clip == clip.id),
                    body: theme::adjustment_clip(),
                    top: theme::adjustment_clip_top(),
                    waveform: None,
                    volume: None,
                    filmstrip: None,
                    colour: None,
                    duration_of_media: MediaTime::ZERO,
                    keyframes: None,
                    transition: None,
                    effects: badges_with_group(Vec::new(), clip.id),
                    frozen: false,
                    speed: Rational::ONE,
                    linked_to_selection: false,
                    ramps: (0, 0),
                    fade_handles: false,
                    label_color: label_color(clip.color_label)
                        .or_else(|| label_color(track.color_label)),
                },
                clip.id,
                track.id,
                interaction,
            );
        }

        lanes.push(LaneLayout {
            track: track.id,
            kind: TrackKind::Adjustment,
            rect: lane,
        });
        y += lane_height + theme::TRACK_GAP;
    }

    // Video tracks top-down in reverse index order: index 0 is the bottom
    // compositing layer (§22), and editors conventionally show that layer
    // nearest the audio tracks.
    for (row, track) in sequence.video_tracks.iter().enumerate().rev() {
        let lane_height = state.lane_height_for(track.id);
        let lane = Rect::from_min_size(Pos2::new(rect.left(), y), vec2(rect.width(), lane_height));
        draw_lane_background(painter, lane, viewport, row);
        draw_track_header(painter, lane, track, None, None);

        for clip in track.clips_in_range(visible) {
            let label = clip.name.as_deref().unwrap_or_else(|| {
                project
                    .media_asset(clip.media_id)
                    .map_or("(missing media)", |m| m.display_name())
            });
            draw_clip(
                painter,
                lane,
                viewport,
                ClipVisual {
                    range: clip.timeline,
                    label,
                    selected: state.selected_clips.contains(&clip.id),
                    track_enabled: track.enabled
                        && sequence.clip_plays(clip.id, true, picture_soloed),
                    dragging: state.drag.as_ref().is_some_and(|d| d.clip == clip.id),
                    body: theme::video_clip(),
                    top: theme::video_clip_top(),
                    waveform: None,
                    volume: None,
                    effects: badges_with_group(crate::effects::badges(clip), clip.id),
                    frozen: clip.frozen,
                    // No filmstrip on a hold: the picture does not move, so
                    // tiles marching across it would say otherwise.
                    filmstrip: (!clip.frozen)
                        .then(|| strips.get(&clip.media_id))
                        .flatten()
                        .map(|(handle, tiles)| (handle, *tiles, clip.source.start)),
                    colour: project
                        .media_asset(clip.media_id)
                        .and_then(|m| match m.generated {
                            Some(bettercut_editor_core::media::Generated::Colour {
                                top,
                                bottom,
                            }) => Some((
                                Color32::from_rgb(top[0], top[1], top[2]),
                                Color32::from_rgb(bottom[0], bottom[1], bottom[2]),
                            )),
                            _ => None,
                        }),
                    duration_of_media: project
                        .media_asset(clip.media_id)
                        .map_or(MediaTime::ZERO, |m| m.duration),
                    keyframes: (!clip.keyframes.is_empty())
                        .then_some((&clip.keyframes, clip.source.start)),
                    transition: clip.transition_out,
                    speed: clip.speed,
                    linked_to_selection: partner(clip.link, clip.id),
                    // §45: the same ramps a sound's fades and a title's
                    // entrance get. A clip that is arriving should look like
                    // it, rather than like an ordinary clip with a badge on it.
                    ramps: clip.motion.ramps(),
                    // A picture fades by its entrance and exit: the same
                    // handles a sound's fades have set how long they take.
                    fade_handles: true,
                    label_color: label_color(clip.color_label)
                        .or_else(|| label_color(track.color_label)),
                },
                clip.id,
                track.id,
                interaction,
            );
        }

        lanes.push(LaneLayout {
            track: track.id,
            kind: TrackKind::Video,
            rect: lane,
        });
        y += lane_height + theme::TRACK_GAP;
    }

    for (row, track) in sequence.audio_tracks.iter().enumerate() {
        let lane_height = state.lane_height_for(track.id);
        let lane = Rect::from_min_size(Pos2::new(rect.left(), y), vec2(rect.width(), lane_height));
        draw_lane_background(painter, lane, viewport, row + 1);
        draw_track_header(
            painter,
            lane,
            track,
            Some((track.gain, track.pan)),
            Some(state.lane_level(row)),
        );

        for clip in track.clips_in_range(visible) {
            let label = clip.name.as_deref().unwrap_or_else(|| {
                project
                    .media_asset(clip.media_id)
                    .map_or("(missing media)", |m| m.display_name())
            });
            draw_clip(
                painter,
                lane,
                viewport,
                ClipVisual {
                    range: clip.timeline,
                    label,
                    selected: state.selected_clips.contains(&clip.id),
                    track_enabled: track.enabled
                        && sequence.clip_plays(clip.id, true, sound_soloed),
                    dragging: state.drag.as_ref().is_some_and(|d| d.clip == clip.id),
                    body: theme::audio_clip(),
                    top: theme::audio_clip_top(),
                    effects: {
                        let mut badges = badges_with_group(Vec::new(), clip.id);
                        if clip.crossfade_out > TimelineTime::ZERO {
                            badges.push("xfade");
                        }
                        if clip.muted {
                            badges.push("muted");
                        }
                        badges
                    },
                    frozen: false,
                    waveform: waveforms
                        .get(&clip.media_id)
                        .map(|w| (w.as_ref(), clip.source.start)),
                    filmstrip: None,
                    colour: None,
                    duration_of_media: MediaTime::ZERO,
                    keyframes: None,
                    volume: clip
                        .keyframes
                        .is_animated(bettercut_editor_core::timeline::AnimatedParameter::Gain)
                        .then_some(clip),
                    // §25 v1 is picture only: an audio crossfade mixes two
                    // sources rather than blending two images.
                    transition: None,
                    speed: clip.speed,
                    linked_to_selection: partner(clip.link, clip.id),
                    ramps: clip.fitted_fades(),
                    fade_handles: true,
                    label_color: label_color(clip.color_label)
                        .or_else(|| label_color(track.color_label)),
                },
                clip.id,
                track.id,
                interaction,
            );
        }

        draw_track_volume(painter, lane, viewport, track, interaction);

        lanes.push(LaneLayout {
            track: track.id,
            kind: TrackKind::Audio,
            rect: lane,
        });
        y += lane_height + theme::TRACK_GAP;
    }

    lanes
}

/// Where a sound lane's meter sits in its header: the strip down the right
/// edge, outside the mute and solo buttons.
///
/// Geometry of its own, like [`header_buttons`], so the drawing and anything
/// that has to leave room for it agree.
pub fn header_meter(header: Rect) -> Rect {
    const WIDTH: f32 = 7.0;
    Rect::from_min_max(
        Pos2::new(header.right() - WIDTH - 1.0, header.top() + 3.0),
        Pos2::new(header.right() - 1.0, header.bottom() - 3.0),
    )
}

/// A lane's live level, two bars tall, down the side of its header.
///
/// The master meter says whether the mix is about to clip. This says *which
/// lane* is making the sound — the question you ask when a voice is missing,
/// or when music is louder than the take under it, and the one no single
/// master meter can answer.
///
/// Drawn on the same decibel scale as the master meter
/// (`crate::panels::meter_fraction`), so a lane at half the bar and the master
/// at half the bar mean the same thing.
fn draw_header_meter(painter: &egui::Painter, header: Rect, (left, right): (f32, f32)) {
    let strip = header_meter(header);
    let bar_width = (strip.width() - 1.0) / 2.0;
    for (index, level) in [left, right].into_iter().enumerate() {
        let x = strip.left() + index as f32 * (bar_width + 1.0);
        let bar = Rect::from_min_size(Pos2::new(x, strip.top()), vec2(bar_width, strip.height()));
        painter.rect_filled(bar, 1.0, theme::timeline_background());

        let filled = strip.height() * crate::panels::meter_fraction(level);
        if filled <= 0.5 {
            continue;
        }
        // Up from the bottom, as a meter reads.
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(x, bar.bottom() - filled),
                Pos2::new(x + bar_width, bar.bottom()),
            ),
            1.0,
            if level >= 1.0 {
                theme::playhead()
            } else if level > 0.7 {
                theme::selection()
            } else {
                theme::audio_clip_top()
            },
        );
    }
}

/// Where the mute and solo buttons sit in a track header: `[mute, solo]`.
///
/// Right-aligned, so a long track name runs out of room before it reaches
/// them — the name is the thing that varies, and a control that moved with it
/// would be a control the user has to look for every time.
///
/// Pure geometry, because both the drawing and the hit test need exactly the
/// same answer: a button drawn in one place and clickable in another is the
/// kind of bug nobody reports, they just decide the app is broken.
pub fn header_buttons(header: Rect) -> [Rect; 3] {
    const SIZE: f32 = 16.0;
    const GAP: f32 = 3.0;
    let y = header.center().y - SIZE / 2.0;
    let solo_x = header.right() - SIZE - 8.0;
    let mute_x = solo_x - SIZE - GAP;
    let target_x = mute_x - SIZE - GAP;
    [
        Rect::from_min_size(Pos2::new(target_x, y), vec2(SIZE, SIZE)),
        Rect::from_min_size(Pos2::new(mute_x, y), vec2(SIZE, SIZE)),
        Rect::from_min_size(Pos2::new(solo_x, y), vec2(SIZE, SIZE)),
    ]
}

fn draw_lane_background(painter: &egui::Painter, lane: Rect, viewport: Viewport, row: usize) {
    let lanes = Rect::from_min_max(
        Pos2::new(viewport.origin_x, lane.top()),
        Pos2::new(lane.right(), lane.bottom()),
    );
    painter.rect_filled(
        lanes,
        0,
        if row.is_multiple_of(2) {
            theme::track_lane()
        } else {
            theme::track_lane_alt()
        },
    );
}

fn draw_track_header<C>(
    painter: &egui::Painter,
    lane: Rect,
    track: &bettercut_editor_core::timeline::Track<C>,
    mix: Option<(f32, f32)>,
    level: Option<(f32, f32)>,
) {
    let name = track.name.as_str();
    let enabled = track.enabled;
    let locked = track.locked;
    let solo = track.solo;
    let header = Rect::from_min_size(lane.min, vec2(theme::TRACK_HEADER_WIDTH, lane.height()));
    painter.rect_filled(header, 0, theme::track_header());
    // The lane's colour, as a strip down the head's left edge and a wash over
    // it: enough to find the lane at a glance, not enough to fight the name.
    if let Some(colour) = label_color(track.color_label) {
        painter.rect_filled(header, 0, colour.gamma_multiply(0.12));
        painter.rect_filled(
            Rect::from_min_size(header.min, vec2(4.0, header.height())),
            0,
            colour,
        );
    }
    painter.line_segment(
        [header.right_top(), header.right_bottom()],
        Stroke::new(1.0, theme::grid_line()),
    );

    painter.text(
        Pos2::new(header.left() + 10.0, header.center().y - 7.0),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.0),
        if enabled {
            theme::clip_text()
        } else {
            theme::disabled()
        },
    );

    // The two the mixer needs most, on the header rather than behind a
    // right-click: muting a lane to hear past it is something a user does
    // dozens of times in a cut, and a menu each time is a menu too many.
    // Before the buttons, so nothing is drawn over them.
    if let Some(level) = level {
        draw_header_meter(painter, header, level);
    }

    let [target_rect, mute_rect, solo_rect] = header_buttons(header);
    for (rect, glyph, on) in [
        (target_rect, "T", track.targeted),
        (mute_rect, "M", !enabled),
        (solo_rect, "S", solo),
    ] {
        painter.rect_filled(
            rect,
            3,
            if on {
                theme::selection()
            } else {
                theme::timeline_background()
            },
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(10.0),
            if on {
                theme::background()
            } else {
                theme::disabled()
            },
        );
    }

    let mut badges: Vec<String> = Vec::new();
    if !enabled {
        // An audio track is muted, not hidden: the same flag, the word the
        // user would look for.
        badges.push(if mix.is_some() { "muted" } else { "hidden" }.to_owned());
    }
    if locked {
        badges.push("locked".to_owned());
    }
    // Sync lock has no button of its own, and a lane that rides along with
    // edits made somewhere else should say so before it moves.
    if track.sync_lock {
        badges.push("sync".to_owned());
    }
    // §20a.4: loudest of the lot, because a solo left on is the reason every
    // other lane has gone quiet — and the only clue, if the header does not
    // say so, is a menu the user has no reason to open.
    if solo {
        badges.push("SOLO".to_owned());
    }
    // A lane at anything but unity and centre says so, or a quiet track is a
    // mystery until someone opens its menu.
    if let Some((gain, pan)) = mix {
        if (gain - 1.0).abs() > 0.005 {
            badges.push(format!("{:.0}%", gain * 100.0));
        }
        if pan.abs() > 0.005 {
            badges.push(crate::context_menu::pan_label(pan));
        }
    }
    if !badges.is_empty() {
        painter.text(
            Pos2::new(header.left() + 10.0, header.center().y + 10.0),
            Align2::LEFT_CENTER,
            badges.join(" · "),
            FontId::proportional(10.0),
            theme::disabled(),
        );
    }
}

/// A transition straddling the cut at `cut` (§25).
///
/// Drawn against the lane rather than the clip, because half of the window sits
/// over the *next* clip — which is exactly what it means: for that stretch both
/// clips are on screen.
fn draw_transition(
    painter: &egui::Painter,
    lane: Rect,
    viewport: Viewport,
    transition: Transition,
    cut: TimelineTime,
) {
    let window = transition.window(cut);
    let x0 = viewport.x_of(window.start).max(viewport.origin_x);
    let x1 = viewport.x_of(window.end).min(lane.right());
    if x1 - x0 < 2.0 {
        return; // zoomed out past legibility; the clips still read correctly
    }

    let rect = Rect::from_min_max(
        Pos2::new(x0, lane.top() + 3.0),
        Pos2::new(x1, lane.bottom() - 3.0),
    );
    painter.rect_filled(
        rect,
        theme::CLIP_CORNER_RADIUS,
        theme::transition().gamma_multiply(0.30),
    );

    // Too narrow for the symbol to be anything but a smudge; the panel alone
    // still says a transition is there.
    if rect.width() < 10.0 {
        return;
    }

    let stroke = Stroke::new(1.5, theme::transition());
    let inner = rect.shrink(3.0);
    match transition.kind {
        // Two crossing diagonals: the shape every editor uses for a dissolve,
        // and it reads as two things overlapping.
        TransitionKind::Crossfade => {
            painter.line_segment([inner.left_top(), inner.right_bottom()], stroke);
            painter.line_segment([inner.left_bottom(), inner.right_top()], stroke);
        }
        // A V down to the middle: down to black, back up again.
        TransitionKind::FadeThroughBlack => {
            let bottom = Pos2::new(inner.center().x, inner.bottom());
            painter.line_segment([inner.left_top(), bottom], stroke);
            painter.line_segment([bottom, inner.right_top()], stroke);
        }
        // Three lines fading apart towards the middle: sharp at the edges,
        // soft where they meet.
        TransitionKind::Blur => {
            for offset in [-4.0_f32, 0.0, 4.0] {
                let y = inner.center().y + offset;
                painter.line_segment(
                    [
                        Pos2::new(inner.left(), y),
                        Pos2::new(inner.center().x - 2.0, y),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        Pos2::new(inner.center().x + 2.0, y),
                        Pos2::new(inner.right(), y),
                    ],
                    stroke,
                );
            }
        }
        // A spiral's worth of arc: most of a circle, turning.
        TransitionKind::Spin => {
            let radius = inner.width().min(inner.height()) * 0.45;
            let points: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = i as f32 / 16.0 * std::f32::consts::TAU * 0.8;
                    let r = radius * (0.4 + 0.6 * i as f32 / 16.0);
                    inner.center() + egui::vec2(a.cos() * r, a.sin() * r)
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
        }
        // A jagged zigzag: a signal breaking up.
        TransitionKind::Glitch => {
            let steps = [0.5_f32, 0.1, 0.8, 0.3, 0.9, 0.5];
            let points: Vec<Pos2> = steps
                .iter()
                .enumerate()
                .map(|(i, y)| {
                    Pos2::new(
                        inner.left() + inner.width() * i as f32 / (steps.len() - 1) as f32,
                        inner.top() + inner.height() * y,
                    )
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
        }
        // The same V the other way up: up to white, back down again — the two
        // are complements, and drawing them as mirror images says so.
        TransitionKind::Flash => {
            let top = Pos2::new(inner.center().x, inner.top());
            painter.line_segment([inner.left_bottom(), top], stroke);
            painter.line_segment([top, inner.right_bottom()], stroke);
        }
        // An arrow to the right: something arriving from that side.
        TransitionKind::Slide => {
            let tip = Pos2::new(inner.right(), inner.center().y);
            painter.line_segment([Pos2::new(inner.left(), inner.center().y), tip], stroke);
            painter.line_segment([Pos2::new(inner.right() - 4.0, inner.top()), tip], stroke);
            painter.line_segment(
                [Pos2::new(inner.right() - 4.0, inner.bottom()), tip],
                stroke,
            );
        }
        // Two arrows, one behind the other: both shots on the move.
        TransitionKind::Push => {
            for offset in [-3.0_f32, 3.0] {
                let y = inner.center().y + offset;
                let tip = Pos2::new(inner.right(), y);
                painter.line_segment([Pos2::new(inner.left(), y), tip], stroke);
                painter.line_segment([Pos2::new(inner.right() - 3.0, y - 3.0), tip], stroke);
            }
        }
        // An edge part way across, the revealed side shaded.
        TransitionKind::Wipe => {
            let x = inner.left() + inner.width() * 0.45;
            painter.rect_filled(
                egui::Rect::from_min_max(inner.left_top(), Pos2::new(x, inner.bottom())),
                0.0,
                theme::transition().gamma_multiply(0.4),
            );
            painter.line_segment(
                [Pos2::new(x, inner.top()), Pos2::new(x, inner.bottom())],
                stroke,
            );
            painter.rect_stroke(inner, 0.0, stroke, StrokeKind::Inside);
        }
        // A circle opening in a frame.
        TransitionKind::Iris => {
            painter.rect_stroke(inner, 0.0, stroke, StrokeKind::Inside);
            painter.circle_stroke(
                inner.center(),
                inner.width().min(inner.height()) * 0.35,
                stroke,
            );
        }
        // A box growing out of a box.
        TransitionKind::Zoom => {
            painter.rect_stroke(
                inner.shrink(inner.width().min(inner.height()) * 0.3),
                0.0,
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_stroke(inner, 0.0, stroke, StrokeKind::Inside);
        }
    }
}

struct ClipVisual<'a> {
    range: TimelineRange,
    label: &'a str,
    selected: bool,
    track_enabled: bool,
    dragging: bool,
    body: Color32,
    top: Color32,
    /// Peaks plus where in the media this clip starts, so a trimmed clip shows
    /// the part of the waveform it actually plays.
    waveform: Option<(&'a bettercut_cache::Waveform, MediaTime)>,
    /// Filmstrip sheet, its tile count, and where in the media this clip
    /// starts — the same trimming question as the waveform.
    filmstrip: Option<(&'a egui::TextureHandle, u32, MediaTime)>,
    /// A colour clip's own colours, top and bottom: what it shows in place
    /// of a filmstrip, so a background is found by its colour.
    colour: Option<(Color32, Color32)>,
    /// Length of the whole source file, which is what the tiles span.
    duration_of_media: MediaTime,
    /// The clip's animation and where in the media it starts, drawn as marks
    /// along the bottom edge (§24). `None` for audio, which Milestone 8 does
    /// not animate.
    keyframes: Option<(&'a bettercut_editor_core::timeline::Keyframes, MediaTime)>,
    /// §25's transition at this clip's end, drawn straddling the cut. It is not
    /// clipped to the clip's own rect, because half of it belongs to the next
    /// clip's side of the boundary.
    transition: Option<Transition>,
    /// §12: this clip moves with the selection because it is linked to it.
    ///
    /// Shown, because otherwise the first a user learns of the link is the
    /// sound jumping when they release a drag of the picture.
    linked_to_selection: bool,
    /// Ticks of fade in and fade out on sound, or of entrance and exit on a
    /// title, drawn as ramps at the clip's ends. `(0, 0)` for none.
    ramps: (i64, i64),
    /// Whether this clip has fade handles at its top corners — sound only.
    fade_handles: bool,
    /// The clip's colour tag, drawn as its top strip in place of the lane's
    /// colour, so a tagged clip is found at a glance.
    label_color: Option<Color32>,
    /// A sound clip's volume envelope (§24), drawn as a line over its
    /// waveform. `None` for picture, and for sound whose volume does not move.
    ///
    /// The clip itself rather than its keys, because the line is
    /// `gain_at` sampled across the width — the same function the mixer asks,
    /// so what is drawn is what is heard (§46).
    volume: Option<&'a bettercut_editor_core::timeline::AudioClip>,

    /// Short names for the effects on this clip — mask, key, a blend other
    /// than normal — shown as badges. Empty for an ordinary clip.
    effects: Vec<&'static str>,

    /// A held frame (§10's freeze), shown as a badge and without a filmstrip.
    ///
    /// A hold looks exactly like an ordinary clip otherwise, and its filmstrip
    /// would be the clearest possible lie: tiles marching across a clip whose
    /// picture never moves.
    frozen: bool,
    /// the clip's playback rate, shown as a badge when it is not normal.
    ///
    /// A re-timed clip looks exactly like any other one, and its length is not
    /// a clue — a two-second clip is two seconds whatever the reason. Without a
    /// mark on it, "why is this playing fast" has no answer on screen.
    speed: Rational,
}

/// The rubber band itself: a translucent fill with a crisp edge.
///
/// Drawn after the clips so it reads as being over them, and only once it has
/// actually become a drag — a one-pixel box flashing under every click would be
/// visual noise.
fn draw_marquee(painter: &egui::Painter, state: &UiState) {
    let Some(marquee) = state.marquee else {
        return;
    };
    if !marquee.moved {
        return;
    }

    let rect = marquee.rect();
    painter.rect_filled(rect, 2, theme::selection().gamma_multiply(0.18));
    painter.rect_stroke(
        rect,
        2,
        Stroke::new(1.0, theme::selection()),
        StrokeKind::Inside,
    );
}

/// Paint the filmstrip tiles a clip covers (§53).
///
/// Tiles span the whole source file, so a trimmed clip shows only the slice it
/// plays. Each tile is drawn at the screen position of the source time it was
/// sampled from, which keeps the strip aligned with the footage as the user
/// trims and zooms rather than stretching to fit the clip.
#[allow(clippy::too_many_arguments)]
fn draw_filmstrip(
    painter: &egui::Painter,
    clip_rect: Rect,
    viewport: Viewport,
    sheet: &egui::TextureHandle,
    tiles: u32,
    timeline_start: TimelineTime,
    source_start: MediaTime,
    media_duration: MediaTime,
    speed: Rational,
) {
    let media_ticks = media_duration.ticks();
    if tiles == 0 || media_ticks <= 0 || clip_rect.width() < 4.0 {
        return;
    }

    let per_tile = media_ticks / i64::from(tiles).max(1);
    if per_tile <= 0 {
        return;
    }

    // Tile width on screen: how much timeline one tile of source covers. At 2×
    // a tile of source covers half as much timeline, so the thumbnails have to
    // pack in tighter — drawn unscaled they would run off the end of a
    // shortened clip and mislabel every frame under them.
    let per_tile_timeline = speed
        .inverse()
        .map_or(per_tile, |inverse| inverse.scale(per_tile))
        .max(1);
    let tile_px = (per_tile_timeline / viewport.ticks_per_pixel.max(1)) as f32;
    if tile_px < 1.0 {
        // Zoomed out so far that a tile is under a pixel; drawing it would be
        // noise, and thousands of draw calls for it.
        return;
    }

    let strip = painter.with_clip_rect(clip_rect);
    let uv_step = 1.0 / tiles as f32;

    // Walk source-tile boundaries rather than screen columns, so tiles land on
    // frame content instead of sliding as the clip scrolls.
    let first = (source_start.ticks() / per_tile).max(0);
    let mut index = first;
    while index < i64::from(tiles) {
        let tile_source = index * per_tile;
        let into_clip = speed
            .inverse()
            .map_or(tile_source - source_start.ticks(), |inverse| {
                inverse.scale(tile_source - source_start.ticks())
            });
        let x = viewport.x_of(TimelineTime::from_ticks(timeline_start.ticks() + into_clip));
        if x > clip_rect.right() {
            break;
        }

        let rect = Rect::from_min_size(
            Pos2::new(x, clip_rect.top() + 4.0),
            vec2(tile_px, clip_rect.height() - 4.0),
        );
        if rect.right() >= clip_rect.left() {
            let u0 = index as f32 * uv_step;
            strip.image(
                sheet.id(),
                rect,
                Rect::from_min_max(Pos2::new(u0, 0.0), Pos2::new(u0 + uv_step, 1.0)),
                Color32::WHITE.gamma_multiply(0.85),
            );
        }
        index += 1;
    }
}

/// The band a volume envelope is drawn in, inside a clip's rect.
///
/// Full volume runs near the top and silence along the bottom, which is the way
/// every editor draws a level: the line falling is the sound getting quieter.
/// Above 1.0 the line would leave the clip, so a boost is pinned at the top —
/// the envelope is read for its *shape*, and the exact number belongs in the
/// Inspector.
fn envelope_band(clip_rect: Rect) -> (f32, f32) {
    (clip_rect.top() + 6.0, clip_rect.bottom() - 6.0)
}

fn envelope_y(clip_rect: Rect, gain: f32) -> f32 {
    let (top, bottom) = envelope_band(clip_rect);
    bottom - gain.clamp(0.0, 1.0) * (bottom - top)
}

/// The level a y position means, the inverse of [`envelope_y`].
fn envelope_gain(clip_rect: Rect, y: f32) -> f32 {
    let (top, bottom) = envelope_band(clip_rect);
    ((bottom - y) / (bottom - top)).clamp(0.0, 1.0)
}

/// Where each key of a clip's envelope sits on screen, with its index.
///
/// Shared by the painter and the hit test so a dot is always grabbable exactly
/// where it is drawn — two copies of this arithmetic would drift apart by a
/// pixel and the dots would become mysteriously hard to hit (§46's habit,
/// applied to a much smaller thing).
fn envelope_dots(
    clip: &bettercut_editor_core::timeline::AudioClip,
    clip_rect: Rect,
    viewport: Viewport,
) -> Vec<(usize, Pos2)> {
    let Some(track) = clip
        .keyframes
        .track(bettercut_editor_core::timeline::AnimatedParameter::Gain)
    else {
        return Vec::new();
    };
    track
        .keys()
        .iter()
        .enumerate()
        .filter_map(|(index, key)| {
            // Back from source time to the timeline, through the speed.
            let into_source = key.time.ticks() - clip.source.start.ticks();
            if into_source < 0 {
                return None;
            }
            let at = clip.timeline.start
                + TimelineTime::from_ticks(bettercut_editor_core::timeline::timeline_ticks_for(
                    bettercut_editor_core::foundation::MediaTime::from_ticks(into_source),
                    clip.speed,
                ));
            if at < clip.timeline.start || at > clip.timeline.end {
                return None;
            }
            Some((
                index,
                Pos2::new(viewport.x_of(at), envelope_y(clip_rect, key.value)),
            ))
        })
        .collect()
}

/// A sound clip's volume envelope (§24), as a line across its waveform.
///
/// Sampled from [`AudioClip::gain_at`] — the same function the mixer asks — so
/// the line is not a drawing of what the keys *mean*, it is a drawing of what
/// is heard (§46). A duck that did not reach the mixer would show as a flat
/// line here.
///
/// Full volume runs near the top of the clip and silence along the bottom,
/// which is the way every editor draws a level: the line falling is the sound
/// getting quieter.
fn draw_volume_envelope(
    painter: &egui::Painter,
    clip_rect: Rect,
    viewport: Viewport,
    clip: &bettercut_editor_core::timeline::AudioClip,
) {
    if clip_rect.width() < 4.0 {
        return;
    }
    let painter = painter.with_clip_rect(clip_rect);

    // One point every three pixels: finer than the eye resolves at timeline
    // scale, and a fraction of the shapes a point per pixel would emit.
    let mut points = Vec::new();
    let mut x = clip_rect.left();
    while x <= clip_rect.right() {
        let at = TimelineTime::from_ticks(viewport.raw_tick_of(x));
        points.push(Pos2::new(x, envelope_y(clip_rect, clip.gain_at(at))));
        x += 3.0;
    }
    if points.len() < 2 {
        return;
    }

    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.5, theme::automation()),
    ));

    // A dot at each key, so it is clear the shape is made of points that can be
    // moved rather than being a picture of the sound.
    for (_, at) in envelope_dots(clip, clip_rect, viewport) {
        painter.circle_filled(at, 3.0, theme::automation());
    }
}

/// Paint the peaks a clip covers, mirrored around its centre line.
///
/// One vertical segment per pixel column, each showing the loudest sample in
/// that column. Drawing every peak instead would emit thousands of shapes for a
/// clip a few hundred pixels wide, and they would land on the same pixels
/// anyway — the column *is* the unit of resolution on screen.
fn draw_waveform(
    painter: &egui::Painter,
    clip_rect: Rect,
    viewport: Viewport,
    waveform: &bettercut_cache::Waveform,
    timeline_start: TimelineTime,
    source_start: MediaTime,
) {
    if waveform.peaks.is_empty() || clip_rect.width() < 2.0 {
        return;
    }

    let centre = clip_rect.center().y;
    // Leave the top cap and a small margin alone so the shape reads as being
    // inside the clip rather than overflowing it.
    let half_height = ((clip_rect.height() - 10.0) / 2.0).max(1.0);
    let per_second = f64::from(waveform.peaks_per_second);
    let ticks_per_second = bettercut_editor_core::foundation::TICKS_PER_SECOND as f64;

    // Bucket index for a timeline instant: back to the clip's start, forward to
    // where that sits in the media, then into peaks.
    let bucket_at = |x: f32| -> usize {
        let tick = viewport.raw_tick_of(x);
        let into_clip = (tick - timeline_start.ticks()).max(0);
        let source_ticks = source_start.ticks() + into_clip;
        ((source_ticks as f64 / ticks_per_second) * per_second) as usize
    };

    let mut segments = Vec::with_capacity(clip_rect.width() as usize + 1);
    let mut x = clip_rect.left();
    while x < clip_rect.right() {
        let next = x + 1.0;
        let peak = waveform.peak_over(bucket_at(x), bucket_at(next).max(bucket_at(x) + 1));
        let magnitude = peak.magnitude();

        // Always draw at least a hairline: a silent passage is information, and
        // a gap in the middle of a clip reads as missing data.
        let extent = (magnitude * half_height).max(0.5);
        segments.push([Pos2::new(x, centre - extent), Pos2::new(x, centre + extent)]);
        x = next;
    }

    let colour = theme::clip_text().gamma_multiply(0.55);
    for [from, to] in segments {
        painter.line_segment([from, to], Stroke::new(1.0, colour));
    }
}

#[allow(clippy::too_many_arguments)]
/// A clip's fades or a title's entrance and exit, as ramps at its ends.
///
/// Shaded above a line from silence to full, the way every editor draws a
/// fade: the shape says "quiet here, rising" without a legend. Positioned from
/// the clip's true start, not its on-screen one, so a clip scrolled half out
/// of view still shows its ramp where it really is.
fn draw_ramps(
    painter: &egui::Painter,
    clip_rect: Rect,
    viewport: Viewport,
    range: TimelineRange,
    (rise, fall): (i64, i64),
) {
    if rise <= 0 && fall <= 0 {
        return;
    }
    let painter = painter.with_clip_rect(clip_rect);
    let shade = Color32::from_black_alpha(90);
    let line = Stroke::new(1.0, Color32::from_white_alpha(170));
    let (top, bottom) = (clip_rect.top() + 4.0, clip_rect.bottom());

    if rise > 0 {
        let x0 = viewport.x_of(range.start);
        let x1 = viewport.x_of(range.start + TimelineTime::from_ticks(rise));
        let points = vec![
            Pos2::new(x0, top),
            Pos2::new(x1, top),
            Pos2::new(x0, bottom),
        ];
        painter.add(egui::Shape::convex_polygon(points, shade, Stroke::NONE));
        painter.line_segment([Pos2::new(x0, bottom), Pos2::new(x1, top)], line);
    }
    if fall > 0 {
        let x1 = viewport.x_of(range.end);
        let x0 = viewport.x_of(range.end - TimelineTime::from_ticks(fall));
        let points = vec![
            Pos2::new(x0, top),
            Pos2::new(x1, top),
            Pos2::new(x1, bottom),
        ];
        painter.add(egui::Shape::convex_polygon(points, shade, Stroke::NONE));
        painter.line_segment([Pos2::new(x0, top), Pos2::new(x1, bottom)], line);
    }
}

fn draw_clip(
    painter: &egui::Painter,
    lane: Rect,
    viewport: Viewport,
    visual: ClipVisual<'_>,
    id: ClipId,
    track: TrackId,
    interaction: &mut Interaction,
) {
    let x0 = viewport.x_of(visual.range.start).max(viewport.origin_x);
    let x1 = viewport.x_of(visual.range.end).min(lane.right());
    if x1 <= x0 {
        return;
    }

    let clip_rect = Rect::from_min_max(
        Pos2::new(x0, lane.top() + 3.0),
        Pos2::new(x1, lane.bottom() - 3.0),
    );

    // Hit-test here, where the rect exists. Acted on after the draw pass
    // releases its borrow of the project.
    if let Some(pos) = interaction.pointer
        && clip_rect.contains(pos)
    {
        interaction.hit = Some(ClipHit {
            clip: id,
            track,
            rect: clip_rect,
            range: visual.range,
        });

        // A fade handle, checked first: the smallest target wins.
        if visual.fade_handles {
            let [start, end] = fade_handle_positions(
                clip_rect,
                visual.range,
                visual.ramps,
                viewport.origin_x,
                viewport.scroll_ticks,
                viewport.ticks_per_pixel,
            );
            for (handle, edge) in [
                (start, crate::state::FadeEdge::In),
                (end, crate::state::FadeEdge::Out),
            ] {
                if handle.distance(pos) <= FADE_HANDLE_REACH {
                    interaction.fade = Some(FadeHit {
                        clip: id,
                        edge,
                        range: visual.range,
                    });
                    break;
                }
            }
        }

        // A volume point under the pointer, checked here for the same reason:
        // this is where the clip's rect is known. Generous by a few pixels,
        // because a 3 px dot is not a 3 px target.
        if let Some(clip) = visual.volume {
            for (index, dot) in envelope_dots(clip, clip_rect, viewport) {
                if dot.distance(pos) <= 7.0 {
                    interaction.envelope = Some(EnvelopeHit {
                        clip: id,
                        index,
                        rect: clip_rect,
                    });
                    break;
                }
            }
            // On the line between the points: where a new point can be put.
            let at = TimelineTime::from_ticks(viewport.raw_tick_of(pos.x));
            if interaction.envelope.is_none()
                && (envelope_y(clip_rect, clip.gain_at(at)) - pos.y).abs() <= 7.0
            {
                interaction.envelope_line = Some((id, clip_rect));
            }
        }
    }

    let top = visual.label_color.unwrap_or(visual.top);
    let (body, top) = if visual.track_enabled {
        (visual.body, top)
    } else {
        (theme::disabled(), theme::disabled())
    };

    // The clip being dragged is dimmed in place; the preview shows where it
    // will land. Without the dimming there appear to be two copies of it.
    let (body, top) = if visual.dragging {
        (body.gamma_multiply(0.35), top.gamma_multiply(0.35))
    } else {
        (body, top)
    };

    painter.rect_filled(clip_rect, theme::CLIP_CORNER_RADIUS, body);

    // A brighter cap along the top edge: at a glance it separates stacked clips
    // far better than a border does.
    let cap = Rect::from_min_max(
        clip_rect.min,
        Pos2::new(clip_rect.right(), clip_rect.top() + 4.0),
    );
    painter.rect_filled(cap, theme::CLIP_CORNER_RADIUS, top);

    // A colour clip shows its colours, fading top to bottom as it does in
    // the frame. Only while it plays and is not being dragged: otherwise the
    // dimmed body says so, as it does on any clip.
    if let Some((upper, lower)) = visual.colour
        && visual.track_enabled
        && !visual.dragging
    {
        let area = Rect::from_min_max(
            Pos2::new(clip_rect.left() + 1.0, clip_rect.top() + 4.0),
            Pos2::new(clip_rect.right() - 1.0, clip_rect.bottom() - 1.0),
        );
        if area.width() > 0.0 && area.height() > 0.0 {
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(area.left_top(), upper);
            mesh.colored_vertex(area.right_top(), upper);
            mesh.colored_vertex(area.right_bottom(), lower);
            mesh.colored_vertex(area.left_bottom(), lower);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(mesh));
        }
    }

    if let Some((sheet, tiles, source_start)) = visual.filmstrip {
        draw_filmstrip(
            painter,
            clip_rect,
            viewport,
            sheet,
            tiles,
            visual.range.start,
            source_start,
            visual.duration_of_media,
            visual.speed,
        );
    }

    // Under the label and over the body, so the file name stays readable.
    if let Some((waveform, source_start)) = visual.waveform {
        draw_waveform(
            painter,
            clip_rect,
            viewport,
            waveform,
            visual.range.start,
            source_start,
        );
    }

    // Over the waveform: the envelope is what is being *done* to those peaks,
    // so it has to sit on top of them to be read against them.
    if let Some(clip) = visual.volume {
        draw_volume_envelope(painter, clip_rect, viewport, clip);
    }

    draw_ramps(painter, clip_rect, viewport, visual.range, visual.ramps);

    // The handles, shown where they can be used: on a selected clip, or one
    // the pointer is over. Everywhere at once, they would clutter every lane.
    let hovered = interaction.pointer.is_some_and(|p| clip_rect.contains(p));
    if visual.fade_handles && (visual.selected || hovered) {
        let handles = fade_handle_positions(
            clip_rect,
            visual.range,
            visual.ramps,
            viewport.origin_x,
            viewport.scroll_ticks,
            viewport.ticks_per_pixel,
        );
        for centre in handles {
            let square =
                Rect::from_center_size(centre, egui::vec2(FADE_HANDLE_SIZE, FADE_HANDLE_SIZE));
            painter.rect_filled(square, 1.0, theme::fade_handle());
            painter.rect_stroke(
                square,
                1.0,
                Stroke::new(1.0, Color32::from_black_alpha(180)),
                egui::StrokeKind::Outside,
            );
        }
    }

    if visual.selected {
        painter.rect_stroke(
            clip_rect,
            theme::CLIP_CORNER_RADIUS,
            Stroke::new(2.0, theme::selection()),
            StrokeKind::Inside,
        );
        draw_trim_handles(painter, clip_rect);
    } else if visual.linked_to_selection {
        // Thinner and paler than the selection itself: "this comes along", not
        // "this is what you picked". No trim handles — the edit is made on the
        // clip that is selected, and this one follows.
        painter.rect_stroke(
            clip_rect,
            theme::CLIP_CORNER_RADIUS,
            Stroke::new(1.0, theme::selection().gamma_multiply(0.6)),
            StrokeKind::Inside,
        );
    }

    if let Some((keyframes, source_start)) = visual.keyframes {
        draw_keyframes(
            painter,
            clip_rect,
            viewport,
            keyframes,
            visual.range.start,
            source_start,
        );
    }

    if let Some(transition) = visual.transition {
        draw_transition(painter, lane, viewport, transition, visual.range.end);
    }

    // Top-right, where they do not collide with the file name on the left, and
    // only for what is *not* ordinary: a badge on every clip would be noise.
    draw_clip_badges(painter, clip_rect, &visual);

    // Only label a clip wide enough to read it; below that the text is noise.
    if clip_rect.width() > 46.0 {
        let text_painter = painter.with_clip_rect(clip_rect.shrink(4.0));
        text_painter.text(
            Pos2::new(clip_rect.left() + 7.0, clip_rect.center().y + 2.0),
            Align2::LEFT_CENTER,
            visual.label,
            FontId::proportional(12.0),
            theme::clip_text(),
        );
    }
}

/// What a clip is doing that the picture on the timeline cannot show.
///
/// A hold, a speed, a mask, a key, a blend other than normal — each looks like
/// an ordinary clip in the lane, and the user is left asking why the preview
/// disagrees with what they see here.
///
/// One right-aligned row rather than a badge each at the same anchor, which is
/// what the hold and the speed used to be: they could not both appear because
/// nothing could hold both, and now a clip can have three of these at once.
fn draw_clip_badges(painter: &egui::Painter, clip_rect: Rect, visual: &ClipVisual<'_>) {
    if clip_rect.width() <= 34.0 {
        return;
    }

    let mut badges: Vec<(String, Color32)> = Vec::new();
    if visual.frozen {
        badges.push(("hold".to_owned(), theme::marker()));
    }
    if !visual.speed.is_one() {
        badges.push((format!("{:.2}×", visual.speed.as_f64()), theme::selection()));
    }
    for effect in &visual.effects {
        badges.push(((*effect).to_owned(), theme::transition()));
    }

    let font = FontId::proportional(10.0);
    let mut right = clip_rect.right() - 5.0;
    for (label, colour) in badges {
        let galley = painter.layout_no_wrap(label, font.clone(), theme::background());
        let box_rect = Rect::from_min_size(
            Pos2::new(right - galley.size().x - 4.0, clip_rect.top() + 5.0),
            galley.size() + vec2(8.0, 3.0),
        );
        // Stop rather than overrun the label: a badge sitting on top of the
        // file name tells the user less than no badge at all.
        if box_rect.left() < clip_rect.left() + 4.0 {
            break;
        }
        painter.rect_filled(box_rect, 3, colour);
        painter.galley(
            Pos2::new(box_rect.left() + 4.0, box_rect.top() + 1.0),
            galley,
            theme::background(),
        );
        right = box_rect.left() - 4.0;
    }
}

/// Diamonds along the bottom edge of a clip, one per instant that has a key.
///
/// Merged across parameters rather than one row each: at this height a lane has
/// room for one strip of marks, and what the user needs from the timeline is
/// *where* the animation happens. Which parameter moves is the inspector's job.
fn draw_keyframes(
    painter: &egui::Painter,
    clip_rect: Rect,
    viewport: Viewport,
    keyframes: &bettercut_editor_core::timeline::Keyframes,
    clip_start: TimelineTime,
    source_start: MediaTime,
) {
    const RADIUS: f32 = 3.5;

    // Too short to place a mark inside without it covering the whole clip.
    if clip_rect.width() < RADIUS * 4.0 {
        return;
    }
    let y = clip_rect.bottom() - RADIUS - 1.0;

    for time in keyframes.times() {
        // Source ticks back to timeline ticks, integer throughout (§9, §74).
        let at =
            TimelineTime::from_ticks(clip_start.ticks() + (time.ticks() - source_start.ticks()));
        let x = viewport.x_of(at);
        // Keys can sit outside the visible span, and a trimmed clip can hold
        // keys outside itself entirely — they are anchored to the media, and
        // trimming does not delete them so that trimming back restores them.
        if x < clip_rect.left() + RADIUS || x > clip_rect.right() - RADIUS {
            continue;
        }

        // §24: the curve, in the shape of the mark. A row of identical
        // diamonds cannot say which keys were eased, and easing a move is the
        // difference between it looking deliberate and looking mechanical.
        let outline = Stroke::new(1.0, theme::timeline_background());
        let at = Pos2::new(x, y);
        match keyframes.interpolation_at(time) {
            // A corner: the value is held and then jumps, which is what a
            // square edge looks like.
            Some(Interpolation::Hold) => {
                painter.rect(
                    Rect::from_center_size(at, vec2(RADIUS * 1.7, RADIUS * 1.7)),
                    0.0,
                    theme::keyframe(),
                    outline,
                    StrokeKind::Middle,
                );
            }
            // Round: the curve comes in and leaves smoothly.
            Some(Interpolation::Linear) | None => {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(x, y - RADIUS),
                        Pos2::new(x + RADIUS, y),
                        Pos2::new(x, y + RADIUS),
                        Pos2::new(x - RADIUS, y),
                    ],
                    theme::keyframe(),
                    outline,
                ));
            }
            Some(_) => {
                painter.circle(at, RADIUS, theme::keyframe(), outline);
            }
        }
    }
}

/// Grab bars on a selected clip's edges, so trimming is discoverable rather
/// than something the user has to already know about.
fn draw_trim_handles(painter: &egui::Painter, clip_rect: Rect) {
    if clip_rect.width() < TRIM_HANDLE_PIXELS * 3.0 {
        return;
    }
    for x in [clip_rect.left(), clip_rect.right() - TRIM_HANDLE_PIXELS] {
        let handle = Rect::from_min_size(
            Pos2::new(x, clip_rect.top()),
            vec2(TRIM_HANDLE_PIXELS, clip_rect.height()),
        );
        painter.rect_filled(handle, theme::CLIP_CORNER_RADIUS, theme::selection());
    }
}

/// Ghost of the dragged clip at its would-be position, plus the snap guide.
fn draw_drag_preview(
    painter: &egui::Painter,
    viewport: Viewport,
    state: &UiState,
    lanes: &[LaneLayout],
) {
    let Some(drag) = state.drag.as_ref() else {
        return;
    };
    let Some(lane) = lanes.iter().find(|l| l.track == drag.target_track) else {
        return;
    };

    let x0 = viewport.x_of(drag.preview.start);
    let x1 = viewport.x_of(drag.preview.end);
    let ghost = Rect::from_min_max(
        Pos2::new(x0.max(viewport.origin_x), lane.rect.top() + 3.0),
        Pos2::new(x1.min(lane.rect.right()), lane.rect.bottom() - 3.0),
    );

    // A roll: the cut, where it will be, and how far it has come.
    if let Some(roll) = drag.roll {
        let x = viewport.x_of(roll.to);
        if x >= viewport.origin_x && x <= lane.rect.right() {
            painter.line_segment(
                [
                    Pos2::new(x, lane.rect.top() - 6.0),
                    Pos2::new(x, lane.rect.bottom() + 6.0),
                ],
                Stroke::new(3.0, theme::selection()),
            );
            let seconds = (roll.to.ticks() - roll.cut.ticks()) as f64
                / bettercut_editor_core::foundation::TICKS_PER_SECOND as f64;
            let label = format!("Roll {seconds:+.2} s");
            painter.text(
                Pos2::new(x + 6.0, lane.rect.top() + 4.0),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(13.0),
                Color32::WHITE,
            );
        }
        return;
    }

    // A slip: the clip itself stays put, so rather than a ghost, say how far
    // through its file it has moved.
    if drag.mode == DragMode::Slip {
        if ghost.width() > 0.0 {
            let seconds =
                drag.slip as f64 / bettercut_editor_core::foundation::TICKS_PER_SECOND as f64;
            let label = if drag.slip == 0 && drag.moved {
                "Slip: no more footage this way".to_owned()
            } else {
                format!("Slip {seconds:+.2} s")
            };
            painter.rect_stroke(
                ghost,
                theme::CLIP_CORNER_RADIUS,
                Stroke::new(2.0, theme::selection()),
                StrokeKind::Inside,
            );
            painter.text(
                ghost.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(13.0),
                Color32::WHITE,
            );
        }
        return;
    }

    if ghost.width() > 0.0 {
        // Red when the drop would be refused — a video clip over an audio
        // track. Better to show it before the release than to error after.
        let tint = if drag.target_invalid {
            theme::error_text()
        } else {
            theme::selection()
        };
        painter.rect_filled(ghost, theme::CLIP_CORNER_RADIUS, tint.gamma_multiply(0.25));
        painter.rect_stroke(
            ghost,
            theme::CLIP_CORNER_RADIUS,
            Stroke::new(2.0, tint),
            StrokeKind::Inside,
        );

        // An insert: a bar at the drop point, through every lane, because
        // everything from there on is about to move along.
        if drag.insert && drag.mode == DragMode::Move && !drag.target_invalid {
            let top = lanes
                .iter()
                .map(|l| l.rect.top())
                .fold(lane.rect.top(), f32::min);
            let bottom = lanes
                .iter()
                .map(|l| l.rect.bottom())
                .fold(lane.rect.bottom(), f32::max);
            if x0 >= viewport.origin_x {
                painter.line_segment(
                    [Pos2::new(x0, top), Pos2::new(x0, bottom)],
                    Stroke::new(3.0, theme::selection()),
                );
            }
            painter.text(
                Pos2::new(ghost.left() + 6.0, ghost.top() + 4.0),
                Align2::LEFT_TOP,
                "Insert",
                FontId::proportional(13.0),
                Color32::WHITE,
            );
        }
    }

    // §12: the linked partners, on their own lanes, where the editor will put
    // them on release. Paler than the dragged clip's ghost — "this comes
    // along", not "this is what you are holding" — and never red: a partner
    // only follows in time, so it cannot land on the wrong kind of track.
    if drag.moved {
        for (track, range) in &drag.partners {
            let Some(partner_lane) = lanes.iter().find(|l| l.track == *track) else {
                continue;
            };
            let landing = drag.partner_preview(*range);
            let x0 = viewport.x_of(landing.start).max(viewport.origin_x);
            let x1 = viewport.x_of(landing.end).min(partner_lane.rect.right());
            if x1 <= x0 {
                continue;
            }
            let ghost = Rect::from_min_max(
                Pos2::new(x0, partner_lane.rect.top() + 3.0),
                Pos2::new(x1, partner_lane.rect.bottom() - 3.0),
            );
            painter.rect_filled(
                ghost,
                theme::CLIP_CORNER_RADIUS,
                theme::selection().gamma_multiply(0.12),
            );
            painter.rect_stroke(
                ghost,
                theme::CLIP_CORNER_RADIUS,
                Stroke::new(1.0, theme::selection().gamma_multiply(0.7)),
                StrokeKind::Inside,
            );
        }
    }

    // A snap the user cannot see reads as the drag being buggy.
    if let Some(target) = drag.snapped_to {
        let x = viewport.x_of(target.time);
        if x >= viewport.origin_x {
            painter.line_segment(
                [
                    Pos2::new(x, lane.rect.top() - 40.0),
                    Pos2::new(x, lane.rect.bottom() + 40.0),
                ],
                Stroke::new(1.0, theme::selection()),
            );
        }
    }
}

/// §78's suggested cuts, shaded where they would come out.
///
/// Over the lanes rather than on one clip: the cut takes the picture and its
/// sound together, and showing it on both is what makes that obvious before
/// the button is pressed.
/// The moments a highlight search found: shaded like a suggested cut, but
/// in the marker's colour, because these are what *stays*.
fn draw_suggested_keeps(
    painter: &egui::Painter,
    rect: Rect,
    viewport: Viewport,
    ranges: &[TimelineRange],
) {
    for range in ranges {
        let x0 = viewport.x_of(range.start).max(viewport.origin_x);
        let x1 = viewport.x_of(range.end).min(rect.right());
        if x1 <= x0 {
            continue;
        }
        let band = Rect::from_min_max(
            Pos2::new(x0, rect.top() + theme::RULER_HEIGHT),
            Pos2::new(x1, rect.bottom()),
        );
        painter.rect_filled(band, 0, theme::marker().gamma_multiply(0.22));
        painter.rect_stroke(
            band,
            0,
            Stroke::new(1.0, theme::marker().gamma_multiply(0.8)),
            StrokeKind::Inside,
        );
    }
}

fn draw_suggested_cuts(
    painter: &egui::Painter,
    rect: Rect,
    viewport: Viewport,
    ranges: &[TimelineRange],
) {
    for range in ranges {
        let x0 = viewport.x_of(range.start).max(viewport.origin_x);
        let x1 = viewport.x_of(range.end).min(rect.right());
        if x1 <= x0 {
            continue;
        }
        let band = Rect::from_min_max(
            Pos2::new(x0, rect.top() + theme::RULER_HEIGHT),
            Pos2::new(x1, rect.bottom()),
        );
        painter.rect_filled(band, 0, theme::playhead().gamma_multiply(0.28));
        painter.rect_stroke(
            band,
            0,
            Stroke::new(1.0, theme::playhead().gamma_multiply(0.8)),
            StrokeKind::Inside,
        );
    }
}

/// Suggested splits: a dashed line down the lanes at each cut detection found.
///
/// A line rather than a band, because a split has no width — shading either
/// side of it, the way a silence is shaded, would suggest something is being
/// taken out when nothing is.
fn draw_suggested_splits(
    painter: &egui::Painter,
    rect: Rect,
    viewport: Viewport,
    cuts: &[TimelineTime],
) {
    for cut in cuts {
        let x = viewport.x_of(*cut);
        if x < viewport.origin_x || x > rect.right() {
            continue;
        }
        let top = rect.top() + theme::RULER_HEIGHT;
        // Dashes, so it reads as a proposal rather than as an edge that is
        // already there.
        let mut y = top;
        while y < rect.bottom() {
            let to = (y + 5.0).min(rect.bottom());
            painter.line_segment(
                [Pos2::new(x, y), Pos2::new(x, to)],
                Stroke::new(1.5, theme::playhead().gamma_multiply(0.9)),
            );
            y = to + 4.0;
        }
    }
}

/// Markers: a flag on the ruler, and a faint line down through the lanes so a
/// clip can be lined up with one by eye before snapping takes over.
/// The in and out marks: brackets on the ruler, and the span between them
/// tinted across every lane, so what an export of the range will hold is
/// visible at a glance.
fn draw_in_out(
    painter: &egui::Painter,
    rect: Rect,
    viewport: Viewport,
    mark_in: Option<TimelineTime>,
    mark_out: Option<TimelineTime>,
) {
    let top = rect.top() + theme::RULER_HEIGHT;
    let left = viewport.origin_x;
    if let (Some(start), Some(end)) = (mark_in, mark_out)
        && start < end
    {
        let x0 = viewport.x_of(start).max(left);
        let x1 = viewport.x_of(end).min(rect.right());
        if x1 > x0 {
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1, rect.bottom())),
                0.0,
                theme::in_out_span(),
            );
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(x0, top - 4.0), Pos2::new(x1, top)),
                0.0,
                theme::in_out_mark(),
            );
        }
    }
    let bracket = |at: TimelineTime, opening: bool| {
        let x = viewport.x_of(at);
        if x < left || x > rect.right() {
            return;
        }
        let arm = if opening { 6.0 } else { -6.0 };
        let stroke = Stroke::new(2.0, theme::in_out_mark());
        painter.line_segment([Pos2::new(x, rect.top() + 2.0), Pos2::new(x, top)], stroke);
        painter.line_segment(
            [
                Pos2::new(x, rect.top() + 2.0),
                Pos2::new(x + arm, rect.top() + 2.0),
            ],
            stroke,
        );
        painter.line_segment([Pos2::new(x, top), Pos2::new(x + arm, top)], stroke);
    };
    if let Some(at) = mark_in {
        bracket(at, true);
    }
    if let Some(at) = mark_out {
        bracket(at, false);
    }
}

/// The render bar: a stripe under the ruler over every baked stretch — solid
/// where the file is still the edit, amber where an edit has moved on and the
/// preview is compositing again (`editor_core::render_in_place`).
///
/// Amber rather than nothing: a stale bake is why a stretch that played at
/// rate a minute ago does not any more, and that is worth being able to see.
fn draw_render_bar(
    painter: &egui::Painter,
    rect: Rect,
    viewport: Viewport,
    project: &bettercut_editor_core::project_format::Project,
    sequence: &bettercut_editor_core::timeline::Sequence,
) {
    let top = rect.top() + theme::RULER_HEIGHT - 3.0;
    for render in &sequence.renders {
        let x0 = viewport.x_of(render.range.start).max(viewport.origin_x);
        let x1 = viewport.x_of(render.range.end).min(rect.right());
        if x1 <= x0 {
            continue;
        }
        let current =
            bettercut_playback::rendered::usable(project, sequence, render.range.start).is_some();
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1, top + 3.0)),
            0.0,
            if current {
                theme::rendered()
            } else {
                theme::rendered_stale()
            },
        );
    }
}

fn draw_markers(
    painter: &egui::Painter,
    rect: Rect,
    viewport: Viewport,
    markers: &[bettercut_editor_core::timeline::Marker],
) {
    let visible = viewport.origin_x..=rect.right();
    for marker in markers {
        let x = viewport.x_of(marker.time);
        if !visible.contains(&x) {
            continue;
        }
        let colour = marker_colour(marker.color);
        // A mark about a passage is drawn as the passage: a band under the
        // ruler from where it starts to where it ends, so "this whole bit"
        // reads as a bit rather than as a flag with a number beside it.
        if marker.is_ranged() {
            let end = viewport.x_of(marker.end()).min(rect.right());
            let top = rect.top() + theme::RULER_HEIGHT;
            if end > x {
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(x, top - 4.0), Pos2::new(end, top)),
                    0.0,
                    colour.gamma_multiply(0.7),
                );
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(x, top), Pos2::new(end, rect.bottom())),
                    0.0,
                    colour.gamma_multiply(0.06),
                );
            }
        }
        painter.line_segment(
            [
                Pos2::new(x, rect.top() + theme::RULER_HEIGHT),
                Pos2::new(x, rect.bottom()),
            ],
            Stroke::new(1.0, colour.gamma_multiply(0.35)),
        );
        let tip = Pos2::new(x, rect.top() + theme::RULER_HEIGHT - 2.0);
        painter.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(x - 5.0, tip.y - 8.0),
                Pos2::new(x + 5.0, tip.y - 8.0),
                tip,
            ],
            colour,
            Stroke::NONE,
        ));
        if !marker.label.is_empty() {
            painter.text(
                Pos2::new(x + 7.0, tip.y - 9.0),
                Align2::LEFT_CENTER,
                &marker.label,
                FontId::proportional(10.0),
                colour,
            );
        }
    }
}

/// A marker's colour: its own when it has one, otherwise the marker green.
pub fn marker_colour(label: bettercut_editor_core::timeline::ColorLabel) -> Color32 {
    label
        .rgb()
        .map_or(theme::marker(), |[r, g, b]| Color32::from_rgb(r, g, b))
}

/// A clip's own marks (`Sequence::clip_marks`), as short ticks hanging from
/// the top of the clip: on the clip rather than through the whole timeline,
/// because that is the difference between them and the ruler's markers.
fn draw_clip_marks(
    painter: &egui::Painter,
    viewport: Viewport,
    sequence: &Sequence,
    lanes: &[LaneLayout],
) {
    if sequence.clip_marks.is_empty() {
        return;
    }
    // One pass over the marked clips: each mark is drawn on the lane its
    // clip sits on, at the height that lane happens to be.
    let mut marked: Vec<ClipId> = sequence.clip_marks.iter().map(|m| m.clip).collect();
    marked.sort_unstable();
    marked.dedup();
    for clip in marked {
        let Some(span) = sequence.clip_span(clip) else {
            continue;
        };
        let Some(lane) = lanes.iter().find(|lane| lane.track == span.track) else {
            continue;
        };
        for (at, mark) in sequence.clip_marks_on_timeline(clip) {
            let x = viewport.x_of(at);
            if x < viewport.origin_x || x > lane.rect.right() + 1.0 {
                continue;
            }
            let colour = match mark.color.rgb() {
                Some([r, g, b]) => Color32::from_rgb(r, g, b),
                None => theme::marker(),
            };
            let top = lane.rect.top();
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x - 4.0, top),
                    Pos2::new(x + 4.0, top),
                    Pos2::new(x, top + 7.0),
                ],
                colour,
                Stroke::new(1.0, theme::timeline_background()),
            ));
        }
    }
}

fn draw_playhead(painter: &egui::Painter, rect: Rect, viewport: Viewport, playhead: TimelineTime) {
    let x = viewport.x_of(playhead);
    if x < viewport.origin_x || x > rect.right() {
        return;
    }

    painter.line_segment(
        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
        Stroke::new(1.5, theme::playhead()),
    );

    // Grab handle at the top, so the playhead reads as draggable.
    let handle = Rect::from_center_size(
        Pos2::new(x, rect.top() + theme::RULER_HEIGHT / 2.0),
        vec2(11.0, 11.0),
    );
    painter.rect_filled(handle, 2, theme::playhead());
}
