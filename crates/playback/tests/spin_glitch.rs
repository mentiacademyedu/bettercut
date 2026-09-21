//! The Spin and Glitch transitions (`moving_transition`, `transition_glitch`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_playback::engine::{moving_transition, transition_glitch};
use bettercut_timeline::TransitionKind;

/// Out spins away while in is hidden; in unwinds while out is hidden; and at
/// the change-over the two are drawn identically, so nothing jumps.
#[test]
fn a_spin_hands_over_without_a_jump() {
    let (out_start, in_start) = moving_transition(TransitionKind::Spin, 0.0);
    assert_eq!(
        (out_start.scale, out_start.rotation, out_start.alpha),
        (1.0, 0.0, 1.0)
    );
    assert_eq!(in_start.alpha, 0.0);

    let (out_before, _) = moving_transition(TransitionKind::Spin, 0.4999);
    let (_, in_after) = moving_transition(TransitionKind::Spin, 0.5);
    assert!((out_before.scale - in_after.scale).abs() < 0.01);
    assert!((out_before.rotation - (in_after.rotation + 360.0)).abs() < 0.5);
    assert_eq!(in_after.alpha, 1.0);

    let (out_end, in_end) = moving_transition(TransitionKind::Spin, 1.0);
    assert_eq!(out_end.alpha, 0.0);
    assert_eq!(
        (in_end.scale, in_end.rotation, in_end.alpha),
        (1.0, 0.0, 1.0)
    );
}

#[test]
fn a_glitch_is_clean_at_the_ends_and_wrecked_at_the_cut() {
    for t in [0.0, 1.0] {
        let (amount, shake) = transition_glitch(t);
        assert!(amount.abs() < 1e-3 && shake.abs() < 1e-3, "at {t}");
    }
    let (amount, _) = transition_glitch(0.5);
    assert!(amount > 99.0, "{amount}");
    // It lurches: not every step the same way.
    let shakes: Vec<f32> = (1..12)
        .map(|i| transition_glitch(i as f32 / 12.0 + 0.01).1)
        .collect();
    assert!(shakes.iter().any(|s| *s > 0.0) && shakes.iter().any(|s| *s < 0.0));
    // And the same instant always lurches the same way.
    assert_eq!(transition_glitch(0.37), transition_glitch(0.37));
}
