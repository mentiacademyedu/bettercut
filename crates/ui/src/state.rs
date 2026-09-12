//! View state. None of this is project data and none of it is saved.

use std::collections::HashSet;

use bettercut_editor_core::foundation::{ClipId, TimelineTime, TrackId};
use bettercut_editor_core::timeline::{SnapTarget, TimelineRange};

/// What a drag on a clip is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
}

/// A drag in progress.
///
/// The edit is **not** dispatched until the mouse is released. Dispatching per
/// frame would push sixty commands onto the undo stack for one gesture, so a
/// single drag has to undo in one step — which means holding a preview here and
/// committing once (§11).
#[derive(Debug, Clone)]
pub struct DragState {
    pub clip: ClipId,
    pub source_track: TrackId,
    pub mode: DragMode,
    /// Ticks between the clip's start and where the pointer grabbed it, so the
    /// clip does not jump to centre itself on the cursor.
    pub grab_offset: i64,
    /// Where the clip was before the drag, for the preview and for cancelling.
    pub original: TimelineRange,
    /// Live preview position.
    pub preview: TimelineRange,
    /// Set once the pointer actually moves.
    ///
    /// The drag is captured on *press*, not on egui's `drag_started`, because
    /// by the time a drag is recognised the pointer may already have left the
    /// clip — which is exactly what happens with a fast mouse on a short clip.
    /// A press that never moves is a click, and this flag tells them apart.
    pub moved: bool,
    /// Track under the pointer right now — may differ from `source_track`.
    pub target_track: TrackId,
    /// True when the target track cannot accept this clip (wrong kind).
    pub target_invalid: bool,
    /// What the preview snapped to, if anything. Drawn as a guide line.
    pub snapped_to: Option<SnapTarget>,
    /// §12's linked partners — a video's sound, or a sound's picture — each
    /// with its track and where it was when the drag began.
    ///
    /// Captured at the press, because the editor moves them on release and the
    /// drag's own preview is the only thing that can show where they are going
    /// in the meantime. Without it the sound jumps when the mouse is let go,
    /// which reads as a glitch rather than as a link.
    pub partners: Vec<(TrackId, TimelineRange)>,
}

impl DragState {
    /// Where a partner will land, given where the dragged clip is going.
    ///
    /// By the same *delta*, exactly as the editor applies it (§12), so the
    /// ghost is where the clip will actually end up rather than an
    /// approximation of it.
    pub fn partner_preview(&self, partner: TimelineRange) -> TimelineRange {
        let shift = |t: TimelineTime, by: i64| TimelineTime::from_ticks(t.ticks() + by);
        match self.mode {
            DragMode::Move => {
                let by = self.preview.start.ticks() - self.original.start.ticks();
                TimelineRange {
                    start: shift(partner.start, by),
                    end: shift(partner.end, by),
                }
            }
            DragMode::TrimStart => TimelineRange {
                start: shift(
                    partner.start,
                    self.preview.start.ticks() - self.original.start.ticks(),
                ),
                end: partner.end,
            },
            DragMode::TrimEnd => TimelineRange {
                start: partner.start,
                end: shift(
                    partner.end,
                    self.preview.end.ticks() - self.original.end.ticks(),
                ),
            },
        }
    }
}

/// Zoom ladder, in ticks per pixel.
///
/// **Integers, deliberately.** §74 forbids floating point in timeline position
/// arithmetic, and the pixel↔tick mapping is exactly that. With an integer
/// ladder, `tick_of_x(x_of_tick(t)) == t` up to one pixel, always, with no
/// accumulated error as the user zooms in and out repeatedly.
///
/// At 960,000 ticks/second, 32,000 ticks/px is 30 pixels per second.
pub const ZOOM_LEVELS: [i64; 16] = [
    500, 1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 64_000, 128_000, 256_000, 512_000, 960_000,
    1_920_000, 3_840_000, 7_680_000, 15_360_000,
];

