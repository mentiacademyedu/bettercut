//! Transitions at a cut (§25).
//!
//! ## Attached to the outgoing clip, not to the track
//!
//! A transition belongs to the clip whose end it decorates. That is the only
//! placement that survives editing: move, trim, split, cut and paste all carry
//! the clip, and the transition goes with it without a second collection to
//! keep in step. A track-level list keyed by boundary time would have to be
//! rewritten by every one of those operations — the same argument §24 makes for
//! anchoring keyframes to the source rather than to the timeline.
//!
//! ## The timeline still does not overlap
//!
//! §8's tracks are non-overlapping and sorted, and that invariant is what makes
//! the visible-range query a binary search (§53, §54). Transitions do **not**
//! relax it. Clips stay exactly where the user put them; a transition is a
//! window *centred on the cut* during which the renderer asks for frames from
//! both neighbours at once.
//!
//! Centred, because the cut is the thing the user placed. A window running
//! entirely after the cut would put the visible change half a transition later
//! than the frame they trimmed to.
//!
//! ## Handles, and why one kind needs them and the other does not
//!
//! A crossfade shows both clips at once, so during the first half of the window
//! the incoming clip must supply frames from *before* its in-point, and during
//! the second half the outgoing clip must supply frames from *after* its
//! out-point. That spare material is the handle, and a clip trimmed to the very
//! start or end of its media has none. [`Transition::max_duration`] works out
//! what is actually there, so the interface can only offer a length that exists.
//!
//! A fade through black has no such problem: the outgoing clip fades out over
//! its own last frames and the incoming fades in over its own first ones.
//! Nothing is ever read outside a clip's own range, so it works on any cut —
//! including one against the very start or end of a file.

use bettercut_foundation::{MediaTime, TimelineTime};
use serde::{Deserialize, Serialize};

use crate::TimelineRange;

/// What happens at the cut (§25's initial set, started).
///
/// Two, deliberately. §25: *"Avoid implementing dozens before the engine is
/// stable."* These cover most cuts, and between them they exercise both shapes
/// the engine has to support — one that needs frames from two clips at once,
/// and one that does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// Both clips visible at once, the incoming one fading up over the
    /// outgoing. Needs handles on both sides.
    #[default]
    Crossfade,
    /// The outgoing clip fades out to black, then the incoming fades in from
    /// it. Needs no handles, and works at the very start or end of a file.
    FadeThroughBlack,
}

impl TransitionKind {
    pub const ALL: [Self; 2] = [Self::Crossfade, Self::FadeThroughBlack];

    pub fn label(self) -> &'static str {
        match self {
            Self::Crossfade => "Crossfade",
            Self::FadeThroughBlack => "Fade through black",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Crossfade => {
                "The next shot fades up over this one. Needs spare footage \
                 either side of the cut."
            }
            Self::FadeThroughBlack => {
                "This shot fades out, the next fades in. Works anywhere, \
                 including against the start or end of a file."
            }
        }
    }

    /// Whether this reads material outside the clips' own ranges.
    pub fn needs_handles(self) -> bool {
        matches!(self, Self::Crossfade)
    }
}

/// Shortest transition worth having.
///
/// Below about a tenth of a second a dissolve reads as a glitch rather than a
/// transition, and it costs a second decoder for a handful of frames.
pub const MIN_DURATION: TimelineTime = TimelineTime::from_ticks(96_000);

/// What a new transition starts at: half a second.
///
/// Short enough to read as a cut with a soft edge rather than a dissolve the
/// user has to sit through, and it is the length most editors default to.
/// Clamped down when the handles cannot reach it.
pub const DEFAULT_DURATION: TimelineTime = TimelineTime::from_ticks(480_000);

/// A transition at the end of one clip (§25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Total length, centred on the cut. Half falls either side.
    pub duration: TimelineTime,
}

impl Transition {
    pub fn new(kind: TransitionKind, duration: TimelineTime) -> Self {
        Self { kind, duration }
    }

    /// Half the duration in ticks — how far the window reaches each side.
    ///
    /// Integer division, and deliberately not compensated: an odd tick count
    /// loses one tick off the total rather than pushing the midpoint off the
    /// cut. §9 forbids floating point here, and a transition one tick short is
    /// invisible where a cut one tick out is not.
    pub fn half_ticks(self) -> i64 {
        self.duration.ticks() / 2
    }

    /// The span the transition covers, centred on `cut`.
    pub fn window(self, cut: TimelineTime) -> TimelineRange {
        let half = self.half_ticks();
        TimelineRange {
            start: TimelineTime::from_ticks((cut.ticks() - half).max(0)),
            end: TimelineTime::from_ticks(cut.ticks() + half),
        }
    }

    /// How far through the transition `at` is: 0 at the window's start, 1 at
    /// its end.
    ///
    /// Outside the window this saturates rather than extrapolating. A caller
    /// asking about the wrong instant gets a sane answer instead of a blend
    /// factor beyond either clip, which the compositor would clamp anyway —
    /// silently, and a long way from the cause.
    pub fn progress(self, cut: TimelineTime, at: TimelineTime) -> f32 {
        let half = self.half_ticks();
        if half <= 0 {
            return 1.0;
        }
        let from_start = at.ticks() - (cut.ticks() - half);
        // A ratio between two positions that were subtracted as integers: the
        // one place §74 allows a fraction, because it is the answer being asked
        // for rather than a position being stored.
        (from_start as f64 / (half * 2) as f64).clamp(0.0, 1.0) as f32
    }

