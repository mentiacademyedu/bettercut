//! §25's wipe and iris: the incoming shot cut to a shape that grows
//! (`engine::transition_mask`).
//!
//! The coverage is worked out here with the compositor's own mask rule
//! (`mask_alpha` in composite.wgsl), written out in Rust, so what is tested
//! is how much of the next shot shows — not just that some numbers moved.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_playback::engine::{moving_transition, transition_mask};
use bettercut_timeline::{Mask, MaskShape, TransitionKind};

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// composite.wgsl's `mask_alpha`, for the two shapes these use.
fn kept(mask: &Mask, uv: [f32; 2]) -> f32 {
    let r = mask.rotation_degrees.to_radians();
    let (c, s) = (r.cos(), r.sin());
    let offset = [uv[0] - mask.center[0], uv[1] - mask.center[1]];
    let local = [
        offset[0] * c + offset[1] * s,
        -offset[0] * s + offset[1] * c,
    ];
    let distance = match mask.shape {
        MaskShape::Linear => local[1],
        MaskShape::Ellipse => {
            let half = [mask.size[0].max(1e-4), mask.size[1].max(1e-4)];
            ((local[0] / half[0]).powi(2) + (local[1] / half[1]).powi(2)).sqrt() - 1.0
        }
        MaskShape::Rectangle | MaskShape::Star | MaskShape::Heart => unreachable!(),
    };
    let feather = mask.feather.max(1e-4);
    let k = 1.0 - smoothstep(-feather * 0.5, feather * 0.5, distance);
    if mask.invert { 1.0 - k } else { k }
}

/// The share of the frame where the incoming shot shows.
fn shown(kind: TransitionKind, progress: f32) -> f32 {
    let mask = transition_mask(kind, progress).expect("a shaped transition");
    let n = 40;
    let mut total = 0.0;
    for y in 0..n {
        for x in 0..n {
            let uv = [(x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32];
            total += kept(&mask, uv);
        }
    }
    total / (n * n) as f32
}

#[test]
fn a_wipe_reveals_from_nothing_to_everything() {
    assert!(
        shown(TransitionKind::Wipe, 0.0) < 0.01,
        "the start shows the next shot"
    );
    assert!(
        shown(TransitionKind::Wipe, 1.0) > 0.99,
        "the end leaves some of the last shot"
    );
    let half = shown(TransitionKind::Wipe, 0.5);
    assert!((half - 0.5).abs() < 0.03, "halfway shows {half}");
    // Growing all the way.
    let mut last = -1.0;
    for step in 0..=10 {
        let now = shown(TransitionKind::Wipe, step as f32 / 10.0);
        assert!(now >= last, "the wipe went backwards at step {step}");
        last = now;
    }
}

/// From the left: part way through, the left edge shows the next shot and the
/// right edge still the last one.
#[test]
fn a_wipe_comes_from_the_left() {
    let mask = transition_mask(TransitionKind::Wipe, 0.3).unwrap();
    assert!(kept(&mask, [0.05, 0.5]) > 0.99);
    assert!(kept(&mask, [0.95, 0.5]) < 0.01);
}

#[test]
fn an_iris_opens_from_the_middle() {
    assert!(shown(TransitionKind::Iris, 0.0) < 0.01);
    assert!(
        shown(TransitionKind::Iris, 1.0) > 0.99,
        "the corners never open"
    );
    let mask = transition_mask(TransitionKind::Iris, 0.3).unwrap();
    assert!(kept(&mask, [0.5, 0.5]) > 0.99, "the middle is not open");
    assert!(kept(&mask, [0.02, 0.02]) < 0.01, "a corner opened early");
}

/// Neither moves or fades either shot, and nothing else is shaped.
#[test]
fn only_wipe_and_iris_are_shaped() {
    for kind in TransitionKind::ALL {
        let shaped = matches!(kind, TransitionKind::Wipe | TransitionKind::Iris);
        assert_eq!(
            transition_mask(kind, 0.5).is_some(),
            shaped,
            "{}",
            kind.label()
        );
        if shaped {
            let (outgoing, incoming) = moving_transition(kind, 0.5);
            assert_eq!((outgoing.offset_x, outgoing.alpha), (0.0, 1.0));
            assert_eq!((incoming.offset_x, incoming.alpha), (0.0, 1.0));
        }
    }
}
