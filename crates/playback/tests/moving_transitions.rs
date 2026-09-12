//! §25's moving transitions: slide, push and zoom.
//!
//! All three show both clips at once and differ only in where the two are
//! drawn, so the whole of each is a handful of numbers — and those numbers are
//! what these tests hold. What the compositor does with a transform is tested
//! where the compositor is; what a *slide* means is here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_playback::engine::{LayerMove, moving_transition};
use bettercut_timeline::TransitionKind;

/// Where the two layers are at the start, middle and end of the window.
fn across(kind: TransitionKind) -> [(LayerMove, LayerMove); 3] {
    [
        moving_transition(kind, 0.0),
        moving_transition(kind, 0.5),
        moving_transition(kind, 1.0),
    ]
}

/// The incoming shot arrives from the right and lands centred; the outgoing one
/// does not move and does not fade — it is being *covered*, which is what makes
/// a slide read differently from a crossfade.
#[test]
fn a_slide_brings_the_next_shot_in_from_the_right() {
    let [start, middle, end] = across(TransitionKind::Slide);

    assert_eq!(start.1.offset_x, 1.0, "it does not start off the frame");
    assert_eq!(middle.1.offset_x, 0.5);
    assert_eq!(end.1.offset_x, 0.0, "it does not finish centred");

    for (outgoing, _) in [start, middle, end] {
        assert_eq!(outgoing.offset_x, 0.0, "the outgoing shot moved");
        assert_eq!(outgoing.alpha, 1.0, "the outgoing shot faded");
    }
    for (_, incoming) in [start, middle, end] {
        assert_eq!(incoming.alpha, 1.0, "a slide is not a fade");
    }
}

/// Both shots move together, as if on one strip: whatever the incoming clip has
/// not covered, the outgoing one has already vacated. A gap between them would
/// show the black behind.
#[test]
fn a_push_moves_both_shots_by_the_same_amount() {
    for step in 0..=10 {
        let t = step as f32 / 10.0;
        let (outgoing, incoming) = moving_transition(TransitionKind::Push, t);
        assert!(
            (incoming.offset_x - outgoing.offset_x - 1.0).abs() < 1e-6,
            "at {t} the shots are {:.3} apart, not one frame",
            incoming.offset_x - outgoing.offset_x
        );
    }

    let [start, _, end] = across(TransitionKind::Push);
    assert_eq!(start.0.offset_x, 0.0, "the outgoing shot starts centred");
    assert_eq!(
        end.0.offset_x, -1.0,
        "the outgoing shot ends off to the left"
    );
    assert_eq!(end.1.offset_x, 0.0, "the incoming shot ends centred");
}

/// The outgoing shot swells and fades with the incoming one at rest behind it.
/// Scaling the *incoming* clip up from small would show the frame's edges
/// around it for the first half of the window.
#[test]
fn a_zoom_swells_the_outgoing_shot_and_leaves_the_next_at_rest() {
    let [start, middle, end] = across(TransitionKind::Zoom);

    assert_eq!(start.0.scale, 1.0, "it does not start at its own size");
    assert!(
        middle.0.scale > 1.0 && end.0.scale > middle.0.scale,
        "it does not grow"
    );
    assert_eq!(start.0.alpha, 1.0);
    assert_eq!(end.0.alpha, 0.0, "the outgoing shot never goes away");

    for (_, incoming) in [start, middle, end] {
        assert_eq!(incoming.scale, 1.0, "the incoming shot was scaled");
        assert_eq!(incoming.offset_x, 0.0, "the incoming shot was moved");
        assert_eq!(incoming.alpha, 1.0, "the incoming shot was faded");
    }
}

/// Progress outside the window saturates rather than extrapolating: a caller
/// asking about the wrong instant gets the end of the transition, not a shot
/// two frame-widths off screen.
#[test]
fn progress_outside_the_window_saturates() {
    for kind in [
        TransitionKind::Slide,
        TransitionKind::Push,
        TransitionKind::Zoom,
    ] {
        assert_eq!(moving_transition(kind, -3.0), moving_transition(kind, 0.0));
        assert_eq!(moving_transition(kind, 9.0), moving_transition(kind, 1.0));
    }
}

/// Every moving transition ends with the next shot exactly where an ordinary
/// cut would have put it. A transition that finished a few pixels off would
/// leave a visible jump on the frame after it.
#[test]
fn every_moving_transition_lands_the_next_shot_square() {
    for kind in TransitionKind::ALL {
        let (_, incoming) = moving_transition(kind, 1.0);
        assert_eq!(
            incoming.offset_x,
            0.0,
            "{} left it off-centre",
            kind.label()
        );
        assert_eq!(
            incoming.scale,
            1.0,
            "{} left it the wrong size",
            kind.label()
        );
        assert_eq!(incoming.alpha, 1.0, "{} left it see-through", kind.label());
    }
}

/// Everything that shows two clips at once needs handles; only a fade through
/// black works against the very start or end of a file.
#[test]
fn the_moving_kinds_need_handles() {
    assert!(!TransitionKind::FadeThroughBlack.needs_handles());
    for kind in [
        TransitionKind::Crossfade,
        TransitionKind::Slide,
        TransitionKind::Push,
        TransitionKind::Zoom,
    ] {
        assert!(kind.needs_handles(), "{} claims to need none", kind.label());
    }
}
