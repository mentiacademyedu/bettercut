//! A/V synchronisation policy (§20a.5, §47a.4).
//!
//! Pure decision logic, deliberately separated from the code that decodes and
//! draws, because these are the rules that decide whether playback *feels*
//! right and they are impossible to test through a window.
//!
//! §20a.5:
//!
//! ```text
//! Video ahead of audio by  > 40 ms  -> hold frame
//! Video behind audio by    > 40 ms  -> drop frame(s)
//! Sustained drift          > 100 ms -> log a warning
//! ```
//!
//! §47a.4:
//!
//! ```text
//! Frame late by < 1 frame interval  -> present anyway
//! Frame late by 1-3 intervals       -> drop it, present the next
//! Sustained lateness                -> reduce preview quality (§17)
//! Buffer underrun                   -> hold last frame, never block audio
//! ```
//!
//! The two agree on the important thing: **audio never stops.** Video is what
//! gives way (§20a.5). A dropped frame is invisible; a gap in sound is not.

use bettercut_foundation::TimelineTime;

/// §20a.5's tolerance, in milliseconds.
pub const SYNC_TOLERANCE_MS: i64 = 40;
/// §20a.5's "something is wrong" threshold, in milliseconds.
pub const DRIFT_WARNING_MS: i64 = 100;

/// What to do with the frame currently in hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDecision {
    /// Show it. Either it is on time, or close enough that swapping it would
    /// look worse than the error (§47a.4: "late by < 1 frame -> present").
    Present,
    /// The frame is early; keep showing what is already on screen until the
    /// clock catches up.
    Hold,
    /// The frame is stale. Skip it and take the next one — §47a.4 is explicit
    /// that late frames are dropped, not queued, or playback falls further
    /// behind with every frame.
    Drop,
}

/// The full decision, including whether quality should back off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramePlan {
    pub decision: SyncDecision,
    /// How far the frame is from where the clock says it should be. Positive
    /// means the frame is ahead of the audio.
    pub drift: TimelineTime,
    /// §20a.5's sustained-drift warning threshold was exceeded.
    pub drift_is_alarming: bool,
    /// §47a.4/§17: playback is not keeping up and preview quality should drop.
    pub should_reduce_quality: bool,
}

/// Decide what to do with a decoded frame.
///
/// * `frame_time` — where the frame belongs on the timeline.
/// * `clock_time` — where the audio device says we are (§20a.1).
/// * `frame_interval` — one frame of the sequence, from §9's exact tick count.
/// * `consecutive_drops` — how many frames have been dropped in a row.
pub fn plan_frame(
    frame_time: TimelineTime,
    clock_time: TimelineTime,
    frame_interval: TimelineTime,
    consecutive_drops: u32,
) -> FramePlan {
    let drift = frame_time - clock_time;
    let drift_ms = ticks_to_millis(drift);

    let plan = |decision| FramePlan {
        decision,
        drift,
        drift_is_alarming: drift_ms.abs() > DRIFT_WARNING_MS,
        // Three drops in a row is not a hiccup, it is a trend (§17, §47a.4).
        should_reduce_quality: consecutive_drops >= 3,
    };

    // Within one frame interval, presenting is always right: swapping frames
    // to chase a sub-frame error produces visible judder for no gain.
    if drift.ticks().abs() <= frame_interval.ticks().max(1) {
        return plan(SyncDecision::Present);
    }

    if drift_ms > SYNC_TOLERANCE_MS {
        // Video is ahead: the frame is for a moment that has not arrived.
        return plan(SyncDecision::Hold);
    }

    if drift_ms < -SYNC_TOLERANCE_MS {
        // Video is behind: this frame is already stale.
        return plan(SyncDecision::Drop);
    }

    // Between one frame and 40 ms: close enough to show.
    plan(SyncDecision::Present)
}

/// Which frame the clock is asking for, snapped to the sequence's frame grid.
///
/// §9: the grid is exact, so this is integer division rather than a rounded
/// float, and it cannot drift however long playback runs.
pub fn frame_index_at(position: TimelineTime, frame_interval: TimelineTime) -> i64 {
    let interval = frame_interval.ticks().max(1);
    position.ticks().div_euclid(interval)
}

