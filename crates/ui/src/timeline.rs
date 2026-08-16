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

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::project_format::Project;
use bettercut_editor_core::timeline::{Sequence, TimelineRange, TrackKind, snap};
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

/// Pointer state for this frame, plus what the draw pass hit-tested.
struct Interaction {
    pointer: Option<Pos2>,
    clicked: bool,
    additive: bool,
    hit: Option<ClipHit>,
}

pub fn draw(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    let full = ui.available_rect_before_wrap();
    let (rect, response) = ui.allocate_exact_size(full.size(), Sense::click_and_drag());

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0, theme::TIMELINE_BACKGROUND);

    if editor.project().active().is_none() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No sequence",
            FontId::proportional(14.0),
            theme::DISABLED,
        );
        return;
    }

    let viewport = Viewport {
        origin_x: rect.left() + theme::TRACK_HEADER_WIDTH,
        scroll_ticks: state.scroll_ticks,
        ticks_per_pixel: state.ticks_per_pixel(),
    };
    let lane_width = (rect.width() - theme::TRACK_HEADER_WIDTH).max(1.0);

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
        hit: None,
    };

    // The draw pass borrows the project immutably.
    let lanes = {
        let project = editor.project();
        let Some(sequence) = project.active() else {
            return;
        };
        let playhead = editor.playhead();

        draw_ruler(&painter, rect, viewport, state, lane_width);
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
                "Import media, then press “Add to timeline” to place a clip here",
                FontId::proportional(13.0),
                theme::DISABLED,
            );
        }

        lanes
    };

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
    if state.drag.is_none()
        && ui.input(|i| i.pointer.primary_pressed())
        && pos.x >= viewport.origin_x
        && pos.y >= rect.top() + theme::RULER_HEIGHT
        && let Some(hit) = interaction.hit
    {
        begin_drag(hit, pos, interaction.additive, viewport, state);
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
    });
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
        }
    }

    state.needs_repaint = true;
    if selected > 0 {
        state.info(format!("Selected {selected} clip(s)"));
    }
}

fn begin_drag(hit: ClipHit, pos: Pos2, additive: bool, viewport: Viewport, state: &mut UiState) {
    // Near an edge means trim; anywhere else means move. The handle is a fixed
    // pixel width so it stays grabbable at every zoom level.
    let mode = if pos.x - hit.rect.left() <= TRIM_HANDLE_PIXELS {
        DragMode::TrimStart
    } else if hit.rect.right() - pos.x <= TRIM_HANDLE_PIXELS {
        DragMode::TrimEnd
    } else {
        DragMode::Move
    };

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
    });

    // Dragging an unselected clip selects it, so the inspector follows.
    //
    // Except when Ctrl is held: that press is the start of a Ctrl+click, and
    // the release below toggles the clip. Selecting it here meant the toggle
    // immediately removed it again — Ctrl+click could never add a second clip
    // to the selection, it only ever cleared it.
    if !additive && !state.selected_clips.contains(&hit.clip) {
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
    }

    state.needs_repaint = true;
}

/// Turn the finished drag into exactly one command (§11).
fn commit_drag(drag: DragState, editor: &mut Editor, state: &mut UiState) {
    // A drag that changed nothing must not create an undo entry.
    if drag.preview == drag.original && drag.target_track == drag.source_track {
        return;
    }

    let result = match drag.mode {
        DragMode::Move => editor.move_clip(
            drag.source_track,
            drag.target_track,
            drag.clip,
            drag.preview.start,
        ),
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
    };

    if let Err(err) = result {
        // The clip stays where it was; say why rather than silently snapping
        // back, which reads as the drag having been ignored.
        state.error(err.to_string());
    }
}

// ---- drawing -------------------------------------------------------------