/// Index into [`ZOOM_LEVELS`]: 32,000 ticks/px = 30 px/second.
pub const DEFAULT_ZOOM_INDEX: usize = 6;

/// A transient message in the status bar.
#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub is_error: bool,
}

/// What playback is actually doing, for the System panel (§49, §52).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackStats {
    pub playing: bool,
    /// §47a.4: frames arriving too late to show. Non-zero while playing means
    /// the machine is not keeping up.
    pub dropped_frames: u64,
    /// §20a: the audio device ran out of samples. Audible, and the one number
    /// that must stay at zero.
    pub underruns: u32,
    /// §20a.4: samples the limiter had to clamp — the mix is too hot.
    pub limited_samples: u64,
    /// §20a: the last block's peak level per side, 0.0 to 1.0 or beyond.
    pub peaks: (f32, f32),
    /// §47a.3: frames served from the decode-ahead ring rather than decoded
    /// inline. Zero while playing means decode-ahead is not helping.
    pub prefetch_hits: u64,
    /// Frames currently waiting in that ring.
    pub ring_frames: usize,
    /// §16's preview scale in force right now.
    pub quality: &'static str,
}

/// Dragging one point of a sound clip's volume envelope (§24).
///
/// The whole envelope is carried, as it was when the drag started, because the
/// envelope is written as one command: each frame moves one point in this copy
/// and sends the lot. Carrying the *original* also means the drag is built
/// against the shape before it began, which is what lets the frames coalesce
/// into one undo step.
#[derive(Debug, Clone)]
pub struct EnvelopeDrag {
    pub clip: ClipId,
    /// Which point is being moved.
    pub index: usize,
    /// The envelope as it is now, in timeline instants and levels.
    pub points: Vec<(bettercut_editor_core::foundation::TimelineTime, f32)>,
    /// The clip's rect when the drag began, which is what turns the pointer's
    /// height into a level. Captured once: after the first frame the pointer
    /// has usually left the dot, and the draw pass then reports no hit to read
    /// a rect from.
    pub rect: egui::Rect,
    /// Set once the pointer moves, so the first frame starts the undo step and
    /// the rest join it. A press that never moves is not an edit at all.
    pub moved: bool,
}

/// A rubber-band selection in progress.
///
/// Held as screen positions rather than a time range because it is drawn as a
/// rectangle: converting to time happens once, on release, when it is used.
#[derive(Debug, Clone, Copy)]
pub struct Marquee {
    pub origin: egui::Pos2,
    pub current: egui::Pos2,
    /// Ctrl was held when the drag started: add to the selection rather than
    /// replacing it, matching Ctrl+click.
    pub additive: bool,
    /// Set once the pointer actually moves. A press that never moves is a
    /// click, and must still move the playhead rather than clearing it.
    pub moved: bool,
}

impl Marquee {
    pub fn rect(&self) -> egui::Rect {
        egui::Rect::from_two_pos(self.origin, self.current)
    }
}

/// Where a right-click landed on the timeline.
///
/// Three cases, because the useful actions differ completely: a clip, a track's
/// header, or empty canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextTarget {
    Clip {
        clip: ClipId,
        track: TrackId,
    },
    TrackHeader {
        track: TrackId,
    },
    Empty {
        at: bettercut_editor_core::foundation::TimelineTime,
    },
}

#[derive(Debug)]
pub struct UiState {
    zoom_index: usize,
    /// Timeline tick at the left edge of the canvas.
    pub scroll_ticks: i64,

    pub selected_clips: HashSet<ClipId>,
    pub selected_track: Option<TrackId>,

    pub status: Option<StatusMessage>,

    /// Font families offered by §26's family picker.
    ///
    /// Read once from the rasterizer, because it is a fact about the machine
    /// rather than about the project and enumerating font directories is not
    /// something to do while drawing a frame. Empty until the preview exists,
    /// which leaves the picker with the three generic names — still usable,
    /// and it fills in on the next launch.
    pub font_families: Vec<String>,

    /// What has been typed into the family picker's filter.
    ///
    /// A machine can have three hundred families and the one you want is
    /// rarely near the top.
    pub font_filter: String,