    /// The longest centred transition two clips can actually support.
    ///
    /// `handle_after` is how much source the outgoing clip has past its
    /// out-point; `handle_before` is how much the incoming clip has ahead of
    /// its in-point. Each half of the window eats one of them, so the total is
    /// twice the smaller.
    ///
    /// Bounded by the shots as well: a transition may not outlast either, or it
    /// would still be running when the next cut arrived.
    pub fn max_duration(
        kind: TransitionKind,
        outgoing_length: TimelineTime,
        incoming_length: TimelineTime,
        handle_after: MediaTime,
        handle_before: MediaTime,
    ) -> TimelineTime {
        let by_length = outgoing_length.ticks().min(incoming_length.ticks());
        let limit = if kind.needs_handles() {
            let handles = handle_after.ticks().min(handle_before.ticks());
            by_length.min(handles.saturating_mul(2))
        } else {
            by_length
        };
        TimelineTime::from_ticks(limit.max(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks(n: i64) -> TimelineTime {
        TimelineTime::from_ticks(n)
    }

    /// The window straddles the cut evenly. The cut is where the user put it,
    /// and a window running only after it would move the visible change.
    #[test]
    fn the_window_is_centred_on_the_cut() {
        let t = Transition::new(TransitionKind::Crossfade, ticks(1000));
        let window = t.window(ticks(5000));
        assert_eq!(window.start.ticks(), 4500);
        assert_eq!(window.end.ticks(), 5500);
    }

    /// A cut near zero cannot reach behind the start of the timeline.
    #[test]
    fn a_window_at_the_start_is_clamped() {
        let t = Transition::new(TransitionKind::Crossfade, ticks(1000));
        let window = t.window(ticks(200));
        assert_eq!(window.start.ticks(), 0);
        assert_eq!(window.end.ticks(), 700);
    }

    #[test]
    fn progress_runs_from_zero_to_one_across_the_window() {
        let t = Transition::new(TransitionKind::Crossfade, ticks(1000));
        let cut = ticks(5000);

        assert_eq!(t.progress(cut, ticks(4500)), 0.0);
        assert_eq!(t.progress(cut, ticks(5500)), 1.0);
        let middle = t.progress(cut, cut);
        assert!((middle - 0.5).abs() < 1e-6, "at the cut it was {middle}");
    }

    #[test]
    fn progress_outside_the_window_saturates() {
        let t = Transition::new(TransitionKind::Crossfade, ticks(1000));
        let cut = ticks(5000);
        assert_eq!(t.progress(cut, ticks(0)), 0.0);
        assert_eq!(t.progress(cut, ticks(99_999)), 1.0);
    }

    /// A zero-length transition is degenerate rather than a division by zero.
    #[test]
    fn a_zero_length_transition_is_already_finished() {
        let t = Transition::new(TransitionKind::Crossfade, ticks(0));
        assert_eq!(t.progress(ticks(5000), ticks(5000)), 1.0);
        let window = t.window(ticks(5000));
        assert_eq!(window.start, window.end);
    }

    /// A crossfade is limited by whichever side has less spare footage, and
    /// each half of the window consumes one handle — so the total is twice the
    /// smaller.
    #[test]
    fn a_crossfade_is_limited_by_the_smaller_handle() {
        let max = Transition::max_duration(
            TransitionKind::Crossfade,
            ticks(100_000),
            ticks(100_000),
            MediaTime::from_ticks(3_000),
            MediaTime::from_ticks(500),
        );
        assert_eq!(max.ticks(), 1_000, "twice the smaller handle");
    }

    /// With no spare footage a crossfade is not possible at all, and saying so
    /// beats offering a length that produces a black flash.
    #[test]
    fn a_crossfade_with_no_handles_is_impossible() {
        let max = Transition::max_duration(
            TransitionKind::Crossfade,
            ticks(100_000),
            ticks(100_000),
            MediaTime::ZERO,
            MediaTime::from_ticks(9_000),
        );
        assert_eq!(max.ticks(), 0);
    }

    /// A fade through black reads nothing outside either clip, so handles do
    /// not constrain it — only the length of the shots.
    #[test]
    fn a_fade_through_black_ignores_handles() {
        let max = Transition::max_duration(
            TransitionKind::FadeThroughBlack,
            ticks(60_000),
            ticks(100_000),
            MediaTime::ZERO,
            MediaTime::ZERO,
        );
        assert_eq!(max.ticks(), 60_000, "bounded by the shorter clip only");
    }

    /// Neither kind may outlast the shots it joins.
    #[test]
    fn a_transition_cannot_outlast_either_shot() {
        let max = Transition::max_duration(
            TransitionKind::Crossfade,
            ticks(4_000),
            ticks(100_000),
            MediaTime::from_ticks(500_000),
            MediaTime::from_ticks(500_000),
        );
        assert_eq!(max.ticks(), 4_000);
    }
}
