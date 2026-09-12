//! What a piece of text looks like (§26).
//!
//! Everything here describes the *bitmap* — the shaping, the colours, the
//! decoration around the glyphs. Where that bitmap ends up on screen, how big
//! it is drawn and how far it is rotated are a [`Transform`] on the clip, not a
//! text property, because they are the same controls every other layer has and
//! §54 keeps one control per idea.
//!
//! [`Transform`]: bettercut_foundation

use serde::{Deserialize, Serialize};

/// A colour as the renderer wants it: sRGB-encoded, straight (not
/// premultiplied) alpha (§21a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const WHITE: Self = Self::opaque(255, 255, 255);
    pub const BLACK: Self = Self::opaque(0, 0, 0);
    pub const TRANSPARENT: Self = Self::new(0, 0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }
}

/// Which family to shape with.
///
/// The three generic names rather than a font picker, for now: a name that is
/// not installed silently falls back to something else, and a title that looks
/// different on the user's machine than it did on ours is exactly what §26
/// warns about. [`Named`] exists because it is how any real font gets used once
/// there are fonts to bundle.
///
/// [`Named`]: FontFamily::Named
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
    Named(String),
}

/// How heavy the strokes are.
///
/// Four steps rather than the full 100–900 axis: the rest are missing from most
/// families, and a weight that silently resolves to the nearest available one
/// is a control that appears to do nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontWeight {
    Light,
    #[default]
    Regular,
    Medium,
    Bold,
}

impl FontWeight {
    pub const ALL: [Self; 4] = [Self::Light, Self::Regular, Self::Medium, Self::Bold];

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Regular => "Regular",
            Self::Medium => "Medium",
            Self::Bold => "Bold",
        }
    }

    /// The OpenType weight axis value.
    pub fn value(self) -> u16 {
        match self {
            Self::Light => 300,
            Self::Regular => 400,
            Self::Medium => 500,
            Self::Bold => 700,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Left,
    #[default]
    Center,
    Right,
}

impl Alignment {
    pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];

    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Center => "Centre",
            Self::Right => "Right",
        }
    }
}

/// An outline around the glyphs (§26).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Half-width in pixels: how far the outline reaches outside the glyph.
    pub width: f32,
    pub color: Rgba,
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            width: 3.0,
            color: Rgba::BLACK,
        }
    }
}

/// A drop shadow (§26).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub offset_x: f32,
    pub offset_y: f32,
    /// Blur radius in pixels. Zero is a hard-edged copy, which is a legitimate
    /// look and not a mistake to guard against.
    pub blur: f32,
    pub color: Rgba,
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 4.0,
            blur: 6.0,
            color: Rgba::new(0, 0, 0, 160),
        }
    }
}

/// A filled box behind the text (§26) — what makes a caption legible over a
/// busy shot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Background {
    pub color: Rgba,
    /// Space between the text and the edge of the box, in pixels.
    pub padding: f32,
    pub corner_radius: f32,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            color: Rgba::new(0, 0, 0, 140),
            padding: 16.0,
            corner_radius: 8.0,
        }
    }
}

/// The largest text we will rasterize, in sequence pixels.
///
/// Not a taste judgement — a bound. The bitmap is allocated at this size, and
/// an unbounded number here is an unbounded allocation driven by a text box.
pub const MAX_SIZE: f32 = 400.0;
pub const MIN_SIZE: f32 = 4.0;

/// A named caption look, for styling a whole lane at once (§27).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptionLook {
    /// White on a dark box: readable over anything, at the cost of covering it.
    Boxed,
    /// Heavy white letters with a black rim, no box.
    Outlined,
    /// Black on yellow — the loudest, for watching with the sound off.
    Highlight,
    /// Smaller and lighter, with a soft shadow: a documentary lower third.
    Soft,
}

impl CaptionLook {
    /// Every look, in the order the interface offers them.
    pub const ALL: [Self; 4] = [Self::Boxed, Self::Outlined, Self::Highlight, Self::Soft];

    /// What to call it in the interface.
    pub fn label(self) -> &'static str {
        match self {
            Self::Boxed => "Boxed",
            Self::Outlined => "Outlined",
            Self::Highlight => "Highlight",
            Self::Soft => "Soft",
        }
    }
}