    /// Live playback counters, mirrored each frame while a preview exists.
    ///
    /// §52's benchmarks and §81's targets are numbers, and the first report
    /// from real hardware came back as "it felt smooth" — because nothing put
    /// these on screen. They are counted either way; showing them costs a few
    /// lines and turns an adjective into evidence.
    pub playback: Option<PlaybackStats>,

    /// §10 "Snapping". On by default; hold Alt during a drag to bypass it,
    /// which is the convention every editor uses and the fastest way to place
    /// something deliberately off-grid.
    pub snapping: bool,

    /// The drag in progress, if any.
    pub drag: Option<DragState>,

    /// Which page of the Inspector's clip section is showing.
    pub inspector_tab: crate::panels::InspectorTab,

    /// A move or scale being dragged out on the preview.
    pub preview_drag: Option<crate::preview_overlay::PreviewDrag>,

    /// The composited picture no longer matches the project.
    ///
    /// The preview re-renders when the playhead moves, which is the common
    /// case and cheap to detect. It cannot see an edit that changes how a clip
    /// looks without moving anything — a slider, a drag on the picture, a
    /// keyframe, a hidden track — so the event stream says so and the shell
    /// clears it.
    pub preview_is_stale: bool,

    /// A rubber-band selection being dragged out (§10 "Multi-select clips").
    pub marquee: Option<Marquee>,

    /// The Export window's state (Milestone 6). Kept here rather than in the
    /// panel so it survives between frames and so the shell, which owns the
    /// job scheduler, can read what the user chose.
    pub export_dialog: crate::export_dialog::ExportDialog,

    /// The Templates window (§31). Here so a half-filled template survives
    /// closing the window to go and import the clip it was missing.
    pub template_dialog: crate::template_dialog::TemplateDialog,

    /// The Shortcuts window (`?` or F1).
    pub shortcuts_open: bool,

    /// A volume point being dragged on the timeline (§24).
    pub envelope_drag: Option<EnvelopeDrag>,

    /// The Captions window: the list of captions, editable in place.
    pub captions_open: bool,

    /// Which caption is being typed into, so the keystrokes after the first
    /// join the same undo step and a different caption starts its own.
    pub caption_typing: Option<ClipId>,

    /// The Remove Silences window (§78), with its suggestion.
    pub silence: Option<crate::silence_dialog::SilenceDialog>,

    /// The Find Cuts window (§45), once detection has been started.
    pub scenes: Option<crate::scene_dialog::SceneDialog>,

    /// A clip the user asked to have its cuts found. Picked up by the shell,
    /// because starting a job needs the scheduler and the scheduler is not the
    /// interface's to hold — the same route the Export window takes.
    pub scene_request: Option<ClipId>,

    /// A detection to stop, for the same reason.
    pub scene_cancel: Option<bettercut_jobs::JobId>,

    /// Work found from a session that did not shut down cleanly (§39).
    ///
    /// Held rather than applied: §39.5 says never overwrite the original
    /// project automatically, so the user decides.
    pub pending_recovery: Option<bettercut_editor_core::RecoverableSession>,

    /// Set by any interaction that changed something, so the app knows to
    /// repaint rather than repainting unconditionally (§54, §81).
    pub needs_repaint: bool,

    /// What the last right-click landed on (§58).
    ///
    /// Captured at click time and held while the menu is open: the pointer
    /// moves onto the menu itself immediately, so re-hit-testing when an item
    /// is chosen would act on whatever happens to be under the cursor then.
    pub context: Option<ContextTarget>,

    /// Poster thumbnails, uploaded on demand (§12).
    pub thumbnails: crate::thumbnails::ThumbnailStore,

    /// Audio peaks for drawing waveforms on audio clips (§12, §53).
    pub waveforms: crate::waveforms::WaveformStore,

    /// The graphics adapter in use (§49), set once at startup.
    ///
    /// `None` when wgpu failed entirely and there is no preview (§50).
    pub gpu: Option<bettercut_renderer::GpuDescription>,

