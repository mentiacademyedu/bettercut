//! Burn-in: the timecode and the file name drawn over the picture.
//!
//! What it is for: a copy sent out for notes. "At 00:01:12 the cut is early"
//! is only useful if the viewer and the editor are reading the same clock, and
//! the file name beside it says which take they are looking at when three are
//! being compared.
//!
//! Deliberately not a title: it is not part of the edit, it belongs to the
//! *export* of it, and turning it off is one switch rather than finding and
//! deleting something on a lane.

use serde::{Deserialize, Serialize};

/// The largest a burn-in may be drawn, as a share of the frame's height.
pub const MAX_BURN_IN_SIZE: f32 = 0.10;
/// And the smallest that is still readable on a phone.
pub const MIN_BURN_IN_SIZE: f32 = 0.02;

/// What is burnt into the picture, and where.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BurnIn {
    /// The timecode of the frame, counted from the start of the sequence.
    #[serde(default)]
    pub timecode: bool,
    /// The name of the file the picture under it came from.
    #[serde(default)]
    pub file_name: bool,
    /// Along the top rather than the bottom.
    #[serde(default)]
    pub top: bool,
    /// How tall the text is, as a share of the frame's height.
    #[serde(default = "default_size")]
    pub size: f32,
}

fn default_size() -> f32 {
    0.04
}

impl Default for BurnIn {
    /// Nothing burnt in: what every project has until someone asks for it.
    fn default() -> Self {
        Self {
            timecode: false,
            file_name: false,
            top: false,
            size: default_size(),
        }
    }
}

impl BurnIn {
    /// Whether anything is drawn at all.
    pub fn is_visible(self) -> bool {
        (self.timecode || self.file_name) && self.size > 0.0
    }

    /// Held to what can be drawn: a size inside its limits, and nothing
    /// absurd from a hand-edited project file.
    pub fn clamped(self) -> Self {
        Self {
            size: if self.size.is_finite() {
                self.size.clamp(MIN_BURN_IN_SIZE, MAX_BURN_IN_SIZE)
            } else {
                default_size()
            },
            ..self
        }
    }

    /// Where the text sits and what it is anchored by: the lower-left corner
    /// of the frame, or the upper-left, a little in from the edge.
    pub fn placement(self) -> ([f32; 2], [f32; 2]) {
        let inset = 0.02;
        if self.top {
            ([-0.5 + inset, -0.5 + inset], [0.0, 0.0])
        } else {
            ([-0.5 + inset, 0.5 - inset], [0.0, 1.0])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_burnt_in_by_default() {
        let burn = BurnIn::default();
        assert!(!burn.is_visible());
        assert!(burn.clamped().size > 0.0);
    }

    #[test]
    fn a_size_from_a_hand_edited_file_is_held_to_its_limits() {
        let big = BurnIn {
            timecode: true,
            size: 9.0,
            ..BurnIn::default()
        }
        .clamped();
        assert_eq!(big.size, MAX_BURN_IN_SIZE);
        let broken = BurnIn {
            timecode: true,
            size: f32::NAN,
            ..BurnIn::default()
        }
        .clamped();
        assert!(broken.size.is_finite());
    }

    #[test]
    fn it_sits_in_the_corner_it_is_asked_for() {
        let (bottom, anchor) = BurnIn::default().placement();
        assert!(bottom[1] > 0.0 && anchor[1] == 1.0, "not at the bottom");
        let (top, anchor) = BurnIn {
            top: true,
            ..BurnIn::default()
        }
        .placement();
        assert!(top[1] < 0.0 && anchor[1] == 0.0, "not at the top");
    }
}
