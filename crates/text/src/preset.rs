//! Text style presets: a title's colour and legibility treatment in one click,
//! without moving or resizing it.
//!
//! [`crate::TitleLook`] is a whole design — font, size *and* where the title
//! sits. These are the other half of what people reach for: "make it pop",
//! "make it glow", "put it in a box", applied to a title that is already the
//! size and in the place the user wants. So a preset replaces only the fill,
//! outline, shadow and box, and keeps the family, size, weight, spacing,
//! alignment and wrap exactly as they were.
//!
//! Outline widths, shadow offsets and box padding scale with the font size, so
//! a preset looks the same on a small caption-sized title as on a huge one —
//! a fixed 6 px outline swamps 30 px text and vanishes on 160 px.

use crate::style::{Background, Rgba, Shadow, Stroke, TextStyle};

/// The size a preset's measurements are designed at; they scale from here.
const DESIGN_SIZE: f32 = 64.0;

/// A named colour-and-legibility treatment for a title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextPreset {
    /// White with a black rim: the default, readable over most footage.
    Classic,
    /// Yellow with a heavy black rim and a hard drop shadow.
    Pop,
    /// Cyan letters with a soft cyan glow and no rim.
    Neon,
    /// White on a dark rounded box.
    Boxed,
    /// Black on a yellow box: the loudest, for watching with the sound off.
    Highlight,
    /// White with a soft shadow and nothing else: quiet, cinematic.
    Soft,
    /// Warm cream letters with a solid offset shadow, like old print.
    Retro,
    /// White on a red box, like a news banner.
    Breaking,
    /// Orange letters glowing like embers.
    Fire,
    /// Pale blue letters with a cold white glow.
    Ice,
    /// White with a heavy black outline and a hard red shadow.
    Comic,
    /// Hot pink with a white outline.
    Candy,
}

impl TextPreset {
    /// Every preset, in the order the interface offers them.
    pub const ALL: [Self; 12] = [
        Self::Classic,
        Self::Pop,
        Self::Neon,
        Self::Boxed,
        Self::Highlight,
        Self::Soft,
        Self::Retro,
        Self::Breaking,
        Self::Fire,
        Self::Ice,
        Self::Comic,
        Self::Candy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Pop => "Pop",
            Self::Neon => "Neon",
            Self::Boxed => "Boxed",
            Self::Highlight => "Highlight",
            Self::Soft => "Soft",
            Self::Retro => "Retro",
            Self::Breaking => "Breaking",
            Self::Fire => "Fire",
            Self::Ice => "Ice",
            Self::Comic => "Comic",
            Self::Candy => "Candy",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Classic => "White with a black outline",
            Self::Pop => "Yellow with a heavy outline and a drop shadow",
            Self::Neon => "Glowing cyan letters",
            Self::Boxed => "White on a dark rounded box",
            Self::Highlight => "Black on a yellow box",
            Self::Soft => "White with a soft shadow",
            Self::Retro => "Cream letters with a solid offset shadow",
            Self::Breaking => "White on a red banner, like breaking news",
            Self::Fire => "Orange letters glowing like embers",
            Self::Ice => "Pale blue letters with a cold glow",
            Self::Comic => "Heavy outline and a hard red shadow, like a comic book",
            Self::Candy => "Hot pink with a white outline",
        }
    }

    /// `style` wearing this preset: fill, outline, shadow and box replaced,
    /// everything about the letters and their layout kept.
    pub fn applied_to(self, style: &TextStyle) -> TextStyle {
        let k = style.size / DESIGN_SIZE;
        let (color, stroke, shadow, background) = match self {
            Self::Classic => (
                Rgba::WHITE,
                Some(Stroke {
                    width: 3.0 * k,
                    color: Rgba::BLACK,
                }),
                None,
                None,
            ),
            Self::Pop => (
                Rgba::opaque(255, 221, 0),
                Some(Stroke {
                    width: 6.0 * k,
                    color: Rgba::BLACK,
                }),
                Some(Shadow {
                    offset_x: 4.0 * k,
                    offset_y: 5.0 * k,
                    blur: 0.0,
                    color: Rgba::BLACK,
                }),
                None,
            ),
            Self::Neon => (
                Rgba::opaque(120, 245, 255),
                None,
                // No offset: a glow spreads evenly around the letters.
                Some(Shadow {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    blur: 18.0 * k,
                    color: Rgba::new(0, 220, 255, 255),
                }),
                None,
            ),
            Self::Boxed => (
                Rgba::WHITE,
                None,
                None,
                Some(Background {
                    color: Rgba::new(0, 0, 0, 190),
                    padding: 16.0 * k,
                    corner_radius: 10.0 * k,
                }),
            ),
            Self::Highlight => (
                Rgba::BLACK,
                None,
                None,
                Some(Background {
                    color: Rgba::opaque(255, 214, 0),
                    padding: 12.0 * k,
                    corner_radius: 4.0 * k,
                }),
            ),
            Self::Soft => (
                Rgba::WHITE,
                None,
                Some(Shadow {
                    offset_x: 0.0,
                    offset_y: 3.0 * k,
                    blur: 12.0 * k,
                    color: Rgba::new(0, 0, 0, 200),
                }),
                None,
            ),
            Self::Retro => (
                Rgba::opaque(255, 236, 200),
                None,
                Some(Shadow {
                    offset_x: 5.0 * k,
                    offset_y: 5.0 * k,
                    blur: 0.0,
                    color: Rgba::opaque(214, 86, 40),
                }),
                None,
            ),
            Self::Breaking => (
                Rgba::WHITE,
                None,
                None,
                Some(Background {
                    color: Rgba::opaque(214, 24, 32),
                    padding: 12.0 * k,
                    corner_radius: 2.0 * k,
                }),
            ),
            Self::Fire => (
                Rgba::opaque(255, 150, 30),
                Some(Stroke {
                    width: 2.0 * k,
                    color: Rgba::opaque(150, 20, 0),
                }),
                Some(Shadow {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    blur: 16.0 * k,
                    color: Rgba::opaque(255, 70, 0),
                }),
                None,
            ),
            Self::Ice => (
                Rgba::opaque(200, 240, 255),
                None,
                Some(Shadow {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    blur: 14.0 * k,
                    color: Rgba::opaque(255, 255, 255),
                }),
                None,
            ),
            Self::Comic => (
                Rgba::WHITE,
                Some(Stroke {
                    width: 7.0 * k,
                    color: Rgba::BLACK,
                }),
                Some(Shadow {
                    offset_x: 6.0 * k,
                    offset_y: 6.0 * k,
                    blur: 0.0,
                    color: Rgba::opaque(230, 30, 40),
                }),
                None,
            ),
            Self::Candy => (
                Rgba::opaque(255, 70, 170),
                Some(Stroke {
                    width: 5.0 * k,
                    color: Rgba::WHITE,
                }),
                None,
                None,
            ),
        };
        TextStyle {
            color,
            stroke,
            shadow,
            background,
            ..style.clone()
        }
    }

    /// The preset `style` is wearing, if it is exactly one of these — so the
    /// interface can light up the current choice, and light up nothing once
    /// the user has tuned a colour by hand.
    pub fn of(style: &TextStyle) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|preset| preset.applied_to(style) == *style)
    }
}