    /// Proxies being generated: how many, and how far along (§42).
    ///
    /// Mirrored here rather than reached for through the `ProxyManager`,
    /// because §42 wants background work *visible* and the status bar should
    /// not have to know what a job scheduler is.
    pub proxy_progress: Option<(usize, f32)>,

    /// The running export and its progress, plus whether the user pressed
    /// stop. The shell owns the scheduler, so the button sets a flag here and
    /// the shell acts on it.
    pub export_progress: Option<f32>,
    pub export_stop_requested: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            zoom_index: DEFAULT_ZOOM_INDEX,
            scroll_ticks: 0,
            selected_clips: HashSet::new(),
            selected_track: None,
            status: None,
            font_families: Vec::new(),
            font_filter: String::new(),
            playback: None,
            snapping: true,
            drag: None,
            inspector_tab: crate::panels::InspectorTab::default(),
            preview_drag: None,
            preview_is_stale: false,
            marquee: None,
            export_dialog: crate::export_dialog::ExportDialog::default(),
            template_dialog: crate::template_dialog::TemplateDialog::default(),
            shortcuts_open: false,
            envelope_drag: None,
            captions_open: false,
            caption_typing: None,
            silence: None,
            scenes: None,
            scene_request: None,
            scene_cancel: None,
            pending_recovery: None,
            needs_repaint: true,
            context: None,
            thumbnails: crate::thumbnails::ThumbnailStore::default(),
            waveforms: crate::waveforms::WaveformStore::default(),
            gpu: None,
            proxy_progress: None,
            export_progress: None,
            export_stop_requested: false,
        }
    }
}

impl UiState {
    pub fn ticks_per_pixel(&self) -> i64 {
        ZOOM_LEVELS[self.zoom_index.min(ZOOM_LEVELS.len() - 1)]
    }

    pub fn zoom_in(&mut self) {
        if self.zoom_index > 0 {
            self.zoom_index -= 1;
            self.needs_repaint = true;
        }
    }

    pub fn zoom_out(&mut self) {
        if self.zoom_index + 1 < ZOOM_LEVELS.len() {
            self.zoom_index += 1;
            self.needs_repaint = true;
        }
    }

    pub fn can_zoom_in(&self) -> bool {
        self.zoom_index > 0
    }

    pub fn can_zoom_out(&self) -> bool {
        self.zoom_index + 1 < ZOOM_LEVELS.len()
    }

    /// Pixels per second at the current zoom, for the ruler's tick spacing.
    pub fn pixels_per_second(&self) -> f32 {
        bettercut_editor_core::foundation::TICKS_PER_SECOND as f32 / self.ticks_per_pixel() as f32
    }

    /// Zoom so that `duration` fits in `width` pixels, picking the nearest
    /// ladder step that is not too tight.
    pub fn zoom_to_fit(&mut self, duration: TimelineTime, width: f32) {
        if duration.is_zero() || width <= 1.0 {
            return;
        }
        let wanted = (duration.ticks() as f32 / width).ceil() as i64;
        self.zoom_index = ZOOM_LEVELS
            .iter()
            .position(|&level| level >= wanted)
            .unwrap_or(ZOOM_LEVELS.len() - 1);
        self.scroll_ticks = 0;
        self.needs_repaint = true;
    }

    /// Horizontal scroll, in pixels. Clamped at the start of the timeline.
    pub fn scroll_by_pixels(&mut self, pixels: f32) {
        let delta = (pixels as i64).saturating_mul(self.ticks_per_pixel());
        let next = self.scroll_ticks.saturating_add(delta).max(0);
        if next != self.scroll_ticks {
            self.scroll_ticks = next;
            self.needs_repaint = true;
        }
    }

