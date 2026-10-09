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
            MaskShape::Linear
            | MaskShape::Rectangle
            | MaskShape::Ellipse
            | MaskShape::Star
            | MaskShape::Heart
            | MaskShape::Mirror => true,
        }
    }

    assert_eq!(MaskShape::ALL.len(), 6, "a mask shape was added or removed");
    assert!(MaskShape::ALL.into_iter().all(offered));
}

#[test]
fn every_backdrop_is_offered() {
    fn offered(backdrop: Backdrop) -> bool {
        match backdrop {
            Backdrop::None | Backdrop::Blur => true,
            // Offered as the project's pictures, not as one entry of ALL.
            Backdrop::Image(_) => false,
        }
    }

    assert_eq!(Backdrop::ALL.len(), 2, "a backdrop was added or removed");
    assert!(Backdrop::ALL.into_iter().all(offered));
}

#[test]
fn every_movement_is_offered() {
    fn offered(movement: Movement) -> bool {
        match movement {
            Movement::None
            | Movement::ZoomIn
            | Movement::ZoomOut
            | Movement::PanLeft
            | Movement::PanRight
            | Movement::PanUp
            | Movement::PanDown => true,
        }
    }

    assert_eq!(Movement::ALL.len(), 7, "a movement was added or removed");
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
            | TransitionKind::Blur
            | TransitionKind::Wipe
            | TransitionKind::Iris
            | TransitionKind::Spin
            | TransitionKind::Glitch
            | TransitionKind::Pixelate
            | TransitionKind::Shake
            | TransitionKind::FadeThroughWhite
            | TransitionKind::SlideUp
            | TransitionKind::PushUp
            | TransitionKind::WipeDown
            | TransitionKind::LightLeak => true,
        }
    }

    assert_eq!(
        TransitionKind::ALL.len(),
        18,
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
            | MotionKind::Bounce
            | MotionKind::Spin
            | MotionKind::ZoomOut
            | MotionKind::Swing => (true, true),
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

    assert_eq!(MotionKind::ALL.len(), 10);
    assert_eq!(MotionKind::FOR_TEXT.len(), 11);
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
            AnimatedParameter::Gain | AnimatedParameter::Pan => false,
        }
    }

    assert_eq!(AnimatedParameter::ALL.len(), 12);
    assert!(AnimatedParameter::ALL.into_iter().all(is_picture));
    assert!(
        !AnimatedParameter::ALL.contains(&AnimatedParameter::Gain),
        "volume is sound, and belongs to the audio controls"
    );
    assert!(
        !AnimatedParameter::ALL.contains(&AnimatedParameter::Pan),
        "pan is sound too, and belongs beside the volume"
    );
}

/// Cinematic bars: how much of the height each covers.
#[test]
fn bar_heights_follow_the_shape() {
    use bettercut_timeline::bar_height;
    // 16:9 cut to 2.39:1 keeps about three quarters of the height.
    let bar = bar_height(16.0 / 9.0, 2.39);
    assert!((bar - 0.128).abs() < 0.001, "{bar}");
    assert_eq!(bar_height(1.0, 2.0), 0.25);
    // No bars, or a shape no wider than the frame, covers nothing.
    assert_eq!(bar_height(16.0 / 9.0, 0.0), 0.0);
    assert_eq!(bar_height(16.0 / 9.0, 1.5), 0.0);
    assert_eq!(bar_height(16.0 / 9.0, f32::NAN), 0.0);
}