fn ticks_to_millis(t: TimelineTime) -> i64 {
    t.ticks() / (bettercut_foundation::TICKS_PER_SECOND / 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame at 30 fps (§9's exact table).
    const FRAME_30: i64 = 32_000;

    fn plan_at(frame_ms: i64, clock_ms: i64, drops: u32) -> FramePlan {
        plan_frame(
            TimelineTime::from_millis(frame_ms),
            TimelineTime::from_millis(clock_ms),
            TimelineTime::from_ticks(FRAME_30),
            drops,
        )
    }

    #[test]
    fn a_frame_on_time_is_presented() {
        assert_eq!(plan_at(1000, 1000, 0).decision, SyncDecision::Present);
    }

    /// §47a.4: "late by < 1 frame interval -> present anyway".
    #[test]
    fn a_frame_within_one_interval_is_presented_either_way() {
        // 33 ms is one frame at 30 fps.
        assert_eq!(plan_at(1000, 1020, 0).decision, SyncDecision::Present);
        assert_eq!(plan_at(1020, 1000, 0).decision, SyncDecision::Present);
    }

    /// §20a.5: video ahead by more than 40 ms holds.
    #[test]
    fn a_frame_well_ahead_of_the_clock_is_held() {
        assert_eq!(plan_at(1100, 1000, 0).decision, SyncDecision::Hold);
    }

    /// §20a.5: video behind by more than 40 ms drops.
    #[test]
    fn a_frame_well_behind_the_clock_is_dropped() {
        assert_eq!(plan_at(1000, 1100, 0).decision, SyncDecision::Drop);
    }

    /// The tolerance is symmetric, and the boundary belongs to "present".
    #[test]
    fn the_forty_millisecond_boundary_still_presents() {
        assert_eq!(plan_at(1040, 1000, 0).decision, SyncDecision::Present);
        assert_eq!(plan_at(1000, 1040, 0).decision, SyncDecision::Present);
        assert_eq!(plan_at(1041, 1000, 0).decision, SyncDecision::Hold);
        assert_eq!(plan_at(1000, 1041, 0).decision, SyncDecision::Drop);
    }

    #[test]
    fn drift_beyond_a_hundred_milliseconds_is_flagged() {
        assert!(!plan_at(1000, 1050, 0).drift_is_alarming);
        assert!(plan_at(1000, 1200, 0).drift_is_alarming);
        assert!(plan_at(1200, 1000, 0).drift_is_alarming);
    }

    /// §17/§47a.4: sustained lateness reduces preview quality. One dropped
    /// frame is a hiccup and must not trigger it.
    #[test]
    fn quality_drops_only_after_sustained_lateness() {
        assert!(!plan_at(1000, 1100, 0).should_reduce_quality);
        assert!(!plan_at(1000, 1100, 2).should_reduce_quality);
        assert!(plan_at(1000, 1100, 3).should_reduce_quality);
    }

    #[test]
    fn drift_is_reported_with_its_sign() {
        assert!(
            plan_at(1100, 1000, 0).drift.ticks() > 0,
            "ahead should be positive"
        );
        assert!(
            plan_at(1000, 1100, 0).drift.ticks() < 0,
            "behind should be negative"
        );
    }

    /// At a high frame rate the "within one interval" window is tighter, so a
    /// gap that is fine at 24 fps is a real error at 120.
    #[test]
    fn the_tolerance_follows_the_frame_rate() {
        let at_120 = plan_frame(
            TimelineTime::from_millis(1020),
            TimelineTime::from_millis(1000),
            TimelineTime::from_ticks(8_000), // 120 fps
            0,
        );
        // 20 ms is well over one 120 fps frame but still inside 40 ms.
        assert_eq!(at_120.decision, SyncDecision::Present);

        let far_at_120 = plan_frame(
            TimelineTime::from_millis(1100),
            TimelineTime::from_millis(1000),
            TimelineTime::from_ticks(8_000),
            0,
        );
        assert_eq!(far_at_120.decision, SyncDecision::Hold);
    }

    #[test]
    fn frame_index_is_exact_and_does_not_drift() {
        let interval = TimelineTime::from_ticks(FRAME_30);
        for index in [0_i64, 1, 30, 1_000, 1_000_000] {
            let position = TimelineTime::from_ticks(index * FRAME_30);
            assert_eq!(frame_index_at(position, interval), index);
        }
    }

    /// NTSC is the case §9 exists for; frame selection must be exact there too.
    #[test]
    fn frame_index_is_exact_at_ntsc_rates() {
        let interval = TimelineTime::from_ticks(32_032); // 29.97 fps
        for index in [0_i64, 1, 107_892] {
            let position = TimelineTime::from_ticks(index * 32_032);
            assert_eq!(frame_index_at(position, interval), index);
        }
    }

    #[test]
    fn a_position_inside_a_frame_belongs_to_that_frame() {
        let interval = TimelineTime::from_ticks(FRAME_30);
        assert_eq!(frame_index_at(TimelineTime::from_ticks(0), interval), 0);
        assert_eq!(
            frame_index_at(TimelineTime::from_ticks(31_999), interval),
            0
        );
        assert_eq!(
            frame_index_at(TimelineTime::from_ticks(32_000), interval),
            1
        );
    }
}
