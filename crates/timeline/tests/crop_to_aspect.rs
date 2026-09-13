//! §33's auto crop: the crop that makes a source the frame's own shape.
//!
//! One property carries almost all of it — **what is left must be the frame's
//! shape** — and it is worth testing as a property rather than on a couple of
//! hand-picked pairs, because the two branches (source wider than the frame,
//! source taller) are easy to write one of correctly and the other backwards.
//! A single 16:9-into-9:16 example would not notice.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_timeline::{Crop, crop_to_aspect};

/// The aspect of what a crop leaves, given the source it was taken from.
fn remaining_aspect(source: f32, crop: Crop) -> f32 {
    let (keep_x, keep_y) = crop.remaining();
    source * keep_x / keep_y
}

/// A spread of real shapes, rather than two: 16:9, vertical, square, ultrawide,
/// 4:3, and a cinema ratio.
const SHAPES: [f32; 6] = [16.0 / 9.0, 9.0 / 16.0, 1.0, 21.0 / 9.0, 4.0 / 3.0, 2.39];

/// The whole point: after cropping, the picture is the frame's shape, so it
/// fits with neither bars nor enlargement.
#[test]
fn what_is_left_is_always_the_frames_shape() {
    for source in SHAPES {
        for output in SHAPES {
            let crop = crop_to_aspect(source, output);
            let got = remaining_aspect(source, crop);
            assert!(
                (got - output).abs() < 0.01,
                "source {source:.3} into frame {output:.3} left {got:.3}, not the frame's shape \
                 ({crop:?})"
            );
        }
    }
}

/// Only one axis is ever touched. Making a wide shot tall is done by taking
/// from the sides; taking from all four edges would lose picture for nothing.
#[test]
fn only_the_axis_that_has_too_much_is_cropped() {
    for source in SHAPES {
        for output in SHAPES {
            let crop = crop_to_aspect(source, output);
            let across = crop.left + crop.right;
            let down = crop.top + crop.bottom;
            assert!(
                across == 0.0 || down == 0.0,
                "source {source:.3} into frame {output:.3} cropped both axes: {crop:?}"
            );
        }
    }
}

/// Centred: the same off each opposite edge.
///
/// Not decoration — an auto crop that took it all off one side would swing
/// every shot to a corner, and the "what is left is the frame's shape" property
/// above would still hold, so it needs saying separately.
#[test]
fn the_crop_is_centred() {
    for source in SHAPES {
        for output in SHAPES {
            let crop = crop_to_aspect(source, output);
            assert!(
                (crop.left - crop.right).abs() < 1e-6,
                "source {source:.3} into frame {output:.3} is off-centre across: {crop:?}"
            );
            assert!(
                (crop.top - crop.bottom).abs() < 1e-6,
                "source {source:.3} into frame {output:.3} is off-centre down: {crop:?}"
            );
        }
    }
}

/// A source already the frame's shape is left entirely alone, so running auto
/// crop on a project that needs none puts nothing in the history.
#[test]
fn a_source_already_the_right_shape_is_not_cropped() {
    for shape in SHAPES {
        assert!(
            crop_to_aspect(shape, shape).is_none(),
            "a {shape:.3} source in a {shape:.3} frame was cropped anyway"
        );
    }
}

/// Which way round the crop runs, stated plainly on the everyday case so the
/// properties above cannot all pass on a version that crops the wrong axis
/// consistently.
#[test]
fn landscape_into_vertical_takes_from_the_sides() {
    let crop = crop_to_aspect(16.0 / 9.0, 9.0 / 16.0);
    assert!(
        crop.left > 0.2 && crop.right > 0.2,
        "a wide shot in a tall frame should lose most of its width: {crop:?}"
    );
    assert_eq!(crop.top, 0.0, "it lost height as well");
    assert_eq!(crop.bottom, 0.0);
}

#[test]
fn vertical_into_landscape_takes_from_the_top_and_bottom() {
    let crop = crop_to_aspect(9.0 / 16.0, 16.0 / 9.0);
    assert!(
        crop.top > 0.2 && crop.bottom > 0.2,
        "a tall shot in a wide frame should lose most of its height: {crop:?}"
    );
    assert_eq!(crop.left, 0.0, "it lost width as well");
    assert_eq!(crop.right, 0.0);
}

/// §50: a shape can be nonsense — a file whose dimensions were never read gives
/// zero, and a division by it gives infinity. No crop is the safe answer.
#[test]
fn a_shape_that_is_not_a_shape_is_no_crop() {
    for (source, output) in [
        (0.0, 1.0),
        (1.0, 0.0),
        (f32::NAN, 1.0),
        (1.0, f32::NAN),
        (f32::INFINITY, 1.0),
        (-1.0, 1.0),
    ] {
        assert!(
            crop_to_aspect(source, output).is_none(),
            "source {source} into frame {output} produced a crop"
        );
    }
}

/// An extreme pairing still leaves a picture. A 21:9 shot in a 9:16 frame keeps
/// under a fifth of its width, and the clamp is what stops the arithmetic
/// running past nothing at all.
#[test]
fn even_the_most_extreme_pairing_leaves_something() {
    let crop = crop_to_aspect(21.0 / 9.0, 9.0 / 16.0);
    let (keep_x, keep_y) = crop.remaining();
    assert!(
        keep_x >= bettercut_timeline::MIN_CROP_REMAINING,
        "nothing left across: {crop:?}"
    );
    assert!(keep_y > 0.0);
}