/// Tones: sepia warms a grey, a duotone maps black and white to its two
/// colours, and a tone alone makes the curves something to draw.
#[test]
fn tones_remap_the_picture_and_count_as_a_grade() {
    use bettercut_timeline::curves::{ColourCurves, Tone};

    let grey = Tone::Sepia.apply([0.5, 0.5, 0.5]);
    assert!(grey[0] > grey[1] && grey[1] > grey[2], "not warm: {grey:?}");

    let duo = Tone::Duotone {
        shadow: [0, 0, 255],
        highlight: [255, 255, 0],
    };
    assert_eq!(duo.apply([0.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
    assert_eq!(duo.apply([1.0, 1.0, 1.0]), [1.0, 1.0, 0.0]);

    let toned = ColourCurves {
        tone: Tone::Sepia,
        ..ColourCurves::default()
    };
    assert!(toned.is_straight() && !toned.is_identity());
    assert_ne!(toned.lut_id(None), ColourCurves::default().lut_id(None));
    assert_eq!(toned.apply([0.5, 0.5, 0.5]), grey);
}

/// Colour wheels: a dot pushed towards a hue tints without brightening,
/// a wheel's position comes back as it went in, and the wheels count as a
/// grade.
#[test]
fn colour_wheels_tint_the_range_they_are_for() {
    use bettercut_timeline::curves::{ColourCurves, ColourWheels};

    // Towards red, halfway: red up, green and blue down, brightness unchanged.
    let red = ColourWheels::offsets(0.0, 0.5, 0.0);
    assert!(red[0] > 0.4 && red[1] < 0.0 && red[2] < 0.0, "{red:?}");
    assert!((red.iter().sum::<f32>()).abs() < 1e-4);
    let (angle, distance, brightness) = ColourWheels::position(red);
    assert!(angle.abs() < 1e-3 && (distance - 0.5).abs() < 1e-3 && brightness.abs() < 1e-4);

    let blue_back = ColourWheels::offsets(2.0, 0.6, 0.25);
    let (a, d, b) = ColourWheels::position(blue_back);
    assert!((a - 2.0).abs() < 1e-3 && (d - 0.6).abs() < 1e-3 && (b - 0.25).abs() < 1e-3);

    // Warm shadows lift red in the dark and leave white alone.
    let wheels = ColourWheels {
        shadows: red,
        ..ColourWheels::default()
    };
    let dark = wheels.apply([0.05, 0.05, 0.05]);
    assert!(dark[0] > dark[2], "{dark:?}");
    let white = wheels.apply([1.0, 1.0, 1.0]);
    assert!(white.iter().all(|v| (v - 1.0).abs() < 1e-4), "{white:?}");

    // Warm highlights move white, not black.
    let bright = ColourWheels {
        highlights: [-0.5, -0.5, -0.5],
        ..ColourWheels::default()
    };
    assert!(bright.apply([1.0, 1.0, 1.0])[0] < 0.9);
    assert_eq!(bright.apply([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);

    let curves = ColourCurves {
        wheels,
        ..ColourCurves::default()
    };
    assert!(curves.is_straight() && !curves.is_identity());
    assert_ne!(curves.lut_id(None), ColourCurves::default().lut_id(None));
}

/// The colour mixer moves one range and leaves greys and far colours alone.
#[test]
fn the_colour_mixer_changes_one_colour_range() {
    use bettercut_timeline::curves::{ColourCurves, ColourMixer, MixerBand};

    let mut mixer = ColourMixer::default();
    // Blue (index 5): less saturated.
    mixer.bands[5] = MixerBand {
        saturation: -1.0,
        ..MixerBand::default()
    };
    let sky = mixer.apply([0.2, 0.4, 0.9]);
    let spread =
        |c: [f32; 3]| c.iter().copied().fold(0.0, f32::max) - c.iter().copied().fold(1.0, f32::min);
    assert!(
        spread(sky) < spread([0.2, 0.4, 0.9]) * 0.5,
        "the blue kept its colour: {sky:?}"
    );

    // A red is a long way round the wheel: untouched.
    let red = mixer.apply([0.9, 0.1, 0.1]);
    assert!(
        red.iter()
            .zip([0.9, 0.1, 0.1])
            .all(|(a, b)| (a - b).abs() < 1e-3),
        "{red:?}"
    );
    // A grey belongs to no range.
    assert_eq!(mixer.apply([0.5, 0.5, 0.5]), [0.5, 0.5, 0.5]);

    // Reds towards orange: green rises.
    let mut warm = ColourMixer::default();
    warm.bands[0].hue = 1.0;
    let moved = warm.apply([0.9, 0.1, 0.1]);
    assert!(moved[1] > 0.2, "{moved:?}");

    let curves = ColourCurves {
        mixer,
        ..ColourCurves::default()
    };
    assert!(!curves.is_identity());
    assert_ne!(curves.lut_id(None), ColourCurves::default().lut_id(None));
}

/// Scrolling titles: credits enter wholly below the frame and leave wholly
/// above it, a ticker right to left, and a sanitized animation keeps its scroll.
#[test]
fn scrolling_titles_cross_the_whole_frame() {
    use bettercut_foundation::TimelineTime;
    use bettercut_timeline::{Scroll, TextAnimation, TimelineRange, Transform};

    let span = TimelineRange::new(
        TimelineTime::from_seconds(2),
        TimelineTime::from_seconds(12),
    )
    .unwrap();
    let credits = TextAnimation {
        scroll: Some(Scroll::Credits),
        ..TextAnimation::default()
    };
    let at = |animation: &TextAnimation, seconds: i64| {
        animation.look(
            Transform::default(),
            1.0,
            span,
            TimelineTime::from_seconds(seconds),
            10,
        )
    };
    let start = at(&credits, 2);
    // The text's top edge on the frame's bottom edge: all of it below.
    assert_eq!(
        (start.transform.anchor.y, start.transform.position.y),
        (0.0, 0.5)
    );
    let end = at(&credits, 12);
    // Its bottom edge on the top edge: all of it above.
    assert_eq!(
        (end.transform.anchor.y, end.transform.position.y),
        (1.0, -0.5)
    );
    let middle = at(&credits, 7);
    assert!((middle.transform.position.y - 0.0).abs() < 1e-5);

    let ticker = TextAnimation {
        scroll: Some(Scroll::Ticker),
        ..TextAnimation::default()
    };
    let begun = at(&ticker, 2);
    assert_eq!(
        (begun.transform.anchor.x, begun.transform.position.x),
        (0.0, 0.5)
    );
    assert_eq!(at(&ticker, 12).transform.position.x, -0.5);

    assert_eq!(credits.sanitized().scroll, Some(Scroll::Credits));
    assert!(!credits.is_none());
}
