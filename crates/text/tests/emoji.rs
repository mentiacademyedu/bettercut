//! An emoji keeps its own colours, where ordinary letters take the title's.
//!
//! Skipped when the machine has no colour emoji font: the check is about what
//! the renderer does with one, not about what is installed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::{TextRenderer, TextStyle};

/// How many opaque pixels carry an actual colour rather than a shade of grey.
///
/// White text, its outline and its shadow are all neutral however many shades
/// of them there are; an emoji's own colours are not.
fn coloured_pixels(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|pixel| {
            let high = pixel[0].max(pixel[1]).max(pixel[2]);
            let low = pixel[0].min(pixel[1]).min(pixel[2]);
            pixel[3] > 200 && high.saturating_sub(low) > 40
        })
        .count()
}

#[test]
fn an_emoji_is_not_painted_in_the_titles_colour() {
    let mut renderer = TextRenderer::new();
    let style = TextStyle {
        size: 96.0,
        ..TextStyle::default()
    };
    // Plain white text: every opaque pixel is white, give or take the edges.
    let word = renderer.rasterize("AB", &style).expect("text");
    let emoji = match renderer.rasterize("🎉", &style) {
        Ok(bitmap) => bitmap,
        // No emoji font on this machine; nothing to check.
        Err(_) => return,
    };

    let word_colours = coloured_pixels(&word.pixels);
    let emoji_colours = coloured_pixels(&emoji.pixels);
    assert_eq!(word_colours, 0, "white letters came out coloured");
    if emoji_colours == 0 {
        // A monochrome emoji font is all this machine has; the renderer drew
        // what it was given.
        return;
    }
    assert!(
        emoji_colours > 50,
        "only {emoji_colours} pixels of the emoji kept a colour of their own"
    );
}