    /// Scroll so `t` is visible, with a margin, if it currently is not.
    pub fn scroll_to_reveal(&mut self, t: TimelineTime, width_pixels: f32) {
        let span = (width_pixels as i64).saturating_mul(self.ticks_per_pixel());
        let margin = span / 8;
        let left = self.scroll_ticks;
        let right = self.scroll_ticks.saturating_add(span);

        if t.ticks() < left {
            self.scroll_ticks = (t.ticks() - margin).max(0);
            self.needs_repaint = true;
        } else if t.ticks() > right {
            self.scroll_ticks = (t.ticks() - span + margin).max(0);
            self.needs_repaint = true;
        }
    }

    /// Keep the playhead on screen while playing, a page at a time.
    ///
    /// Not the same as [`Self::scroll_to_reveal`], which brings a point just
    /// inside the edge it went past. Doing that sixty times a second would
    /// scroll the timeline by a pixel per frame with the playhead pinned to the
    /// right edge — the picture moving under a stationary line, which is
    /// horrible to watch and impossible to read. This pages instead: when the
    /// playhead reaches the last eighth of the view, the view jumps forward so
    /// it starts again near the left.
    pub fn follow_playhead(&mut self, t: TimelineTime, width_pixels: f32) {
        let span = (width_pixels as i64).saturating_mul(self.ticks_per_pixel());
        if span <= 0 {
            return;
        }
        let margin = span / 8;
        let left = self.scroll_ticks;
        let trigger = left.saturating_add(span - margin);

        if t.ticks() < left || t.ticks() >= trigger {
            self.scroll_ticks = (t.ticks() - margin).max(0);
            self.needs_repaint = true;
        }
    }

    pub fn select_only(&mut self, clip: ClipId) {
        self.selected_clips.clear();
        self.selected_clips.insert(clip);
        self.needs_repaint = true;
    }

    pub fn toggle_selection(&mut self, clip: ClipId) {
        if !self.selected_clips.remove(&clip) {
            self.selected_clips.insert(clip);
        }
        self.needs_repaint = true;
    }

    pub fn clear_selection(&mut self) {
        if !self.selected_clips.is_empty() {
            self.selected_clips.clear();
            self.needs_repaint = true;
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage {
            text: text.into(),
            is_error: false,
        });
        self.needs_repaint = true;
    }

