//! Curved text (`TextStyle::curve`, `raster::arc`): the finished text bent
//! round an arch or a smile.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::TextBitmap;
use bettercut_text::raster::arc;

/// A 200 × 30 strip, a solid bar across its middle rows: a stand-in for a line
/// of text that needs no fonts.
fn strip() -> TextBitmap {
    let (width, height) = (200_u32, 30_u32);
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for y in 10..20 {
        for x in 0..width {
            let at = ((y * width + x) * 4) as usize;
            pixels[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    TextBitmap {
        width,
        height,
        pixels,
    }
}

/// The first row with ink in column `x`, from the top.
fn top_ink(bitmap: &TextBitmap, x: u32) -> Option<u32> {
    (0..bitmap.height).find(|y| bitmap.pixel(x, *y).a > 128)
}

fn ink_columns(bitmap: &TextBitmap) -> (u32, u32) {
    let has = |x: u32| (0..bitmap.height).any(|y| bitmap.pixel(x, y).a > 128);
    let first = (0..bitmap.width).find(|x| has(*x)).unwrap();
    let last = (0..bitmap.width).rev().find(|x| has(*x)).unwrap();
    (first, last)
}

#[test]
fn straight_is_left_alone() {
    let flat = strip();
    let same = arc(strip(), 0.0);
    assert_eq!((same.width, same.height), (flat.width, flat.height));
    assert_eq!(same.pixels, flat.pixels);
}

/// Arched up: the middle is higher than the ends, and the picture grows taller
/// to hold the bend.
#[test]
fn a_positive_curve_arches_up() {
    let bent = arc(strip(), 0.5);
    assert!(bent.height > 30, "no room was made for the bend");
    let (first, last) = ink_columns(&bent);
    let middle = (first + last) / 2;
    let centre = top_ink(&bent, middle).unwrap();
    let end = top_ink(&bent, first + 3).unwrap();
    assert!(centre + 10 < end, "middle {centre}, end {end}");
}

/// Sagging: the ends are higher than the middle.
#[test]
fn a_negative_curve_sags() {
    let bent = arc(strip(), -0.5);
    let (first, last) = ink_columns(&bent);
    let middle = (first + last) / 2;
    let centre = top_ink(&bent, middle).unwrap();
    let end = top_ink(&bent, last - 3).unwrap();
    assert!(end + 10 < centre, "middle {centre}, end {end}");
}

/// The bend keeps the text's middle at its length along the arc, so the picture
/// is never much wider than the straight line — only the outside of the bend,
/// a text's height further out, spreads a little.
#[test]
fn bending_does_not_stretch_the_line() {
    for curve in [0.2, 0.6, 1.0, -1.0] {
        let bent = arc(strip(), curve);
        assert!(bent.width <= 200 + 30, "{curve}: {} wide", bent.width);
    }
}

/// Through the renderer: a curved title is its own cached picture, and taller
/// than the straight one.
#[test]
fn a_curved_title_is_drawn_bent() {
    use bettercut_text::{TextRenderer, TextStyle};
    let mut renderer = TextRenderer::new();
    let straight = TextStyle::default();
    let curved = TextStyle {
        curve: 0.5,
        ..TextStyle::default()
    };
    assert_ne!(straight.key("ROUND WE GO"), curved.key("ROUND WE GO"));
    let flat = renderer.rasterize("ROUND WE GO", &straight).unwrap();
    let bent = renderer.rasterize("ROUND WE GO", &curved).unwrap();
    assert!(
        bent.height > flat.height + 20,
        "{} vs {}",
        bent.height,
        flat.height
    );
}
