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
    pub show_diagnostics: bool,

    /// §10 "Snapping". On by default; hold Alt during a drag to bypass it,
    /// which is the convention every editor uses and the fastest way to place
    /// something deliberately off-grid.
    pub snapping: bool,

    /// The drag in progress, if any.
    pub drag: Option<DragState>,

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

    /// Proxies being generated: how many, and how far along (§42).
    ///
    /// Mirrored here rather than reached for through the `ProxyManager`,
    /// because §42 wants background work *visible* and the status bar should
    /// not have to know what a job scheduler is.
    pub proxy_progress: Option<(usize, f32)>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            zoom_index: DEFAULT_ZOOM_INDEX,
            scroll_ticks: 0,
            selected_clips: HashSet::new(),
            selected_track: None,
            status: None,
            show_diagnostics: false,
            snapping: true,
            drag: None,
            pending_recovery: None,
            needs_repaint: true,
            context: None,
            proxy_progress: None,
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
