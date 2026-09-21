//! A gradient fill (`TextStyle::gradient`): the letters shade from the style's
//! colour at the top to the gradient's at the bottom.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::{Mark, Rgba, TextRenderer, TextStyle};

fn style(gradient: Option<Rgba>) -> TextStyle {
    TextStyle {
        size: 96.0,
        color: Rgba::opaque(255, 0, 0),
        stroke: None,
        shadow: None,
        background: None,
        gradient,
        ..TextStyle::default()
    }
}

/// The average red and blue of fully covered pixels in each row band.
fn band_average(bitmap: &bettercut_text::TextBitmap, rows: std::ops::Range<u32>) -> (f32, f32) {
    let (mut red, mut blue, mut n) = (0.0, 0.0, 0.0);
    for y in rows {
        for x in 0..bitmap.width {
            let i = ((y * bitmap.width + x) * 4) as usize;
            let p = &bitmap.pixels[i..i + 4];
            if p[3] == 255 {
                red += f32::from(p[0]);
                blue += f32::from(p[2]);
                n += 1.0;
            }
        }
    }
    assert!(n > 0.0, "no solid pixels in rows");
    (red / n, blue / n)
}

/// Red at the top of the letters, blue at the bottom, and the same shapes as
/// a flat fill.
#[test]
fn the_letters_shade_from_top_to_bottom() {
    let mut renderer = TextRenderer::new();
    let flat = renderer.rasterize("HIH", &style(None)).unwrap();
    let shaded = renderer
        .rasterize("HIH", &style(Some(Rgba::opaque(0, 0, 255))))
        .unwrap();
    assert_eq!((flat.width, flat.height), (shaded.width, shaded.height));
    let alpha = |pixels: &[u8]| pixels.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
    assert_eq!(
        alpha(&flat.pixels),
        alpha(&shaded.pixels),
        "the shapes changed"
    );

    let h = shaded.height;
    let (top_red, top_blue) = band_average(&shaded, h / 4..h * 2 / 5);
    let (bottom_red, bottom_blue) = band_average(&shaded, h * 3 / 5..h * 3 / 4);
    assert!(
        top_red > bottom_red + 40.0,
        "red does not fade: {top_red} → {bottom_red}"
    );
    assert!(
        bottom_blue > top_blue + 40.0,
        "blue does not rise: {top_blue} → {bottom_blue}"
    );

    let (flat_top, _) = band_average(&flat, h / 4..h * 2 / 5);
    let (flat_bottom, _) = band_average(&flat, h * 3 / 5..h * 3 / 4);
    assert_eq!(
        (flat_top, flat_bottom),
        (255.0, 255.0),
        "a flat fill shaded"
    );
}

/// A gradient is part of the style's identity: the cache must not hand back
/// the flat picture for a shaded title.
#[test]
fn a_gradient_changes_the_style_key() {
    assert_ne!(
        style(None).key("HIH"),
        style(Some(Rgba::opaque(0, 0, 255))).key("HIH")
    );
    assert_ne!(
        style(Some(Rgba::opaque(0, 0, 255))).key("HIH"),
        style(Some(Rgba::opaque(0, 255, 0))).key("HIH")
    );
}

/// A lit word keeps its highlight colour over a gradient.
#[test]
fn a_highlight_wins_over_the_gradient() {
    let mut renderer = TextRenderer::new();
    let lit = Rgba::opaque(10, 250, 10);
    let bitmap = renderer
        .rasterize_marked(
            "HI HI",
            &style(Some(Rgba::opaque(0, 0, 255))),
            None,
            Some(Mark {
                chars: 0..2,
                color: lit,
            }),
        )
        .unwrap();
    let lit_pixels = bitmap
        .pixels
        .chunks_exact(4)
        .filter(|p| p[3] == 255 && p[0] == lit.r && p[1] == lit.g && p[2] == lit.b)
        .count();
    assert!(lit_pixels > 0, "the highlight was shaded away");
}
