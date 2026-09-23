//! Shrink to fit (`TextStyle::shrink_to_fit`): a line longer than its width
//! comes down in size until it fits, instead of wrapping or running off.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::{TextRenderer, TextStyle};

fn style(size: f32, wrap: Option<f32>, shrink: bool) -> TextStyle {
    TextStyle {
        size,
        wrap_width: wrap,
        shrink_to_fit: shrink,
        ..TextStyle::default()
    }
}

const LONG: &str = "A name far too long for the space it has been given";

/// The line fits the width on one line, smaller.
#[test]
fn a_long_line_shrinks_onto_one_line_within_its_width() {
    let mut renderer = TextRenderer::new();
    let natural = renderer.rasterize(LONG, &style(48.0, None, false)).unwrap();
    assert!(
        natural.width > 400,
        "setup: the line is not long: {}",
        natural.width
    );

    let fitted = renderer
        .rasterize(LONG, &style(48.0, Some(300.0), true))
        .unwrap();
    // The bitmap carries a little padding round the text; the text itself
    // is what fits the width.
    assert!(
        fitted.width <= 312,
        "did not fit the width: {}",
        fitted.width
    );
    assert!(
        fitted.height < natural.height * 3 / 2,
        "it wrapped instead of shrinking: {} against {}",
        fitted.height,
        natural.height
    );
}

/// Without the flag the same width wraps, as it always did.
#[test]
fn without_the_flag_the_line_wraps() {
    let mut renderer = TextRenderer::new();
    let natural = renderer.rasterize(LONG, &style(48.0, None, false)).unwrap();
    let wrapped = renderer
        .rasterize(LONG, &style(48.0, Some(300.0), false))
        .unwrap();
    assert!(wrapped.width <= 302);
    assert!(
        wrapped.height > natural.height * 3 / 2,
        "it did not wrap: {} against {}",
        wrapped.height,
        natural.height
    );
}

/// A line that already fits is left exactly as it was.
#[test]
fn a_short_line_is_untouched() {
    let mut renderer = TextRenderer::new();
    let plain = renderer
        .rasterize("Hi", &style(48.0, Some(300.0), false))
        .unwrap();
    let fitted = renderer
        .rasterize("Hi", &style(48.0, Some(300.0), true))
        .unwrap();
    assert_eq!((plain.width, plain.height), (fitted.width, fitted.height));
}

/// And it never shrinks below the smallest readable size.
#[test]
fn it_stops_at_the_smallest_readable_size() {
    let mut renderer = TextRenderer::new();
    let tiny = renderer
        .rasterize(LONG, &style(48.0, Some(10.0), true))
        .unwrap();
    let floor = renderer
        .rasterize(LONG, &style(bettercut_text::MIN_FIT_SIZE, None, false))
        .unwrap();
    assert!(
        (i64::from(tiny.width) - i64::from(floor.width)).abs() <= 4,
        "went below the floor: {} against {}",
        tiny.width,
        floor.width
    );
}