fn draw_ruler(
    painter: &egui::Painter,
    rect: Rect,
    viewport: Viewport,
    state: &UiState,
    lane_width: f32,
) {
    let ruler = Rect::from_min_size(rect.min, vec2(rect.width(), theme::RULER_HEIGHT));
    painter.rect_filled(ruler, 0, theme::TRACK_HEADER);

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
                Stroke::new(1.0, theme::RULER_TEXT),
            );
            painter.text(
                Pos2::new(x + 4.0, ruler.top() + 4.0),
                Align2::LEFT_TOP,
                t.format_timecode(),
                FontId::monospace(11.0),
                theme::RULER_TEXT,
            );
            painter.line_segment(
                [Pos2::new(x, ruler.bottom()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, theme::GRID_LINE),
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
        theme::DISABLED,
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
    let mut y = rect.top() + theme::RULER_HEIGHT + theme::TRACK_GAP;

    // Video tracks top-down in reverse index order: index 0 is the bottom
    // compositing layer (§22), and editors conventionally show that layer
    // nearest the audio tracks.
    for (row, track) in sequence.video_tracks.iter().enumerate().rev() {
        let lane = Rect::from_min_size(
            Pos2::new(rect.left(), y),
            vec2(rect.width(), theme::TRACK_HEIGHT),
        );
        draw_lane_background(painter, lane, viewport, row);
        draw_track_header(painter, lane, &track.name, track.enabled, track.locked);

        for clip in track.clips_in_range(visible) {
            let label = project
                .media_asset(clip.media_id)
                .map_or("(missing media)", |m| m.file_name.as_str());
            draw_clip(
                painter,
                lane,
                viewport,
                ClipVisual {
                    range: clip.timeline,
                    label,
                    selected: state.selected_clips.contains(&clip.id),
                    track_enabled: track.enabled,
                    dragging: state.drag.as_ref().is_some_and(|d| d.clip == clip.id),
                    body: theme::VIDEO_CLIP,
                    top: theme::VIDEO_CLIP_TOP,
                    waveform: None,
                    filmstrip: strips
                        .get(&clip.media_id)
                        .map(|(handle, tiles)| (handle, *tiles, clip.source.start)),
                    duration_of_media: project
                        .media_asset(clip.media_id)
                        .map_or(MediaTime::ZERO, |m| m.duration),
                    keyframes: (!clip.keyframes.is_empty())
                        .then_some((&clip.keyframes, clip.source.start)),
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
        y += theme::TRACK_HEIGHT + theme::TRACK_GAP;
    }

    for (row, track) in sequence.audio_tracks.iter().enumerate() {
        let lane = Rect::from_min_size(
            Pos2::new(rect.left(), y),
            vec2(rect.width(), theme::TRACK_HEIGHT),
        );
        draw_lane_background(painter, lane, viewport, row + 1);
        draw_track_header(painter, lane, &track.name, track.enabled, track.locked);

        for clip in track.clips_in_range(visible) {
            let label = project
                .media_asset(clip.media_id)
                .map_or("(missing media)", |m| m.file_name.as_str());
            draw_clip(
                painter,
                lane,
                viewport,
                ClipVisual {
                    range: clip.timeline,
                    label,
                    selected: state.selected_clips.contains(&clip.id),
                    track_enabled: track.enabled,
                    dragging: state.drag.as_ref().is_some_and(|d| d.clip == clip.id),
                    body: theme::AUDIO_CLIP,
                    top: theme::AUDIO_CLIP_TOP,
                    waveform: waveforms
                        .get(&clip.media_id)
                        .map(|w| (w.as_ref(), clip.source.start)),
                    filmstrip: None,
                    duration_of_media: MediaTime::ZERO,
                    keyframes: None,
                },
                clip.id,
                track.id,
                interaction,
            );
        }

        lanes.push(LaneLayout {
            track: track.id,
            kind: TrackKind::Audio,
            rect: lane,
        });
        y += theme::TRACK_HEIGHT + theme::TRACK_GAP;
    }

    lanes
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
            theme::TRACK_LANE
        } else {
            theme::TRACK_LANE_ALT
        },
    );
}

fn draw_track_header(painter: &egui::Painter, lane: Rect, name: &str, enabled: bool, locked: bool) {
    let header = Rect::from_min_size(lane.min, vec2(theme::TRACK_HEADER_WIDTH, lane.height()));
    painter.rect_filled(header, 0, theme::TRACK_HEADER);
    painter.line_segment(
        [header.right_top(), header.right_bottom()],
        Stroke::new(1.0, theme::GRID_LINE),
    );

    painter.text(
        Pos2::new(header.left() + 10.0, header.center().y - 7.0),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.0),
        if enabled {
            theme::CLIP_TEXT
        } else {
            theme::DISABLED
        },
    );

    let mut badges: Vec<&str> = Vec::new();
    if !enabled {
        badges.push("hidden");
    }
    if locked {
        badges.push("locked");
    }
    if !badges.is_empty() {
        painter.text(
            Pos2::new(header.left() + 10.0, header.center().y + 10.0),
            Align2::LEFT_CENTER,
            badges.join(" · "),
            FontId::proportional(10.0),
            theme::DISABLED,
        );
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
    /// Length of the whole source file, which is what the tiles span.
    duration_of_media: MediaTime,
    /// The clip's animation and where in the media it starts, drawn as marks
    /// along the bottom edge (§24). `None` for audio, which Milestone 8 does
    /// not animate.
    keyframes: Option<(&'a bettercut_editor_core::timeline::Keyframes, MediaTime)>,
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
    painter.rect_filled(rect, 2, theme::SELECTION.gamma_multiply(0.18));
    painter.rect_stroke(
        rect,
        2,
        Stroke::new(1.0, theme::SELECTION),
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
) {
    let media_ticks = media_duration.ticks();
    if tiles == 0 || media_ticks <= 0 || clip_rect.width() < 4.0 {
        return;
    }

    let per_tile = media_ticks / i64::from(tiles).max(1);
    if per_tile <= 0 {
        return;
    }

    // Tile width on screen: how much timeline one tile of source covers.
    let tile_px = (per_tile / viewport.ticks_per_pixel.max(1)) as f32;
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
        let into_clip = tile_source - source_start.ticks();
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

    let colour = theme::CLIP_TEXT.gamma_multiply(0.55);
    for [from, to] in segments {
        painter.line_segment([from, to], Stroke::new(1.0, colour));
    }
}

#[allow(clippy::too_many_arguments)]
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
    }

    let (body, top) = if visual.track_enabled {
        (visual.body, visual.top)
    } else {
        (theme::DISABLED, theme::DISABLED)
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

    if visual.selected {
        painter.rect_stroke(
            clip_rect,
            theme::CLIP_CORNER_RADIUS,
            Stroke::new(2.0, theme::SELECTION),
            StrokeKind::Inside,
        );
        draw_trim_handles(painter, clip_rect);
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

    // Only label a clip wide enough to read it; below that the text is noise.
    if clip_rect.width() > 46.0 {
        let text_painter = painter.with_clip_rect(clip_rect.shrink(4.0));
        text_painter.text(
            Pos2::new(clip_rect.left() + 7.0, clip_rect.center().y + 2.0),
            Align2::LEFT_CENTER,
            visual.label,
            FontId::proportional(12.0),
            theme::CLIP_TEXT,
        );
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

        painter.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(x, y - RADIUS),
                Pos2::new(x + RADIUS, y),
                Pos2::new(x, y + RADIUS),
                Pos2::new(x - RADIUS, y),
            ],
            theme::KEYFRAME,
            Stroke::new(1.0, theme::TIMELINE_BACKGROUND),
        ));
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
        painter.rect_filled(handle, theme::CLIP_CORNER_RADIUS, theme::SELECTION);
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

    if ghost.width() > 0.0 {
        // Red when the drop would be refused — a video clip over an audio
        // track. Better to show it before the release than to error after.
        let tint = if drag.target_invalid {
            theme::ERROR_TEXT
        } else {
            theme::SELECTION
        };
        painter.rect_filled(ghost, theme::CLIP_CORNER_RADIUS, tint.gamma_multiply(0.25));
        painter.rect_stroke(
            ghost,
            theme::CLIP_CORNER_RADIUS,
            Stroke::new(2.0, tint),
            StrokeKind::Inside,
        );
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
                Stroke::new(1.0, theme::SELECTION),
            );
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
        Stroke::new(1.5, theme::PLAYHEAD),
    );

    // Grab handle at the top, so the playhead reads as draggable.
    let handle = Rect::from_center_size(
        Pos2::new(x, rect.top() + theme::RULER_HEIGHT / 2.0),
        vec2(11.0, 11.0),
    );
    painter.rect_filled(handle, 2, theme::PLAYHEAD);
}
