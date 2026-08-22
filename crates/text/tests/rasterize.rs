//! Shaping and rasterization end to end (§26).
//!
//! These assert *properties* rather than exact pixels, because the font that
//! shapes the text comes from the machine. A golden bitmap would encode which
//! version of which system font happened to be installed on the machine that
//! recorded it, and would fail on every other one — the §51.1 golden frames
//! that cover text belong in the renderer, against a bundled font.
//!
//! What can be pinned down without knowing the font: that ink appears, that it
//! grows with the size, that decorations reach outside the glyphs, that
//! alignment moves lines, and that the bitmap is well formed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::style::{Background, Shadow, Stroke};
use bettercut_text::{Alignment, FontWeight, Rgba, TextError, TextRenderer, TextStyle};

/// One renderer for the whole file. Building a `FontSystem` reads the machine's
/// font directories, which is far and away the slowest thing here.
fn renderer() -> TextRenderer {
    TextRenderer::new()
}

fn plain(size: f32) -> TextStyle {
    TextStyle {
        size,
        stroke: None,
        ..TextStyle::default()
    }
}

/// Pixels with any coverage at all.
fn ink(bitmap: &bettercut_text::TextBitmap) -> usize {
    bitmap.pixels.chunks_exact(4).filter(|p| p[3] > 0).count()
}

#[test]
fn text_rasterizes_to_something_visible() {
    let mut renderer = renderer();
    assert!(renderer.font_count() > 0, "no fonts on this machine");

    let bitmap = renderer.rasterize("Hello", &plain(48.0)).unwrap();

    assert!(bitmap.width > 0 && bitmap.height > 0);
    assert_eq!(
        bitmap.pixels.len(),
        (bitmap.width * bitmap.height * 4) as usize,
        "the buffer does not match the dimensions it claims"
    );
    assert!(ink(&bitmap) > 0, "the bitmap is completely transparent");
}

/// Whatever the font, twice the size is a bigger picture. A size control that
/// silently did nothing would still produce a valid bitmap.
#[test]
fn larger_text_makes_a_larger_bitmap() {
    let mut renderer = renderer();
    let small = renderer.rasterize("Hello", &plain(24.0)).unwrap();
    let large = renderer.rasterize("Hello", &plain(72.0)).unwrap();

    assert!(
        large.width > small.width && large.height > small.height,
        "{}×{} was not larger than {}×{}",
        large.width,
        large.height,
        small.width,
        small.height
    );
}

#[test]
fn more_text_makes_a_wider_bitmap() {
    let mut renderer = renderer();
    let short = renderer.rasterize("Hi", &plain(48.0)).unwrap();
    let long = renderer
        .rasterize("Hi there, this is much longer", &plain(48.0))
        .unwrap();

    assert!(long.width > short.width);
    assert_eq!(
        long.height, short.height,
        "one line of text grew taller when it got longer"
    );
}

/// A newline is a second line, not a space.
#[test]
fn a_second_line_makes_a_taller_bitmap() {
    let mut renderer = renderer();
    let one = renderer.rasterize("Hello", &plain(48.0)).unwrap();
    let two = renderer.rasterize("Hello\nthere", &plain(48.0)).unwrap();

    assert!(
        two.height > one.height,
        "a second line did not add any height"
    );
}

/// Line spacing is one of §26's required controls, and the only way to see it
/// is that the same two lines occupy more room.
#[test]
fn line_spacing_changes_the_height() {
    let mut renderer = renderer();
    let tight = TextStyle {
        line_height: 1.0,
        ..plain(48.0)
    };
    let loose = TextStyle {
        line_height: 2.0,
        ..plain(48.0)
    };

    let tight = renderer.rasterize("Hello\nthere", &tight).unwrap();
    let loose = renderer.rasterize("Hello\nthere", &loose).unwrap();
    assert!(loose.height > tight.height);
}

/// Letter spacing widens the line without changing what is on it.
#[test]
fn letter_spacing_widens_the_line() {
    let mut renderer = renderer();
    let normal = renderer.rasterize("Hello", &plain(48.0)).unwrap();
    let tracked = renderer
        .rasterize(
            "Hello",
            &TextStyle {
                letter_spacing: 0.5,
                ..plain(48.0)
            },
        )
        .unwrap();

    assert!(
        tracked.width > normal.width,
        "letter spacing of 0.5 em changed nothing"
    );
}

