//! Text style presets (`bettercut_text::preset`).
//!
//! The promise of a preset is narrow and easy to break: it changes how the
//! letters are coloured and set off from the picture, and nothing about their
//! size, font or layout. And it has to be worth a button — every preset must
//! actually draw differently, and be readable over footage.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::{
    Alignment, FontFamily, FontWeight, Rgba, TextPreset, TextRenderer, TextStyle,
};

/// A title someone has already set up: not the defaults, so a preset that
/// quietly reset the layout would show.
fn arranged(size: f32) -> TextStyle {
    TextStyle {
        family: FontFamily::Serif,
        size,
        weight: FontWeight::Light,
        italic: true,
        align: Alignment::Right,
        letter_spacing: 0.1,
        line_height: 1.5,
        wrap_width: Some(800.0),
        ..TextStyle::default()
    }
}

/// Everything about the letters and their layout survives a preset.
#[test]
fn a_preset_keeps_the_font_size_and_layout() {
    let before = arranged(80.0);
    for preset in TextPreset::ALL {
        let after = preset.applied_to(&before);
        assert_eq!(after.family, before.family, "{preset:?}");
        assert_eq!(after.size, before.size, "{preset:?}");
        assert_eq!(after.weight, before.weight, "{preset:?}");
        assert_eq!(after.italic, before.italic, "{preset:?}");
        assert_eq!(after.align, before.align, "{preset:?}");
        assert_eq!(after.letter_spacing, before.letter_spacing, "{preset:?}");
        assert_eq!(after.line_height, before.line_height, "{preset:?}");
        assert_eq!(after.wrap_width, before.wrap_width, "{preset:?}");
    }
}

/// A preset is recognised on the style it produced, whatever the size, and is
/// no longer claimed once a colour has been tuned by hand. Applying it twice
/// changes nothing.
#[test]
fn the_current_preset_is_recognised_until_it_is_tuned() {
    for size in [30.0, 64.0, 160.0] {
        for preset in TextPreset::ALL {
            let styled = preset.applied_to(&arranged(size));
            assert_eq!(
                TextPreset::of(&styled),
                Some(preset),
                "{preset:?} at {size}"
            );
            assert_eq!(preset.applied_to(&styled), styled);

            let tuned = TextStyle {
                color: Rgba::opaque(1, 2, 3),
                ..styled
            };
            assert_eq!(TextPreset::of(&tuned), None, "{preset:?} still claimed");
        }
    }
}

/// A new title is already wearing Classic, so the row shows a choice from the
/// start rather than nothing.
#[test]
fn a_new_title_is_classic() {
    assert_eq!(
        TextPreset::of(&TextStyle::default()),
        Some(TextPreset::Classic)
    );
}

/// Decorations scale with the text: a preset on huge text gets a proportionally
/// thick outline, not a hairline.
#[test]
fn decorations_scale_with_the_font_size() {
    let small = TextPreset::Pop.applied_to(&arranged(32.0));
    let large = TextPreset::Pop.applied_to(&arranged(128.0));
    let (s, l) = (small.stroke.unwrap().width, large.stroke.unwrap().width);
    assert!(
        (l / s - 4.0).abs() < 1e-3,
        "outline {s} at 32 px, {l} at 128 px"
    );
}

/// Every preset carries its own contrast — an outline, a shadow or a box —
/// because plain coloured letters vanish over a shot of the same colour.
#[test]
fn every_preset_can_be_read_over_footage() {
    for preset in TextPreset::ALL {
        let style = preset.applied_to(&arranged(64.0));
        assert!(
            style.stroke.is_some() || style.shadow.is_some() || style.background.is_some(),
            "{preset:?} is bare letters"
        );
    }
}

/// Drawn, every preset is a different picture: no two buttons that do the same
/// thing.
#[test]
fn every_preset_draws_differently() {
    let mut renderer = TextRenderer::new();
    let bitmaps: Vec<_> = TextPreset::ALL
        .into_iter()
        .map(|preset| {
            (
                preset,
                renderer
                    .rasterize("Hello", &preset.applied_to(&TextStyle::default()))
                    .unwrap(),
            )
        })
        .collect();
    for (i, (a, first)) in bitmaps.iter().enumerate() {
        for (b, second) in &bitmaps[i + 1..] {
            assert_ne!(first, second, "{a:?} and {b:?} draw the same picture");
        }
    }
}

/// The list the interface shows is every preset there is.
#[test]
fn every_preset_is_offered() {
    fn offered(preset: TextPreset) -> bool {
        match preset {
            TextPreset::Classic
            | TextPreset::Pop
            | TextPreset::Neon
            | TextPreset::Boxed
            | TextPreset::Highlight
            | TextPreset::Soft
            | TextPreset::Retro => true,
        }
    }
    assert_eq!(TextPreset::ALL.len(), 7, "a preset was added or removed");
    assert!(TextPreset::ALL.into_iter().all(offered));
    let mut labels: Vec<_> = TextPreset::ALL.iter().map(|p| p.label()).collect();
    labels.dedup();
    assert_eq!(labels.len(), 7);
}
