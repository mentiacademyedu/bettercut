//! Shapes, drawn (`bettercut_text::shape`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::{MAX_SHAPE_SIDE, Rgba, Shape, ShapeKind, Stroke, TextBitmap};

fn alpha(bitmap: &TextBitmap, x: u32, y: u32) -> u8 {
    bitmap.pixel(x, y).a
}

fn coverage(bitmap: &TextBitmap) -> f64 {
    bitmap
        .pixels
        .chunks_exact(4)
        .map(|p| f64::from(p[3]) / 255.0)
        .sum::<f64>()
        / f64::from(bitmap.width * bitmap.height)
}

fn shape(kind: ShapeKind, width: f32, height: f32) -> Shape {
    Shape {
        width,
        height,
        fill: Rgba::opaque(200, 40, 60),
        ..Shape::new(kind)
    }
}

/// A rectangle is its size, solid to the edge, in its fill colour.
#[test]
fn a_rectangle_fills_its_whole_size() {
    let bitmap = shape(ShapeKind::Rectangle, 120.0, 40.0).rasterize();
    assert_eq!((bitmap.width, bitmap.height), (120, 40));
    assert!(
        bitmap
            .pixels
            .chunks_exact(4)
            .all(|p| p == [200, 40, 60, 255])
    );
}

/// An ellipse covers a quarter-circle's share of its box — π/4 — with its
/// centre solid, its corners empty, and soft edges rather than a staircase.
#[test]
fn an_ellipse_is_an_ellipse() {
    let bitmap = shape(ShapeKind::Ellipse, 200.0, 100.0).rasterize();
    let share = coverage(&bitmap);
    assert!(
        (share - std::f64::consts::FRAC_PI_4).abs() < 0.01,
        "covers {share}"
    );
    assert_eq!(alpha(&bitmap, 100, 50), 255);
    assert_eq!(alpha(&bitmap, 0, 0), 0);
    assert_eq!(alpha(&bitmap, 199, 99), 0);
    let soft = bitmap
        .pixels
        .chunks_exact(4)
        .filter(|p| (10..245).contains(&p[3]))
        .count();
    assert!(
        soft > 50,
        "the edge is not anti-aliased: {soft} partial pixels"
    );
}

/// Rounded corners are cut away; the straight edges between them are not.
#[test]
fn a_rounded_rectangle_loses_only_its_corners() {
    let bitmap = Shape {
        corner_radius: 20.0,
        ..shape(ShapeKind::RoundedRectangle, 160.0, 80.0)
    }
    .rasterize();
    assert_eq!(alpha(&bitmap, 0, 0), 0, "the corner was not rounded");
    assert_eq!(alpha(&bitmap, 80, 0), 255, "the top edge was cut");
    assert_eq!(alpha(&bitmap, 0, 40), 255, "the left edge was cut");
}

/// An outline sits inside the edge in its own colour; the middle keeps the fill.
#[test]
fn an_outline_rings_the_fill() {
    let bitmap = Shape {
        outline: Some(Stroke {
            width: 6.0,
            color: Rgba::opaque(255, 255, 255),
        }),
        ..shape(ShapeKind::Rectangle, 100.0, 60.0)
    }
    .rasterize();
    let edge = bitmap.pixel(2, 30);
    assert_eq!(
        (edge.r, edge.g, edge.b),
        (255, 255, 255),
        "no outline at the edge"
    );
    let middle = bitmap.pixel(50, 30);
    assert_eq!(
        (middle.r, middle.g, middle.b),
        (200, 40, 60),
        "the outline reached the middle"
    );
}

/// Whatever a project file says, the bitmap is a sane size.
#[test]
fn absurd_sizes_are_brought_into_range() {
    let tiny = shape(ShapeKind::Rectangle, f32::NAN, -5.0).rasterize();
    assert_eq!((tiny.width, tiny.height), (1, 1));
    let huge = shape(ShapeKind::Rectangle, 1e9, 2.0).rasterize();
    assert_eq!(huge.width, MAX_SHAPE_SIDE as u32);
}

/// The cache key follows every field that changes the picture.
#[test]
fn the_key_changes_with_the_picture() {
    let base = Shape::new(ShapeKind::Rectangle);
    assert_eq!(base.key(), Shape::new(ShapeKind::Rectangle).key());
    let variants = [
        Shape {
            kind: ShapeKind::Ellipse,
            ..base.clone()
        },
        Shape {
            width: 481.0,
            ..base.clone()
        },
        Shape {
            height: 271.0,
            ..base.clone()
        },
        Shape {
            fill: Rgba::BLACK,
            ..base.clone()
        },
        Shape {
            corner_radius: 3.0,
            ..base.clone()
        },
        Shape {
            outline: Some(Stroke::default()),
            ..base.clone()
        },
    ];
    for variant in variants {
        assert_ne!(
            variant.key(),
            base.key(),
            "{variant:?} shares a key with the plain shape"
        );
    }
}
