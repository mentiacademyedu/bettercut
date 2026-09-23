//! The colour wheels as data (`bettercut_timeline::ColorWheels`): at rest by
//! default, held to their range, blended and composed like the rest of the
//! grade.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_timeline::{ColorAdjust, ColorWheels};

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-6)
}

/// A grade that says nothing about its wheels has them at rest, and a grade
/// at rest is still the identity the renderer can skip.
#[test]
fn wheels_start_at_rest_and_keep_the_grade_an_identity() {
    assert!(ColorWheels::default().is_identity());
    assert_eq!(ColorWheels::default(), ColorWheels::IDENTITY);
    assert!(ColorAdjust::default().is_identity());
    assert!(ColorAdjust::IDENTITY.is_identity());

    let mut graded = ColorAdjust::default();
    graded.wheels.gain = [0.0, 0.0, 0.1];
    assert!(
        !graded.is_identity(),
        "a touched wheel still read as untouched"
    );
}

/// Past full is not more of a colour, and nonsense is nothing: the shader is
/// never asked for a power it would turn into NaN.
#[test]
fn wheels_are_held_to_their_range() {
    let wild = ColorWheels {
        lift: [3.0, -9.0, 0.5],
        gamma: [f32::NAN, f32::INFINITY, -0.25],
        gain: [1.0, -1.0, 0.0],
    };
    let held = wild.clamped();
    assert!(close(held.lift, [1.0, -1.0, 0.5]));
    assert!(close(held.gamma, [0.0, 0.0, -0.25]));
    assert!(close(held.gain, [1.0, -1.0, 0.0]));
}

/// Halfway between two grades is halfway on every wheel, and a look's
/// strength blends the wheels along with the rest of the grade.
#[test]
fn wheels_blend_with_the_rest_of_the_grade() {
    let mut warm = ColorAdjust::default();
    warm.wheels.lift = [0.2, 0.0, -0.2];
    warm.wheels.gain = [0.4, 0.2, 0.0];

    let half = ColorAdjust::default().lerp(warm, 0.5);
    assert!(close(half.wheels.lift, [0.1, 0.0, -0.1]));
    assert!(close(half.wheels.gain, [0.2, 0.1, 0.0]));
    assert!(close(half.wheels.gamma, [0.0; 3]));

    // NaN and out-of-range strengths land on an end, never in nonsense.
    assert!(close(
        ColorAdjust::default().lerp(warm, f32::NAN).wheels.lift,
        [0.0; 3]
    ));
    assert!(close(
        ColorAdjust::default().lerp(warm, 7.0).wheels.gain,
        warm.wheels.gain
    ));
}

/// One set of wheels on top of another adds, held to the range — what
/// grading an already-graded picture does.
#[test]
fn wheels_compose_by_adding() {
    let outer = ColorWheels {
        lift: [0.5, 0.0, 0.0],
        gamma: [0.0, 0.8, 0.0],
        gain: [0.0, 0.0, -0.3],
    };
    let inner = ColorWheels {
        lift: [0.7, 0.1, 0.0],
        gamma: [0.0, 0.5, 0.0],
        gain: [0.0, 0.0, -0.2],
    };
    let both = outer.combined(inner);
    assert!(close(both.lift, [1.0, 0.1, 0.0]), "{:?}", both.lift);
    assert!(close(both.gamma, [0.0, 1.0, 0.0]), "{:?}", both.gamma);
    assert!(close(both.gain, [0.0, 0.0, -0.5]), "{:?}", both.gain);
    assert_eq!(outer.combined(ColorWheels::IDENTITY), outer);
}

/// Older projects say nothing about wheels, and load with them at rest.
#[test]
fn a_grade_written_before_the_wheels_loads_at_rest() {
    let json = r#"{"brightness":1.2,"contrast":1.0,"saturation":0.9,"temperature":0.1,"tint":0.0,"vibrance":0.3}"#;
    let grade: ColorAdjust = serde_json::from_str(json).unwrap();
    assert_eq!(grade.brightness, 1.2);
    assert!(grade.wheels.is_identity());

    let mut graded = grade;
    graded.wheels.lift = [0.1, 0.2, 0.3];
    let back: ColorAdjust = serde_json::from_str(&serde_json::to_string(&graded).unwrap()).unwrap();
    assert_eq!(back, graded);
}
