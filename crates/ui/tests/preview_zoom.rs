//! Zooming and panning the preview (`preview_overlay::preview_canvas`,
//! `PreviewZoom`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_ui::preview_overlay::preview_canvas;
use bettercut_ui::state::PreviewZoom;
use egui::{Rect, pos2, vec2};

fn area() -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0))
}

const HD: (u32, u32) = (1920, 1080);

#[test]
fn fit_letterboxes_the_whole_frame() {
    let (canvas, pan) = preview_canvas(area(), HD, PreviewZoom::Fit, vec2(300.0, 0.0), 1.0);
    assert_eq!(pan, egui::Vec2::ZERO, "a fitted picture kept a pan");
    assert!(area().contains_rect(canvas));
    assert!((canvas.width() / canvas.height() - 16.0 / 9.0).abs() < 0.01);
    assert_eq!(canvas.center(), area().center());
}

/// 100% is one frame pixel to one screen pixel, whatever the display scale;
/// 200% twice that.
#[test]
fn a_scale_is_in_frame_pixels() {
    let (actual, _) = preview_canvas(area(), HD, PreviewZoom::Scale(1.0), vec2(0.0, 0.0), 1.0);
    assert_eq!(actual.size(), vec2(1920.0, 1080.0));
    // On a 150% display each frame pixel is two thirds of a point.
    let (hidpi, _) = preview_canvas(
        area(),
        HD,
        PreviewZoom::Scale(1.0),
        vec2(0.0, 0.0),
        1.0 / 1.5,
    );
    assert!((hidpi.width() - 1280.0).abs() < 0.01);
    let (double, _) = preview_canvas(area(), HD, PreviewZoom::Scale(2.0), vec2(0.0, 0.0), 1.0);
    assert_eq!(double.size(), vec2(3840.0, 2160.0));
    assert_eq!(double.center(), area().center());
}

/// Panning moves the picture, but never so far that the middle of the
/// preview shows nothing.
#[test]
fn a_pan_moves_the_picture_and_is_held_in_view() {
    let (moved, pan) = preview_canvas(area(), HD, PreviewZoom::Scale(1.0), vec2(-200.0, 50.0), 1.0);
    assert_eq!(pan, vec2(-200.0, 50.0));
    assert_eq!(moved.center(), area().center() + pan);

    let (held, pan) = preview_canvas(
        area(),
        HD,
        PreviewZoom::Scale(1.0),
        vec2(-9_000.0, 9_000.0),
        1.0,
    );
    assert_eq!(pan, vec2(-960.0, 540.0), "the pan was not held");
    assert!(held.contains(area().center()) || held.right() == area().center().x);
}

#[test]
fn ctrl_scroll_steps_through_the_scales() {
    assert_eq!(PreviewZoom::Fit.step(true), PreviewZoom::Scale(1.0));
    assert_eq!(PreviewZoom::Fit.step(false), PreviewZoom::Fit);
    assert_eq!(PreviewZoom::Scale(1.0).step(true), PreviewZoom::Scale(2.0));
    assert_eq!(PreviewZoom::Scale(1.0).step(false), PreviewZoom::Scale(0.5));
    assert_eq!(PreviewZoom::Scale(4.0).step(true), PreviewZoom::Scale(4.0));
    assert_eq!(PreviewZoom::Scale(0.25).step(false), PreviewZoom::Fit);
    assert_eq!(PreviewZoom::Scale(2.0).label(), "200%");
}