    /// Snap tolerance in ticks, from a fixed pixel distance.
    ///
    /// Constant in *pixels* so snapping grabs from the same visual distance
    /// whether zoomed to frames or to minutes. Integer throughout (§74).
    pub fn snap_tolerance(&self) -> TimelineTime {
        const GRAB_PIXELS: i64 = 8;
        TimelineTime::from_ticks(GRAB_PIXELS * self.ticks_per_pixel())
    }

    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::warn!(%text, "ui error");
        self.status = Some(StatusMessage {
            text,
            is_error: true,
        });
        self.needs_repaint = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: i64, end: i64) -> TimelineRange {
        TimelineRange {
            start: TimelineTime::from_seconds(start),
            end: TimelineTime::from_seconds(end),
        }
    }

    fn drag(mode: DragMode, original: TimelineRange, preview: TimelineRange) -> DragState {
        DragState {
            clip: ClipId::new(),
            source_track: TrackId::new(),
            mode,
            grab_offset: 0,
            original,
            preview,
            moved: true,
            target_track: TrackId::new(),
            target_invalid: false,
            snapped_to: None,
            partners: Vec::new(),
        }
    }

    /// §12: the partner's ghost has to land where the editor will put it, by
    /// the same delta. A ghost somewhere else would be worse than none.
    #[test]
    fn a_moved_partner_lands_by_the_same_delta() {
        let d = drag(DragMode::Move, range(0, 10), range(4, 14));
        assert_eq!(d.partner_preview(range(0, 10)), range(4, 14));
        // A partner that had drifted keeps its offset, as the editor does.
        assert_eq!(d.partner_preview(range(1, 9)), range(5, 13));
    }

    #[test]
    fn a_trimmed_partner_moves_only_the_same_edge() {
        let start = drag(DragMode::TrimStart, range(0, 10), range(3, 10));
        assert_eq!(start.partner_preview(range(0, 10)), range(3, 10));

        let end = drag(DragMode::TrimEnd, range(0, 10), range(0, 6));
        assert_eq!(end.partner_preview(range(0, 10)), range(0, 6));
    }

    #[test]
    fn default_zoom_is_thirty_pixels_per_second() {
        let s = UiState::default();
        assert_eq!(s.ticks_per_pixel(), 32_000);
        assert!((s.pixels_per_second() - 30.0).abs() < 0.001);
    }

    #[test]
    fn zoom_is_clamped_at_both_ends() {
        let mut s = UiState::default();
        for _ in 0..50 {
            s.zoom_in();
        }
        assert_eq!(s.ticks_per_pixel(), ZOOM_LEVELS[0]);
        assert!(!s.can_zoom_in());

        for _ in 0..50 {
            s.zoom_out();
        }
        assert_eq!(s.ticks_per_pixel(), ZOOM_LEVELS[ZOOM_LEVELS.len() - 1]);
        assert!(!s.can_zoom_out());
    }

    #[test]
    fn scrolling_never_goes_before_zero() {
        let mut s = UiState::default();
        s.scroll_by_pixels(-1000.0);
        assert_eq!(s.scroll_ticks, 0);
    }

    #[test]
    fn zoom_to_fit_picks_a_level_that_shows_the_whole_duration() {
        let mut s = UiState::default();
        let ten_minutes = TimelineTime::from_seconds(600);
        s.zoom_to_fit(ten_minutes, 1000.0);

        let visible = 1000_i64 * s.ticks_per_pixel();
        assert!(
            visible >= ten_minutes.ticks(),
            "zoom {} shows only {visible} of {} ticks",
            s.ticks_per_pixel(),
            ten_minutes.ticks()
        );
    }

    #[test]
    fn zoom_to_fit_ignores_an_empty_timeline() {
        let mut s = UiState::default();
        let before = s.ticks_per_pixel();
        s.zoom_to_fit(TimelineTime::ZERO, 1000.0);
        assert_eq!(s.ticks_per_pixel(), before);
    }

    #[test]
    fn revealing_scrolls_only_when_offscreen() {
        let mut s = UiState::default();
        // 1000 px at 32,000 ticks/px = 32,000,000 ticks visible.
        s.scroll_to_reveal(TimelineTime::from_ticks(1_000_000), 1000.0);
        assert_eq!(s.scroll_ticks, 0, "already visible");

        s.scroll_to_reveal(TimelineTime::from_ticks(100_000_000), 1000.0);
        assert!(s.scroll_ticks > 0);
    }

    /// While playing, the view pages forward rather than creeping a pixel at a
    /// time with the playhead stuck to the right edge.
    #[test]
    fn following_the_playhead_pages_the_view() {
        let mut s = UiState::default();
        let span = 1000 * s.ticks_per_pixel();
        let margin = span / 8;

        // Inside the view and not near the edge: nothing moves.
        s.follow_playhead(TimelineTime::from_ticks(span / 2), 1000.0);
        assert_eq!(s.scroll_ticks, 0);

        // Reaching the last eighth pages forward, leaving the playhead near
        // the left edge.
        let at = span - margin;
        s.follow_playhead(TimelineTime::from_ticks(at), 1000.0);
        assert_eq!(s.scroll_ticks, at - margin);

        // And it does not page again immediately.
        let scrolled = s.scroll_ticks;
        s.follow_playhead(TimelineTime::from_ticks(at + 10), 1000.0);
        assert_eq!(s.scroll_ticks, scrolled);

        // Jumping backwards — the playhead moved behind the view — brings it
        // back into sight.
        s.follow_playhead(TimelineTime::ZERO, 1000.0);
        assert_eq!(s.scroll_ticks, 0);
    }

    #[test]
    fn selection_toggles() {
        let mut s = UiState::default();
        let a = ClipId::new();
        s.select_only(a);
        assert!(s.selected_clips.contains(&a));
        s.toggle_selection(a);
        assert!(s.selected_clips.is_empty());
    }
}
