//! Filters: one-click looks for a clip.
//!
//! A filter is not a new kind of effect — it is a set of the colour and
//! texture controls a clip already has, dialled to a recognisable look and
//! applied together, the way a phone editor's filter row works. So a filtered
//! clip is still fully adjustable: pick "Vintage", then pull the warmth back
//! by hand. And applying one is exactly a Paste Look (one undo step, onto every
//! selected clip), which needs no machinery of its own.
//!
//! Only the controls a filter is about are set. Framing, opacity, masks, keys
//! and the like are the clip's own and are never touched by choosing a look.

use bettercut_foundation::ClipId;
use bettercut_timeline::curves::Tone;

use crate::command::ClipProperty;
use crate::editor::Editor;
use crate::error::EditorError;

/// A named look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    /// Back to the clip as shot: every control a filter sets, at its default.
    Original,
    BlackAndWhite,
    Vintage,
    Cinematic,
    Warm,
    Cool,
    Vivid,
    Faded,
    Vhs,
    Dream,
    Sepia,
    Duotone,
    Noir,
    Golden,
    Night,
    Food,
    Film,
    Pastel,
    Cyber,
    Matrix,
}

/// The settings a filter sets, in the units the Inspector shows.
struct Recipe {
    brightness: f32,
    contrast: f32,
    saturation: f32,
    temperature: f32,
    tint: f32,
    sharpen: f32,
    blur: f32,
    rgb_split: f32,
    tone: Tone,
}

const NEUTRAL: Recipe = Recipe {
    brightness: 1.0,
    contrast: 1.0,
    saturation: 1.0,
    temperature: 0.0,
    tint: 0.0,
    sharpen: 0.0,
    blur: 0.0,
    rgb_split: 0.0,
    tone: Tone::None,
};

