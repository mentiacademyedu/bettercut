//! Strobe (`strobe_alpha`): white flashes in a steady rhythm, with the
//! picture seen between them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::TimelineTime;
use bettercut_playback::engine::{STROBE_HZ, strobe_alpha};

fn seconds(s: f64) -> TimelineTime {
    TimelineTime::from_ticks((s * bettercut_foundation::TICKS_PER_SECOND as f64) as i64)
}

#[test]
fn off_does_nothing() {
    assert_eq!(strobe_alpha(0.0, seconds(1.0)), 0.0);
    assert_eq!(strobe_alpha(f32::NAN, seconds(1.0)), 0.0);
}

/// Brightest at the start of each beat, clear of it by the middle, and the
/// same on every beat.
#[test]
fn it_flashes_on_each_beat_and_clears_between() {
    let beat = 1.0 / STROBE_HZ;
    for n in 0..8 {
        let start = n as f64 * beat;
        let on = strobe_alpha(100.0, seconds(start));
        assert!(on > 0.8, "beat {n}: {on}");
        assert_eq!(strobe_alpha(100.0, seconds(start + beat * 0.5)), 0.0);
    }
    assert!(strobe_alpha(30.0, seconds(0.0)) < strobe_alpha(100.0, seconds(0.0)));
}
