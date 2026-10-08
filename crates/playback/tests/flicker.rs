//! Flicker (`flicker_alpha`): the picture dims at random moments, the same
//! moments every time.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::TimelineTime;
use bettercut_playback::engine::{FLICKER_HZ, flicker_alpha};

fn seconds(s: f64) -> TimelineTime {
    TimelineTime::from_ticks((s * bettercut_foundation::TICKS_PER_SECOND as f64) as i64)
}

fn slice(n: usize) -> TimelineTime {
    seconds((n as f64 + 0.5) / FLICKER_HZ)
}

#[test]
fn off_does_nothing() {
    assert_eq!(flicker_alpha(0.0, seconds(1.0)), 0.0);
    assert_eq!(flicker_alpha(f32::NAN, seconds(1.0)), 0.0);
}

/// Over a few seconds it dips hard now and then, is often near clear, and
/// never blacks the picture out.
#[test]
fn it_dips_now_and_then_and_never_blacks_out() {
    let levels: Vec<f32> = (0..120).map(|n| flicker_alpha(100.0, slice(n))).collect();
    assert!(levels.iter().all(|a| (0.0..=0.75).contains(a)));
    assert!(levels.iter().any(|a| *a > 0.3), "no hard dip in ten seconds");
    let faint = levels.iter().filter(|a| **a < 0.1).count();
    assert!(faint > levels.len() / 3, "too often dark: {faint} faint of 120");
}

/// The same moment dips the same way every time, and less with less.
#[test]
fn it_is_the_same_every_time_and_scales_with_amount() {
    for n in 0..30 {
        assert_eq!(flicker_alpha(80.0, slice(n)), flicker_alpha(80.0, slice(n)));
        assert!(flicker_alpha(30.0, slice(n)) <= flicker_alpha(100.0, slice(n)));
    }
}