/// Bold is heavier than light, in the only way a test can see: more ink for the
/// same string at the same size.
///
/// Skipped rather than failed when the machine's default family has no separate
/// weights — that is a fact about the machine, not about this code.
#[test]
fn a_bolder_weight_puts_down_more_ink() {
    let mut renderer = renderer();
    let light = renderer
        .rasterize(
            "Hello",
            &TextStyle {
                weight: FontWeight::Light,
                ..plain(64.0)
            },
        )
        .unwrap();
    let bold = renderer
        .rasterize(
            "Hello",
            &TextStyle {
                weight: FontWeight::Bold,
                ..plain(64.0)
            },
        )
        .unwrap();

    if light.pixels == bold.pixels {
        eprintln!("skipped: this machine's sans-serif family has one weight");
        return;
    }
    assert!(
        ink(&bold) > ink(&light),
        "bold ({}) was not heavier than light ({})",
        ink(&bold),
        ink(&light)
    );
}

/// The fill colour is the colour that comes out. Checked on the most-covered
/// pixel, which is the one furthest from any antialiased edge.
#[test]
fn the_fill_colour_is_used() {
    let mut renderer = renderer();
    let bitmap = renderer
        .rasterize(
            "Hello",
            &TextStyle {
                color: Rgba::opaque(255, 0, 0),
                ..plain(64.0)
            },
        )
        .unwrap();

    let solid = bitmap
        .pixels
        .chunks_exact(4)
        .find(|p| p[3] == 255)
        .expect("no fully opaque pixel");
    assert_eq!((solid[0], solid[1], solid[2]), (255, 0, 0));
}

/// A stroke reaches outside the glyphs, so the bitmap has to grow to hold it —
/// and the outline colour has to actually appear.
#[test]
fn a_stroke_surrounds_the_text() {
    let mut renderer = renderer();
    let bare = renderer.rasterize("Hello", &plain(64.0)).unwrap();
    let outlined = renderer
        .rasterize(
            "Hello",
            &TextStyle {
                color: Rgba::WHITE,
                stroke: Some(Stroke {
                    width: 6.0,
                    color: Rgba::opaque(0, 0, 255),
                }),
                ..plain(64.0)
            },
        )
        .unwrap();

    assert!(
        outlined.width > bare.width && outlined.height > bare.height,
        "the bitmap did not grow to hold the outline"
    );
    assert!(
        ink(&outlined) > ink(&bare),
        "the outline added no coverage at all"
    );

    let blue = outlined
        .pixels
        .chunks_exact(4)
        .any(|p| p[2] > 200 && p[0] < 80 && p[3] > 200);
    assert!(blue, "the stroke colour never appears in the bitmap");
}

/// A shadow is offset and soft, so it puts coverage where the glyphs are not.
#[test]
fn a_shadow_falls_outside_the_glyphs() {
    let mut renderer = renderer();
    let bare = renderer.rasterize("Hello", &plain(64.0)).unwrap();
    let shadowed = renderer
        .rasterize(
            "Hello",
            &TextStyle {
                shadow: Some(Shadow {
                    offset_x: 0.0,
                    offset_y: 10.0,
                    blur: 8.0,
                    color: Rgba::BLACK,
                }),
                ..plain(64.0)
            },
        )
        .unwrap();

    assert!(shadowed.height > bare.height, "no room made for the shadow");
    assert!(ink(&shadowed) > ink(&bare));

    // Soft: a shadow that was all-or-nothing would mean the blur did not run.
    let partial = shadowed
        .pixels
        .chunks_exact(4)
        .filter(|p| (16..200).contains(&p[3]))
        .count();
    assert!(partial > 0, "every shadow pixel is fully on or fully off");
}

/// The background is the one decoration that fills solid area rather than
/// tracing the glyphs, and it must sit *behind* them.
#[test]
fn a_background_fills_the_box_behind_the_text() {
    let mut renderer = renderer();
    let boxed = renderer
        .rasterize(
            "Hello",
            &TextStyle {
                color: Rgba::opaque(255, 255, 255),
                background: Some(Background {
                    color: Rgba::opaque(255, 0, 0),
                    padding: 20.0,
                    corner_radius: 0.0,
                }),
                ..plain(64.0)
            },
        )
        .unwrap();

    // A corner is inside the padding but well away from any letter.
    let corner = boxed.pixel(3, 3);
    assert_eq!(
        (corner.r, corner.g, corner.b, corner.a),
        (255, 0, 0, 255),
        "the background does not cover the padding"
    );

    let white = boxed
        .pixels
        .chunks_exact(4)
        .any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200);
    assert!(white, "the text was buried under its own background");
}

