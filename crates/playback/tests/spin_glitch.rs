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

#[test]
fn a_pixelate_is_sharp_at_the_ends_and_blockiest_at_the_cut() {
    use bettercut_playback::engine::transition_pixelate;
    assert!(transition_pixelate(0.0).abs() < 1e-3);
    assert!(transition_pixelate(1.0).abs() < 1e-3);
    assert!((transition_pixelate(0.5) - 100.0).abs() < 1e-3);
    assert!(transition_pixelate(0.25) > 0.0 && transition_pixelate(0.25) < 100.0);
    // The two clips get the same blockiness the same distance from the cut.
    assert!((transition_pixelate(0.3) - transition_pixelate(0.7)).abs() < 1e-3);
}

#[test]
fn a_shake_is_still_at_the_ends_and_covers_the_frame_while_it_moves() {
    use bettercut_playback::engine::transition_shake;
    for t in [0.0, 1.0] {
        let (x, y, zoom) = transition_shake(t);
        assert!(
            x.abs() < 1e-3 && y.abs() < 1e-3 && (zoom - 1.0).abs() < 1e-3,
            "at {t}"
        );
    }
    // Never thrown further than the enlargement hides: the edges stay covered.
    for i in 0..=100 {
        let (x, y, zoom) = transition_shake(i as f32 / 100.0);
        let spare = (zoom - 1.0) / 2.0;
        assert!(
            x.abs() <= spare + 1e-4 && y.abs() <= spare + 1e-4,
            "at {i}%: {x} {y} {zoom}"
        );
    }
    assert_eq!(transition_shake(0.42), transition_shake(0.42));
}

#[test]
fn the_new_one_shot_kinds_do_not_move_either_clip() {
    for kind in [
        TransitionKind::Pixelate,
        TransitionKind::Shake,
        TransitionKind::FadeThroughWhite,
    ] {
        let (out, incoming) = moving_transition(kind, 0.5);
        assert_eq!((out.scale, out.rotation), (1.0, 0.0), "{}", kind.label());
        assert_eq!(
            (incoming.scale, incoming.rotation),
            (1.0, 0.0),
            "{}",
            kind.label()
        );
    }
}
