//! Sway (`sway`): the picture rocks from side to side, enlarged just enough
//! that no corner of what is behind it ever shows.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::TimelineTime;
use bettercut_playback::engine::{MAX_SWAY_DEGREES, SWAY_HZ, sway};

fn seconds(s: f64) -> TimelineTime {
    TimelineTime::from_ticks((s * bettercut_foundation::TICKS_PER_SECOND as f64) as i64)
}

#[test]
fn off_does_nothing() {
    assert!(sway(0.0, seconds(1.0), 16.0 / 9.0).is_none());
    assert!(sway(f32::NAN, seconds(1.0), 16.0 / 9.0).is_none());
}

/// It tips one way, then the other, and never further than the most.
#[test]
fn it_rocks_both_ways_within_the_most() {
    let period = 1.0 / SWAY_HZ;
    let (left, _) = sway(100.0, seconds(period * 0.25), 16.0 / 9.0).unwrap();
    let (right, _) = sway(100.0, seconds(period * 0.75), 16.0 / 9.0).unwrap();
    assert!(left > MAX_SWAY_DEGREES * 0.99, "{left}");
    assert!(right < -MAX_SWAY_DEGREES * 0.99, "{right}");
    for i in 0..100 {
        let (degrees, _) = sway(100.0, seconds(i as f64 * 0.037), 16.0 / 9.0).unwrap();
        assert!(degrees.abs() <= MAX_SWAY_DEGREES + 1e-4);
    }
}

/// The enlargement is held, so the picture does not breathe as it rocks,
/// and it covers the corners: a frame turned by the furthest tip and scaled
/// by it still reaches every corner of the frame.
#[test]
fn the_enlargement_is_steady_and_covers_the_corners() {
    for aspect in [16.0 / 9.0, 9.0 / 16.0, 1.0] {
        let (_, a) = sway(60.0, seconds(0.1), aspect).unwrap();
        let (_, b) = sway(60.0, seconds(0.9), aspect).unwrap();
        assert_eq!(a, b);

        // The frame's corner, turned back by the furthest tip, must land
        // inside the enlarged picture.
        let (w, h) = (aspect.max(1.0), aspect.max(1.0) / aspect);
        let theta = (MAX_SWAY_DEGREES * 0.6).to_radians();
        for (x, y) in [(w / 2.0, h / 2.0), (w / 2.0, -h / 2.0)] {
            let (c, s) = (theta.cos(), theta.sin());
            let (u, v) = (x * c - y * s, x * s + y * c);
            assert!(u.abs() <= a * w / 2.0 + 1e-4 && v.abs() <= a * h / 2.0 + 1e-4);
        }
    }
    let (_, gentle) = sway(20.0, seconds(0.0), 16.0 / 9.0).unwrap();
    let (_, strong) = sway(100.0, seconds(0.0), 16.0 / 9.0).unwrap();
    assert!(1.0 < gentle && gentle < strong);
}
