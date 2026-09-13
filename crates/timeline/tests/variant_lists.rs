//! Every `ALL` list, held to the enum it belongs to.
//!
//! An `ALL` is a hand-written list of variants and nothing makes it complete.
//! Add a variant, forget the list, and the interface simply never offers it —
//! there is no error, no warning, and the control is missing in a way only a
//! user notices. That is not hypothetical: the template validator carried its
//! own list of two transition kinds while the editor grew to seven, so a
//! template asking for a zoom was told a zoom is not a transition.
//!
//! Each test here classifies its enum with an **exhaustive match**, which is
//! the only completeness check the compiler actually enforces. Adding a variant
//! stops this file compiling until someone has said which list it belongs in —
//! and the length assertions then make sure the list was really updated rather
//! than the match alone.
//!
//! Some lists are deliberately partial. Those say so, and say why, rather than
//! being quietly short.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_timeline::{
    AnimatedParameter, Backdrop, BlendMode, FlipAxis, MaskShape, MotionKind, Movement, Transform,
    TransitionKind,
};

#[test]
fn every_blend_mode_is_offered() {
    fn offered(mode: BlendMode) -> bool {
        match mode {
            BlendMode::Normal | BlendMode::Screen | BlendMode::Multiply | BlendMode::Add => true,
        }
    }

    assert_eq!(BlendMode::ALL.len(), 4, "a blend mode was added or removed");
    assert!(BlendMode::ALL.into_iter().all(offered));
}

#[test]
fn every_mask_shape_is_offered() {
    fn offered(shape: MaskShape) -> bool {
        match shape {
            MaskShape::Linear | MaskShape::Rectangle | MaskShape::Ellipse => true,
        }
    }

    assert_eq!(MaskShape::ALL.len(), 3, "a mask shape was added or removed");
    assert!(MaskShape::ALL.into_iter().all(offered));
}

#[test]
fn every_backdrop_is_offered() {
    fn offered(backdrop: Backdrop) -> bool {
        match backdrop {
            Backdrop::None | Backdrop::Blur => true,
        }
    }

    assert_eq!(Backdrop::ALL.len(), 2, "a backdrop was added or removed");
    assert!(Backdrop::ALL.into_iter().all(offered));
}

#[test]
fn every_movement_is_offered() {
    fn offered(movement: Movement) -> bool {
        match movement {
            Movement::None | Movement::ZoomIn | Movement::ZoomOut => true,
        }
    }

    assert_eq!(Movement::ALL.len(), 3, "a movement was added or removed");
    assert!(Movement::ALL.into_iter().all(offered));
}

/// §25's seven, split by whether they read outside a clip's own range. The
/// handle rule is tested elsewhere; this is about the list being whole.
#[test]
fn every_transition_kind_is_offered() {
    fn offered(kind: TransitionKind) -> bool {
        match kind {
            TransitionKind::Crossfade
            | TransitionKind::FadeThroughBlack
            | TransitionKind::Slide
            | TransitionKind::Push
            | TransitionKind::Zoom
            | TransitionKind::Flash
            | TransitionKind::Blur => true,
        }
    }

    assert_eq!(
        TransitionKind::ALL.len(),
        7,
        "a transition was added or removed"
    );
    assert!(TransitionKind::ALL.into_iter().all(offered));
}

/// Two lists, and the difference between them is the point: a picture has no
/// letters to reveal, so the typewriter is a title's alone.
#[test]
fn every_motion_is_in_the_right_lists() {
    /// `(for a shot, for a title)`.
    fn offered(kind: MotionKind) -> (bool, bool) {
        match kind {
            MotionKind::Fade
            | MotionKind::SlideUp
            | MotionKind::SlideDown
            | MotionKind::SlideRight
            | MotionKind::SlideLeft
            | MotionKind::Pop
            | MotionKind::Spin => (true, true),
            MotionKind::Typewriter => (false, true),
        }
    }

    for kind in MotionKind::FOR_TEXT {
        let (shot, title) = offered(kind);
        assert_eq!(
            MotionKind::ALL.contains(&kind),
            shot,
            "{} is on the wrong side of the picture list",
            kind.label()
        );
        assert!(title, "{} is missing from the title list", kind.label());
    }

    assert_eq!(MotionKind::ALL.len(), 7);
    assert_eq!(MotionKind::FOR_TEXT.len(), 8);
}

/// Two axes, and each has to reach its own flag. A `flag` that returned the
/// horizontal one for both would make the vertical button mirror sideways —
/// which looks like something happening, so it is the kind of wiring mistake
/// that survives a glance at the interface.
#[test]
fn every_mirror_axis_is_offered_and_reaches_its_own_flag() {
    fn offered(axis: FlipAxis) -> bool {
        match axis {
            FlipAxis::Horizontal | FlipAxis::Vertical => true,
        }
    }

    assert_eq!(FlipAxis::ALL.len(), 2, "a mirror axis was added or removed");
    assert!(FlipAxis::ALL.into_iter().all(offered));

    for axis in FlipAxis::ALL {
        let mut transform = Transform::default();
        *axis.flag(&mut transform) = true;
        assert!(axis.is_set(&transform), "{} did not set", axis.label());
        for other in FlipAxis::ALL {
            assert_eq!(
                other.is_set(&transform),
                other == axis,
                "{} moved {}'s flag",
                axis.label(),
                other.label()
            );
        }
    }
}

/// Deliberately partial: `ALL` is what the Inspector offers for a *picture*,
/// and a clip's volume is not a picture property. Stated here so the omission
/// is a decision rather than an oversight.
#[test]
fn the_animated_parameters_offer_picture_only() {
    fn is_picture(parameter: AnimatedParameter) -> bool {
        match parameter {
            AnimatedParameter::Opacity
            | AnimatedParameter::PositionX
            | AnimatedParameter::PositionY
            | AnimatedParameter::ScaleX
            | AnimatedParameter::ScaleY
            | AnimatedParameter::Rotation
            | AnimatedParameter::Brightness
            | AnimatedParameter::Contrast
            | AnimatedParameter::Saturation
            | AnimatedParameter::Temperature
            | AnimatedParameter::Tint
            | AnimatedParameter::Blur => true,
            AnimatedParameter::Gain => false,
        }
    }

    assert_eq!(AnimatedParameter::ALL.len(), 12);
    assert!(AnimatedParameter::ALL.into_iter().all(is_picture));
    assert!(
        !AnimatedParameter::ALL.contains(&AnimatedParameter::Gain),
        "volume is sound, and belongs to the audio controls"
    );
}