/// Everything about how a piece of text is drawn (§26).
///
/// Sizes are in **sequence pixels**, not preview pixels: the bitmap is
/// rasterized once at the sequence's own resolution and then scaled like any
/// other layer. That is what makes §46 hold for text — preview and export use
/// the same bitmap, so they cannot disagree about line breaks, and the preview
/// is merely a smaller copy of the picture that will be exported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    #[serde(default)]
    pub family: FontFamily,
    pub size: f32,
    #[serde(default)]
    pub weight: FontWeight,
    #[serde(default)]
    pub italic: bool,
    pub color: Rgba,
    #[serde(default)]
    pub align: Alignment,
    /// Extra space between letters, as a fraction of the font size. Zero is the
    /// spacing the font's designer chose.
    #[serde(default)]
    pub letter_spacing: f32,
    /// Line spacing as a multiple of the font size.
    pub line_height: f32,
    #[serde(default)]
    pub stroke: Option<Stroke>,
    #[serde(default)]
    pub shadow: Option<Shadow>,
    #[serde(default)]
    pub background: Option<Background>,
    /// Longest line before wrapping, in sequence pixels. `None` never wraps,
    /// and the caller is responsible for the text fitting.
    #[serde(default)]
    pub wrap_width: Option<f32>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: FontFamily::default(),
            // Roughly 6% of a 1080p frame's height: large enough to read on a
            // phone, which is where most of this will be watched.
            size: 64.0,
            weight: FontWeight::Bold,
            italic: false,
            color: Rgba::WHITE,
            align: Alignment::Center,
            letter_spacing: 0.0,
            line_height: 1.2,
            // White text over an unknown shot is unreadable about half the
            // time. An outline costs nothing and is what every social editor
            // defaults to.
            stroke: Some(Stroke::default()),
            shadow: None,
            background: None,
            wrap_width: None,
        }
    }
}

impl TextStyle {
    /// What an imported subtitle looks like (§27, Milestone 10).
    ///
    /// Deliberately different from a title. A title is a design element and
    /// wants to be large; a caption is there to be read without being looked
    /// at, so it is smaller, and it carries a background box rather than an
    /// outline — a solid panel is what makes a line legible over footage that
    /// changes underneath it every few frames, which is exactly the case a
    /// subtitle is in and a title usually is not.
    pub fn caption() -> Self {
        Self {
            size: 44.0,
            weight: FontWeight::Medium,
            stroke: None,
            background: Some(Background {
                color: Rgba::new(0, 0, 0, 165),
                padding: 14.0,
                corner_radius: 6.0,
            }),
            // Long lines wrap rather than running off the frame. Two thirds of
            // a 1080-wide canvas, which is about the longest line that stays
            // comfortable to read on a phone.
            wrap_width: Some(720.0),
            ..Self::default()
        }
    }

    /// The named looks a caption lane can wear (§27).
    ///
    /// Captions are read at a glance over footage nobody controls, so every
    /// look here carries its own contrast — a box, an outline or a shadow.
    /// Plain white text is not among them: over a bright shot it disappears,
    /// and a caption that cannot be read is worse than none, because the
    /// viewer knows they are missing something.
    pub fn look(look: CaptionLook) -> Self {
        let caption = Self::caption();
        match look {
            CaptionLook::Boxed => caption,
            // What most social editors default to: heavy letters with a black
            // rim, no box, so the picture is not cut into.
            CaptionLook::Outlined => Self {
                size: 52.0,
                weight: FontWeight::Bold,
                stroke: Some(Stroke {
                    width: 5.0,
                    color: Rgba::BLACK,
                }),
                background: None,
                ..caption
            },
            // Black on yellow: the loudest of the lot, for a phone held at
            // arm's length with the sound off.
            CaptionLook::Highlight => Self {
                size: 54.0,
                weight: FontWeight::Bold,
                color: Rgba::BLACK,
                stroke: None,
                background: Some(Background {
                    color: Rgba::new(255, 214, 0, 255),
                    padding: 12.0,
                    corner_radius: 4.0,
                }),
                ..caption
            },
            // A documentary lower third: smaller, lighter, a soft shadow
            // rather than a rim.
            CaptionLook::Soft => Self {
                size: 40.0,
                weight: FontWeight::Medium,
                stroke: None,
                background: None,
                shadow: Some(Shadow {
                    offset_x: 0.0,
                    offset_y: 3.0,
                    blur: 10.0,
                    color: Rgba::new(0, 0, 0, 200),
                }),
                ..caption
            },
        }
    }

