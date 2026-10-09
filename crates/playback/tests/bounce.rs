//! Bounce (`bounce`): the picture hops up and lands, enlarged just enough
//! that its bottom edge never shows.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::TimelineTime;
use bettercut_playback::engine::{BOUNCE_HZ, MAX_BOUNCE, bounce};

fn at_phase(hop: u32, phase: f64) -> TimelineTime {
    let s = (f64::from(hop) + phase) / BOUNCE_HZ;
    TimelineTime::from_ticks((s * bettercut_foundation::TICKS_PER_SECOND as f64) as i64)
}

#[test]
fn off_does_nothing() {
    assert!(bounce(0.0, at_phase(0, 0.5)).is_none());
    assert!(bounce(f32::NAN, at_phase(0, 0.5)).is_none());
}

/// On the ground at the start of each hop, highest halfway, always upwards.
#[test]
fn it_hops_up_and_lands() {
    for hop in 0..4 {
        let (ground, _) = bounce(100.0, at_phase(hop, 0.001)).unwrap();
        let (top, _) = bounce(100.0, at_phase(hop, 0.5)).unwrap();
        assert!(ground.abs() < 0.001, "{ground}");
        assert!((top + MAX_BOUNCE).abs() < 1e-4, "{top}");
    }
    for i in 0..100 {
        let (y, _) = bounce(100.0, at_phase(0, f64::from(i) / 100.0)).unwrap();
        assert!((-MAX_BOUNCE - 1e-6..=0.0).contains(&y));
    }
}

/// Lifted by `l` frame heights, the bottom edge rises by `l`; enlarged about
/// the middle by the steady zoom, it reaches at least that far back down.
#[test]
fn the_bottom_edge_stays_covered() {
    for amount in [10.0, 50.0, 100.0] {
        for i in 0..50 {
            let (y, zoom) = bounce(amount, at_phase(0, f64::from(i) / 50.0)).unwrap();
            let bottom = 0.5 * zoom + y;
            assert!(bottom >= 0.5 - 1e-6, "amount {amount}: bottom at {bottom}");
        }
    }
}
