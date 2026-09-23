//! The secondary as data (`bettercut_timeline::HslSecondary`): at rest by
//! default, the pick a choice rather than a change, held to its range, and
//! blended like the rest of the grade.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_timeline::{ColorAdjust, HslSecondary};

/// Moving the pick alone touches nothing: it says which colours, not what
/// happens to them.
#[test]
fn a_pick_with_no_shift_is_at_rest() {
    let mut picked = HslSecondary::IDENTITY;
    picked.hue = 0.66;
    picked.width = 0.2;
    assert!(picked.is_identity());
    assert!(ColorAdjust::default().is_identity());

    let grade = ColorAdjust {
        secondary: HslSecondary {
            saturation: 0.3,
            ..HslSecondary::IDENTITY
        },
        ..ColorAdjust::default()
    };
    assert!(
        !grade.is_identity(),
        "a shifted secondary read as untouched"
    );
}

/// The hue wraps round the circle, the width is held to a quarter turn each
/// way, and nonsense is rest.
#[test]
fn a_secondary_is_held_to_its_range() {
    let wild = HslSecondary {
        hue: 1.25,
        width: 9.0,
        hue_shift: -4.0,
        saturation: f32::NAN,
        luminance: 0.5,
    };
    let held = wild.clamped();
    assert!((held.hue - 0.25).abs() < 1e-6, "{}", held.hue);
    assert_eq!(held.width, HslSecondary::MAX_WIDTH);
    assert_eq!(held.hue_shift, -1.0);
    assert_eq!(held.saturation, 0.0);
    assert_eq!(held.luminance, 0.5);

    let broken = HslSecondary {
        width: f32::INFINITY,
        ..HslSecondary::IDENTITY
    };
    assert_eq!(broken.clamped().width, HslSecondary::DEFAULT_WIDTH);
}

/// Halfway to a look is half its shifts, on the look's own pick — half of one
/// pick and half of another would be a colour nobody chose.
#[test]
fn a_secondary_blends_its_shifts_and_keeps_a_pick() {
    let look = HslSecondary {
        hue: 0.6,
        width: 0.1,
        hue_shift: 0.4,
        saturation: -0.2,
        luminance: 0.0,
    };
    let half = HslSecondary::IDENTITY.lerp(look, 0.5);
    assert!((half.hue - 0.6).abs() < 1e-6);
    assert!((half.width - 0.1).abs() < 1e-6);
    assert!((half.hue_shift - 0.2).abs() < 1e-6);
    assert!((half.saturation + 0.1).abs() < 1e-6);

    let grade = ColorAdjust {
        secondary: look,
        ..ColorAdjust::default()
    };
    let halfway = ColorAdjust::default().lerp(grade, 0.5);
    assert!((halfway.secondary.hue_shift - 0.2).abs() < 1e-6);
}

/// Over another grade, the one that does something wins: two picks cannot
/// be one pick.
#[test]
fn a_secondary_over_another_keeps_the_one_that_moves() {
    let moving = HslSecondary {
        hue: 0.3,
        hue_shift: 0.5,
        ..HslSecondary::IDENTITY
    };
    assert_eq!(HslSecondary::IDENTITY.combined(moving), moving);
    assert_eq!(moving.combined(HslSecondary::IDENTITY), moving);
    let other = HslSecondary {
        hue: 0.9,
        luminance: -0.5,
        ..HslSecondary::IDENTITY
    };
    assert_eq!(moving.combined(other), moving);
}

/// Older projects say nothing about a secondary, and load with one at rest
/// — with the default reach, not a reach of nothing.
#[test]
fn a_grade_written_before_the_secondary_loads_at_rest() {
    let json = r#"{"brightness":1.0,"contrast":1.0,"saturation":1.0}"#;
    let grade: ColorAdjust = serde_json::from_str(json).unwrap();
    assert!(grade.secondary.is_identity());
    assert_eq!(grade.secondary.width, HslSecondary::DEFAULT_WIDTH);

    let mut picked = grade;
    picked.secondary = HslSecondary {
        hue: 0.12,
        width: 0.05,
        hue_shift: 0.1,
        saturation: 0.2,
        luminance: -0.3,
    };
    let back: ColorAdjust = serde_json::from_str(&serde_json::to_string(&picked).unwrap()).unwrap();
    assert_eq!(back, picked);
}