    /// A key identifying the picture this style and text would produce.
    ///
    /// Two clips that say the same thing in the same way *are* the same
    /// bitmap, so the cache is keyed by what was asked for rather than by which
    /// clip asked — which also means re-typing a character and undoing it costs
    /// a lookup rather than a re-shape.
    ///
    /// Floats are hashed by their bits. That makes two values that compare
    /// equal but differ in representation — `0.0` and `-0.0` — miss each other
    /// in the cache, which costs one extra rasterization and never a wrong
    /// picture. The reverse trade would be the dangerous one.
    pub fn key(&self, text: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        text.hash(&mut hasher);
        self.family.hash(&mut hasher);
        self.weight.hash(&mut hasher);
        self.italic.hash(&mut hasher);
        self.color.hash(&mut hasher);
        self.align.hash(&mut hasher);
        for value in [
            self.size,
            self.letter_spacing,
            self.line_height,
            self.wrap_width.unwrap_or(f32::NAN),
        ] {
            value.to_bits().hash(&mut hasher);
        }

        match self.stroke {
            Some(stroke) => {
                stroke.width.to_bits().hash(&mut hasher);
                stroke.color.hash(&mut hasher);
            }
            None => 0u8.hash(&mut hasher),
        }
        match self.shadow {
            Some(shadow) => {
                for value in [shadow.offset_x, shadow.offset_y, shadow.blur] {
                    value.to_bits().hash(&mut hasher);
                }
                shadow.color.hash(&mut hasher);
            }
            None => 1u8.hash(&mut hasher),
        }
        match self.background {
            Some(background) => {
                for value in [background.padding, background.corner_radius] {
                    value.to_bits().hash(&mut hasher);
                }
                background.color.hash(&mut hasher);
            }
            None => 2u8.hash(&mut hasher),
        }

        hasher.finish()
    }

    /// Clamp every number to something that can be rasterized.
    ///
    /// Called by the rasterizer rather than trusted from the caller: this
    /// struct is deserialized from project files, and a hand-edited size of
    /// 1e9 must not become a 1e9-pixel allocation.
    pub fn sanitized(&self) -> Self {
        let mut style = self.clone();
        style.size = style.size.clamp(MIN_SIZE, MAX_SIZE);
        style.line_height = style.line_height.clamp(0.5, 4.0);
        style.letter_spacing = style.letter_spacing.clamp(-0.5, 2.0);
        if let Some(stroke) = &mut style.stroke {
            stroke.width = stroke.width.clamp(0.0, style.size / 4.0);
        }
        if let Some(shadow) = &mut style.shadow {
            shadow.blur = shadow.blur.clamp(0.0, style.size);
            shadow.offset_x = shadow.offset_x.clamp(-style.size, style.size);
            shadow.offset_y = shadow.offset_y.clamp(-style.size, style.size);
        }
        if let Some(background) = &mut style.background {
            background.padding = background.padding.clamp(0.0, style.size * 2.0);
            background.corner_radius = background.corner_radius.clamp(0.0, style.size);
        }
        style.wrap_width = style.wrap_width.map(|w| w.clamp(style.size, 16_384.0));
        style
    }
}

#[cfg(test)]
mod look_tests {
    use super::*;

    /// Every look carries its own contrast.
    ///
    /// A caption is read at a glance over footage nobody controls. Plain
    /// letters with no box, no rim and no shadow vanish over a bright shot,
    /// and a caption that cannot be read is worse than none — the viewer knows
    /// they are missing something.
    #[test]
    fn every_look_can_be_read_over_anything() {
        for look in CaptionLook::ALL {
            let style = TextStyle::look(look);
            assert!(
                style.background.is_some() || style.stroke.is_some() || style.shadow.is_some(),
                "{} is bare letters over the footage",
                look.label()
            );
        }
    }

    /// Four looks that are actually four looks: a picker whose options produce
    /// the same picture is a picker that wastes the user's time.
    #[test]
    fn the_looks_differ_from_each_other() {
        for (index, look) in CaptionLook::ALL.into_iter().enumerate() {
            for other in CaptionLook::ALL.into_iter().skip(index + 1) {
                assert_ne!(
                    TextStyle::look(look),
                    TextStyle::look(other),
                    "{} and {} are the same style",
                    look.label(),
                    other.label()
                );
            }
        }
    }

    /// The lane's own default is one of the offered looks, so the picker can
    /// show what a fresh import is already wearing.
    #[test]
    fn the_caption_default_is_one_of_the_looks() {
        assert_eq!(TextStyle::look(CaptionLook::Boxed), TextStyle::caption());
    }

    /// Wrapping survives every look: a caption that runs off the side of the
    /// frame is unreadable however it is dressed.
    #[test]
    fn every_look_still_wraps() {
        for look in CaptionLook::ALL {
            assert!(
                TextStyle::look(look).wrap_width.is_some(),
                "{} runs off the frame",
                look.label()
            );
        }
    }
}