impl Filter {
    /// Every filter, in the order the interface offers them.
    pub const ALL: [Self; 20] = [
        Self::Original,
        Self::BlackAndWhite,
        Self::Vintage,
        Self::Cinematic,
        Self::Warm,
        Self::Cool,
        Self::Vivid,
        Self::Faded,
        Self::Vhs,
        Self::Dream,
        Self::Sepia,
        Self::Duotone,
        Self::Noir,
        Self::Golden,
        Self::Night,
        Self::Food,
        Self::Film,
        Self::Pastel,
        Self::Cyber,
        Self::Matrix,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::BlackAndWhite => "B&W",
            Self::Vintage => "Vintage",
            Self::Cinematic => "Cinematic",
            Self::Warm => "Warm",
            Self::Cool => "Cool",
            Self::Vivid => "Vivid",
            Self::Faded => "Faded",
            Self::Vhs => "VHS",
            Self::Dream => "Dream",
            Self::Sepia => "Sepia",
            Self::Duotone => "Duotone",
            Self::Noir => "Noir",
            Self::Golden => "Golden",
            Self::Night => "Night",
            Self::Food => "Food",
            Self::Film => "Film",
            Self::Pastel => "Pastel",
            Self::Cyber => "Cyber",
            Self::Matrix => "Matrix",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Original => "The clip as shot: the filter's settings back to normal",
            Self::BlackAndWhite => "No colour, a little more contrast",
            Self::Vintage => "Warm, soft and a little washed out, like an old print",
            Self::Cinematic => "Deeper contrast, cooler shadows, calmer colour",
            Self::Warm => "Golden-hour warmth",
            Self::Cool => "A cool, clean blue cast",
            Self::Vivid => "Punchier colour and contrast",
            Self::Faded => "Low contrast and quiet colour",
            Self::Vhs => "Bleeding colour edges and a soft, tired tape picture",
            Self::Dream => "Soft, bright and gently glowing",
            Self::Sepia => "The warm brown of an old photograph",
            Self::Duotone => "Two bold colours: deep blue shadows, pink highlights",
            Self::Noir => "Hard black and white, deep shadows, like an old detective film",
            Self::Golden => "Rich, glowing late-afternoon sun",
            Self::Night => "Darker and moonlit blue",
            Self::Food => "Warm, bright and appetising, the colours lifted",
            Self::Film => "Gentle contrast and a slightly warm, muted colour, as shot on film",
            Self::Pastel => "Light, soft and sweet: gentle colour with the shadows lifted",
            Self::Cyber => "Neon purple shadows into electric cyan highlights",
            Self::Matrix => "Everything in shades of computer-screen green",
        }
    }

    fn recipe(self) -> Recipe {
        match self {
            Self::Original => NEUTRAL,
            Self::BlackAndWhite => Recipe {
                contrast: 1.15,
                saturation: 0.0,
                ..NEUTRAL
            },
            Self::Vintage => Recipe {
                brightness: 1.03,
                contrast: 0.85,
                saturation: 0.7,
                temperature: 0.35,
                tint: 0.08,
                ..NEUTRAL
            },
            Self::Cinematic => Recipe {
                brightness: 0.95,
                contrast: 1.2,
                saturation: 0.85,
                temperature: -0.12,
                sharpen: 10.0,
                ..NEUTRAL
            },
            Self::Warm => Recipe {
                brightness: 1.02,
                saturation: 1.08,
                temperature: 0.4,
                ..NEUTRAL
            },
            Self::Cool => Recipe {
                saturation: 0.95,
                temperature: -0.4,
                tint: -0.05,
                ..NEUTRAL
            },
            Self::Vivid => Recipe {
                contrast: 1.15,
                saturation: 1.4,
                sharpen: 15.0,
                ..NEUTRAL
            },
            Self::Faded => Recipe {
                brightness: 1.05,
                contrast: 0.75,
                saturation: 0.6,
                ..NEUTRAL
            },
            Self::Vhs => Recipe {
                contrast: 0.9,
                saturation: 1.2,
                tint: 0.1,
                blur: 6.0,
                rgb_split: 35.0,
                ..NEUTRAL
            },
            Self::Dream => Recipe {
                brightness: 1.12,
                contrast: 0.85,
                saturation: 1.1,
                temperature: 0.1,
                blur: 10.0,
                ..NEUTRAL
            },
            Self::Sepia => Recipe {
                contrast: 1.05,
                tone: Tone::Sepia,
                ..NEUTRAL
            },
            Self::Duotone => Recipe {
                contrast: 1.1,
                tone: Tone::DUOTONE,
                ..NEUTRAL
            },
            Self::Noir => Recipe {
                brightness: 0.92,
                contrast: 1.45,
                saturation: 0.0,
                sharpen: 10.0,
                ..NEUTRAL
            },
            Self::Golden => Recipe {
                brightness: 1.04,
                contrast: 1.05,
                saturation: 1.15,
                temperature: 0.55,
                tint: 0.05,
                ..NEUTRAL
            },
            Self::Night => Recipe {
                brightness: 0.85,
                contrast: 1.1,
                saturation: 0.8,
                temperature: -0.35,
                ..NEUTRAL
            },
            Self::Food => Recipe {
                brightness: 1.05,
                saturation: 1.25,
                temperature: 0.2,
                sharpen: 10.0,
                ..NEUTRAL
            },
            Self::Film => Recipe {
                contrast: 1.1,
                saturation: 0.9,
                temperature: 0.08,
                tint: 0.04,
                ..NEUTRAL
            },
            Self::Pastel => Recipe {
                brightness: 1.1,
                contrast: 0.8,
                saturation: 0.75,
                tint: 0.05,
                ..NEUTRAL
            },
            Self::Cyber => Recipe {
                contrast: 1.15,
                tone: Tone::Duotone {
                    shadow: [40, 0, 80],
                    highlight: [0, 255, 230],
                },
                ..NEUTRAL
            },
            Self::Matrix => Recipe {
                contrast: 1.2,
                tone: Tone::Duotone {
                    shadow: [0, 18, 6],
                    highlight: [140, 255, 160],
                },
                ..NEUTRAL
            },
        }
    }

    /// The properties this filter sets — always the same set, so any filter
    /// fully replaces another.
    pub fn properties(self) -> Vec<ClipProperty> {
        let r = self.recipe();
        vec![
            ClipProperty::Brightness(r.brightness),
            ClipProperty::Contrast(r.contrast),
            ClipProperty::Saturation(r.saturation),
            ClipProperty::Temperature(r.temperature),
            ClipProperty::Tint(r.tint),
            ClipProperty::Sharpen(r.sharpen),
            ClipProperty::Blur(r.blur),
            ClipProperty::RgbSplit(r.rgb_split),
            ClipProperty::Tone(r.tone),
        ]
    }
}

impl Editor {
    /// Give `clips` a filter's look, as one undo step. Picture clips only; the
    /// rest are skipped. Returns how many took it.
    pub fn apply_filter(
        &mut self,
        filter: Filter,
        clips: impl IntoIterator<Item = ClipId>,
    ) -> Result<usize, EditorError> {
        self.paste_look(&filter.properties(), clips)
    }

    /// The filter `clip` looks like right now, if its settings are exactly
    /// one. `None` once any of them has been tuned by hand.
    pub fn filter_of(&self, clip: ClipId) -> Option<Filter> {
        let video = self.video_clip(clip)?;
        let now = [
            ClipProperty::Brightness(video.color.brightness),
            ClipProperty::Contrast(video.color.contrast),
            ClipProperty::Saturation(video.color.saturation),
            ClipProperty::Temperature(video.color.temperature),
            ClipProperty::Tint(video.color.tint),
            ClipProperty::Sharpen(video.sharpen),
            ClipProperty::Blur(video.blur),
            ClipProperty::RgbSplit(video.rgb_split),
            ClipProperty::Tone(video.curves.tone),
        ];
        Filter::ALL
            .into_iter()
            .find(|filter| filter.properties() == now)
    }
}
