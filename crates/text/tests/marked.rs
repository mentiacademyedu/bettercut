//! Drawing some characters in their own colour (`TextRenderer::rasterize_marked`),
//! as a caption lighting each word does. Properties rather than pixels, as in
//! `rasterize.rs`: the font comes from the machine.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::{Mark, Rgba, TextRenderer, TextStyle};

const LIT: Rgba = Rgba::opaque(255, 214, 10);

fn plain() -> TextStyle {
    TextStyle {
        size: 48.0,
        color: Rgba::WHITE,
        stroke: None,
        shadow: None,
        background: None,
        ..TextStyle::default()
    }
}

fn is_lit(p: &[u8]) -> bool {
    p[3] == 255 && p[0] == LIT.r && p[1] == LIT.g && p[2] == LIT.b
}

fn is_white(p: &[u8]) -> bool {
    p[3] == 255 && p[0] == 255 && p[1] == 255 && p[2] == 255
}

/// The marked word is drawn in its colour, the rest stays white, and nothing
/// moves: same size, same coverage.
#[test]
fn the_marked_word_takes_its_colour_and_nothing_moves() {
    let mut renderer = TextRenderer::new();
    let style = plain();
    let text = "one two";
    let whole = renderer.rasterize(text, &style).unwrap();
    let marked = renderer
        .rasterize_marked(
            text,
            &style,
            None,
            Some(Mark {
                chars: 4..7,
                color: LIT,
            }),
        )
        .unwrap();
    assert_eq!((whole.width, whole.height), (marked.width, marked.height));
    let alpha = |pixels: &[u8]| pixels.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
    assert_eq!(
        alpha(&whole.pixels),
        alpha(&marked.pixels),
        "the lit word moved or changed shape"
    );

    let width = marked.width as usize;
    let lit: Vec<usize> = marked
        .pixels
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, p)| is_lit(p))
        .map(|(i, _)| i % width)
        .collect();
    assert!(!lit.is_empty(), "nothing was drawn in the highlight colour");
    // "two" is the second word: every lit pixel is right of the first third.
    assert!(
        lit.iter().all(|&x| x > width / 3),
        "a highlighted pixel sits over the first word"
    );
    let white = marked
        .pixels
        .chunks_exact(4)
        .filter(|p| is_white(p))
        .count();
    let white_before = whole.pixels.chunks_exact(4).filter(|p| is_white(p)).count();
    assert!(white > 0, "the unmarked word lost its colour");
    assert!(white < white_before, "the second word is still white");
}

/// An empty mark is the ordinary picture.
#[test]
fn an_empty_mark_is_the_ordinary_picture() {
    let mut renderer = TextRenderer::new();
    let style = plain();
    let whole = renderer.rasterize("one two", &style).unwrap();
    let empty = renderer
        .rasterize_marked(
            "one two",
            &style,
            None,
            Some(Mark {
                chars: 3..3,
                color: LIT,
            }),
        )
        .unwrap();
    assert_eq!(whole.pixels, empty.pixels);
}
