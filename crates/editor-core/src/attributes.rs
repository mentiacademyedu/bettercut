//! Which parts of a copied clip to paste (§45's "Copy Look", made choosy).
//!
//! Copy Look takes everything about how a clip is finished and puts all of
//! it on the next clip. That is right most of the time and wrong exactly
//! when it matters: the grade from the wide shot belongs on the close-up,
//! its *framing* does not. So the copied list is sorted into groups, and the
//! paste takes the groups asked for.
//!
//! The grouping is by what a person would call it, not by how the property
//! is stored: "framing" is position, scale, rotation and crop together,
//! because moving one without the others is not a thing anybody means.

use crate::command::ClipProperty;

/// A group of properties, as the paste dialog lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AttributeGroup {
    /// Where the picture sits in the frame: position, scale, rotation,
    /// anchor, crop, corner pin.
    Framing,
    /// The grade: brightness through the colour wheels and curves.
    Colour,
    /// What is done to the picture: blur, sharpen, glitches, glow, lens,
    /// posterise, tilt-shift, borders, shadows.
    Effects,
    /// Transparency: the chroma and luma keys, and the mask.
    Keys,
    /// How the sound is treated: volume, pan, EQ, clean-up, the lot.
    Sound,
    /// Speed and how it moves: speed, keep pitch, motion blur, smooth motion.
    Motion,
}

impl AttributeGroup {
    pub const ALL: [Self; 6] = [
        Self::Framing,
        Self::Colour,
        Self::Effects,
        Self::Keys,
        Self::Sound,
        Self::Motion,
    ];

    /// What the dialog calls it, and what it covers.
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::Framing => ("Framing", "Position, scale, rotation, crop and corner pin"),
            Self::Colour => (
                "Colour",
                "Brightness, contrast, white balance, wheels, curves and LUT",
            ),
            Self::Effects => (
                "Effects",
                "Blur, sharpen, glow, glitch, lens, posterise, border, shadow",
            ),
            Self::Keys => (
                "Transparency",
                "The green screen, the brightness key and the mask",
            ),
            Self::Sound => (
                "Sound",
                "Volume, pan, equaliser, clean-up and the rest of the sound chain",
            ),
            Self::Motion => ("Motion", "Speed, pitch and the motion blurs"),
        }
    }

    /// Which group a property belongs to.
    pub fn of(property: &ClipProperty) -> Self {
        use ClipProperty as P;
        match property {
            P::Position { .. }
            | P::Scale { .. }
            | P::Rotation(_)
            | P::Anchor { .. }
            | P::Crop(_)
            | P::CornerPin(_) => Self::Framing,
            P::Brightness(_)
            | P::Contrast(_)
            | P::Saturation(_)
            | P::Temperature(_)
            | P::Tint(_)
            | P::Vibrance(_)
            | P::Wheels(_)
            | P::Secondary(_)
            | P::Curves(_)
            | P::Lut(_) => Self::Colour,
            P::ChromaKey(_) | P::LumaKey(_) | P::Mask(_) => Self::Keys,
            P::Gain(_) | P::Pan(_) => Self::Sound,
            P::KeepPitch(_) | P::MotionBlur(_) | P::SmoothMotion(_) | P::Motion(_) => Self::Motion,
            // Everything else is something done to the picture: opacity,
            // blur, the glitches, the finish. A new property lands here,
            // which is the least surprising place for it to land.
            _ => Self::Effects,
        }
    }
}

/// The properties of `look` that belong to `groups`, in order.
pub fn only(look: &[ClipProperty], groups: &[AttributeGroup]) -> Vec<ClipProperty> {
    look.iter()
        .copied()
        .filter(|property| groups.contains(&AttributeGroup::of(property)))
        .collect()
}

/// Which groups `look` actually carries, so the dialog offers only those.
pub fn groups_in(look: &[ClipProperty]) -> Vec<AttributeGroup> {
    let mut found: Vec<AttributeGroup> = look.iter().map(AttributeGroup::of).collect();
    found.sort_unstable();
    found.dedup();
    found
}