/// Rounded corners round. Without this the radius could be ignored entirely and
/// every other background test would still pass.
#[test]
fn a_rounded_background_leaves_its_corners_clear() {
    let mut renderer = renderer();
    let style = |corner_radius| TextStyle {
        background: Some(Background {
            color: Rgba::opaque(255, 0, 0),
            padding: 24.0,
            corner_radius,
        }),
        ..plain(64.0)
    };

    let square = renderer.rasterize("Hello", &style(0.0)).unwrap();
    let rounded = renderer.rasterize("Hello", &style(20.0)).unwrap();

    // Just inside the box's own corner: solid when square, cut away when the
    // corner has a 20-pixel radius.
    assert_eq!(
        square.pixel(3, 3).a,
        255,
        "the square box has a soft corner"
    );
    assert_eq!(
        rounded.pixel(3, 3).a,
        0,
        "the rounded box filled its corner anyway"
    );
}

/// Alignment moves the shorter line, which is only visible when there are two
/// of unequal length.
#[test]
fn alignment_moves_the_shorter_line() {
    let mut renderer = renderer();
    let text = "A very long first line\nshort";

    let left = renderer
        .rasterize(
            text,
            &TextStyle {
                align: Alignment::Left,
                ..plain(48.0)
            },
        )
        .unwrap();
    let right = renderer
        .rasterize(
            text,
            &TextStyle {
                align: Alignment::Right,
                ..plain(48.0)
            },
        )
        .unwrap();

    assert_eq!(
        (left.width, left.height),
        (right.width, right.height),
        "alignment changed the size of the bitmap"
    );
    assert_ne!(
        left.pixels, right.pixels,
        "left and right alignment produced identical pictures"
    );
}

/// Wrapping is what makes a long caption usable at all.
#[test]
fn a_wrap_width_breaks_the_line() {
    let mut renderer = renderer();
    let text = "This sentence is long enough that it has to break somewhere";

    let unwrapped = renderer.rasterize(text, &plain(32.0)).unwrap();
    let wrapped = renderer
        .rasterize(
            text,
            &TextStyle {
                wrap_width: Some(300.0),
                ..plain(32.0)
            },
        )
        .unwrap();

    assert!(wrapped.width < unwrapped.width, "the line did not wrap");
    assert!(wrapped.height > unwrapped.height, "no extra lines appeared");
}

/// Empty and whitespace-only text is not an error to report to the user — there
/// is simply no layer to draw. The caller needs to be able to tell.
#[test]
fn empty_text_produces_no_bitmap() {
    let mut renderer = renderer();
    assert!(matches!(
        renderer.rasterize("", &plain(48.0)),
        Err(TextError::Empty)
    ));
    assert!(matches!(
        renderer.rasterize("   \n  ", &plain(48.0)),
        Err(TextError::Empty)
    ));
}

/// A project file can be edited by hand, and a size of a million must not
/// become a million-pixel allocation.
#[test]
fn an_absurd_size_is_clamped_rather_than_allocated() {
    let mut renderer = renderer();
    let bitmap = renderer
        .rasterize(
            "Hello",
            &TextStyle {
                size: 1.0e9,
                ..plain(48.0)
            },
        )
        .unwrap();

    assert!(
        bitmap.width < 8192 && bitmap.height < 8192,
        "clamping did not stop a {}×{} allocation",
        bitmap.width,
        bitmap.height
    );
}

/// §46: the same text and style must give the same picture every time, or
/// preview and export would disagree between two runs of the same code.
#[test]
fn rasterizing_twice_gives_the_same_picture() {
    let mut renderer = renderer();
    let style = TextStyle {
        stroke: Some(Stroke::default()),
        shadow: Some(Shadow::default()),
        background: Some(Background::default()),
        ..plain(56.0)
    };

    let first = renderer.rasterize("Repeatable", &style).unwrap();
    let second = renderer.rasterize("Repeatable", &style).unwrap();
    assert_eq!(first, second);

    // And from a renderer that has never seen this text before, so nothing
    // depends on the glyph cache being warm.
    let cold = TextRenderer::new().rasterize("Repeatable", &style).unwrap();
    assert_eq!(first, cold, "a warm cache changed the picture");
}
