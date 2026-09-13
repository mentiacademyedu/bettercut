//! J, K and L: shuttle playback, the way every editor since the tape deck
//! has done it.
//!
//! L plays forwards, and each further press doubles the speed up to eight
//! times. J does the same backwards. K stops. Holding K and tapping J or L
//! steps a single frame, for finding the exact frame to cut on by hand.
//!
//! Normal-speed forward play is ordinary playback, sound and all. Every other
//! speed — faster, or backwards — moves the playhead from the frame timer and
//! is silent: the audio clock (§20a.1) runs at the device's rate and nothing
//! else, and chipmunk sound at 4× tells nobody anything the picture does not.
//!
//! This module is the arithmetic, kept free of the renderer so it can be held
//! to its rules without a GPU; [`crate::Preview`] does the moving.

use bettercut_editor_core::foundation::TimelineTime;

/// The fastest shuttle, either way.
pub const MAX_RATE: i32 = 8;

/// Which of the three keys was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShuttleKey {
    /// J.
    Back,
    /// K.
    Stop,
    /// L.
    Forward,
}

/// The rate after a press, from the rate before it. Rates are whole multiples
/// of normal speed, negative backwards, zero stopped.
///
/// A press in the direction already playing doubles the speed; a press the
/// other way starts again at normal speed in that direction — reversing at 8×
/// is never what a tap on the other key means.
pub fn after_press(rate: i32, key: ShuttleKey) -> i32 {
    match key {
        ShuttleKey::Stop => 0,
        ShuttleKey::Forward if rate > 0 => (rate * 2).min(MAX_RATE),
        ShuttleKey::Forward => 1,
        ShuttleKey::Back if rate < 0 => (rate * 2).max(-MAX_RATE),
        ShuttleKey::Back => -1,
    }
}

/// Where the playhead is after `elapsed` at `rate`, held to `0..=end`, and
/// whether it reached either end — where a shuttle stops.
pub fn advance(
    position: TimelineTime,
    rate: i32,
    elapsed: std::time::Duration,
    end: TimelineTime,
) -> (TimelineTime, bool) {
    // Integer ticks from nanoseconds: 960,000 ticks a second (§9).
    let moved = elapsed.as_nanos() as i128
        * i128::from(rate)
        * i128::from(bettercut_editor_core::foundation::TICKS_PER_SECOND)
        / 1_000_000_000;
    let wanted = i128::from(position.ticks()) + moved;
    let end_ticks = i128::from(end.ticks().max(0));
    let held = wanted.clamp(0, end_ticks);
    let at_edge = rate != 0 && ((rate > 0 && held >= end_ticks) || (rate < 0 && held <= 0));
    (TimelineTime::from_ticks(held as i64), at_edge)
}

/// The status line for a rate.
pub fn describe(rate: i32) -> String {
    match rate {
        0 => "Paused".to_owned(),
        1 => "Playing".to_owned(),
        r if r > 1 => format!("Fast forward {r}×"),
        r => format!("Reverse {}×", -r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn l_doubles_up_to_the_limit_and_j_turns_round() {
        let mut rate = 0;
        let mut seen = Vec::new();
        for _ in 0..5 {
            rate = after_press(rate, ShuttleKey::Forward);
            seen.push(rate);
        }
        assert_eq!(seen, [1, 2, 4, 8, 8]);
        assert_eq!(
            after_press(8, ShuttleKey::Back),
            -1,
            "turning round starts slow"
        );
        assert_eq!(after_press(-1, ShuttleKey::Back), -2);
        assert_eq!(after_press(-8, ShuttleKey::Back), -8);
        assert_eq!(after_press(-4, ShuttleKey::Forward), 1);
        assert_eq!(after_press(-4, ShuttleKey::Stop), 0);
        assert_eq!(after_press(0, ShuttleKey::Back), -1);
    }

    #[test]
    fn the_playhead_moves_at_the_rate_and_stops_at_the_ends() {
        let s = TimelineTime::from_seconds;
        let end = s(10);
        assert_eq!(
            advance(s(2), 2, Duration::from_millis(500), end),
            (s(3), false)
        );
        assert_eq!(
            advance(s(2), -4, Duration::from_millis(250), end),
            (s(1), false)
        );
        assert_eq!(advance(s(9), 4, Duration::from_secs(1), end), (end, true));
        assert_eq!(
            advance(s(1), -2, Duration::from_secs(1), end),
            (TimelineTime::ZERO, true)
        );
        // Exact in ticks: a sixtieth of a second at 1× is 16,000 ticks.
        let (at, _) = advance(TimelineTime::ZERO, 1, Duration::from_nanos(16_666_667), end);
        assert_eq!(at.ticks(), 16_000);
    }

    #[test]
    fn the_status_names_the_direction_and_speed() {
        assert_eq!(describe(0), "Paused");
        assert_eq!(describe(1), "Playing");
        assert_eq!(describe(4), "Fast forward 4×");
        assert_eq!(describe(-2), "Reverse 2×");
    }
}
