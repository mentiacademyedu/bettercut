//! Camera shake (`camera_shake`): the picture thrown about, never showing an
//! edge, and nothing at all when it is off.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::TimelineTime;
use bettercut_playback::engine::{MAX_SHAKE, camera_shake};

/// Frame `n` at 24 frames a second.
fn frame_at(n: i64) -> TimelineTime {
    TimelineTime::from_ticks(n * bettercut_foundation::TICKS_PER_SECOND / 24)
}

#[test]
fn off_does_nothing() {
    assert!(camera_shake(0.0, TimelineTime::from_seconds(1)).is_none());
    assert!(camera_shake(f32::NAN, TimelineTime::from_seconds(1)).is_none());
}

/// However far it is thrown, the enlargement covers it: an offset of `x`
/// frame widths needs the picture `2x` wider to keep the edge out of frame.
#[test]
fn the_edges_never_show() {
    for frame in 0..240 {
        let at = frame_at(frame);
        let (x, y, zoom) = camera_shake(100.0, at).unwrap();
        assert!(x.abs() <= MAX_SHAKE && y.abs() <= MAX_SHAKE, "{x} {y}");
        assert!(zoom - 1.0 >= 2.0 * x.abs().max(y.abs()), "frame {frame}");
    }
}

/// It moves from frame to frame, and harder at a higher amount.
#[test]
fn it_moves_and_scales_with_the_amount() {
    let path = |amount: f32| -> f32 {
        (0..48)
            .map(|f| {
                let (x, y, _) = camera_shake(amount, frame_at(f)).unwrap();
                x.abs() + y.abs()
            })
            .sum()
    };
    let a = camera_shake(50.0, frame_at(10)).unwrap();
    let b = camera_shake(50.0, frame_at(11)).unwrap();
    assert_ne!((a.0, a.1), (b.0, b.1), "a still frame is not a shake");
    assert!(path(100.0) > path(20.0) * 4.0);
}
