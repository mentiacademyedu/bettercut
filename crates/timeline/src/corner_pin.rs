//! Corner pin: the picture's four corners moved one at a time.
//!
//! What it is for: putting a clip *into* the shot — on the phone screen
//! somebody is holding, on the poster on the wall, on the laptop on the desk.
//! A move, a scale and a rotation can only ever make a rectangle; a screen
//! filmed from an angle is not one, and no amount of scaling will match it.
//!
//! The corners are stored as offsets from where the clip's own corners are,
//! so the pin travels with the clip: move the clip and the pinned shape moves
//! with it, and taking the pin off leaves the framing exactly as it was.

use serde::{Deserialize, Serialize};

/// The furthest a corner may be dragged from where it started, in frame
/// widths. Two frames either way is far past anything useful and keeps a
/// hand-edited project from asking for a quad the size of a county.
pub const MAX_CORNER_REACH: f32 = 2.0;

/// The four corners' offsets, in output-frame units: top-left, top-right,
/// bottom-right, bottom-left — the order the quad is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CornerPin {
    #[serde(default)]
    pub offsets: [[f32; 2]; 4],
}

impl CornerPin {
    /// No pin at all: every corner where the clip put it.
    pub const NONE: Self = Self {
        offsets: [[0.0; 2]; 4],
    };

    /// Whether the picture is still a plain rectangle.
    pub fn is_none(self) -> bool {
        self.offsets
            .iter()
            .all(|corner| corner[0] == 0.0 && corner[1] == 0.0)
    }

    /// Held to what can be drawn: finite offsets inside [`MAX_CORNER_REACH`].
    pub fn clamped(self) -> Self {
        let hold = |value: f32| {
            if value.is_finite() {
                value.clamp(-MAX_CORNER_REACH, MAX_CORNER_REACH)
            } else {
                0.0
            }
        };
        Self {
            offsets: self
                .offsets
                .map(|corner| [hold(corner[0]), hold(corner[1])]),
        }
    }

    /// One corner moved to a new offset, the rest left alone.
    pub fn with_corner(self, corner: usize, offset: [f32; 2]) -> Self {
        let mut offsets = self.offsets;
        if let Some(slot) = offsets.get_mut(corner) {
            *slot = offset;
        }
        Self { offsets }.clamped()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_moved_is_no_pin() {
        assert!(CornerPin::default().is_none());
        assert!(CornerPin::NONE.is_none());
        assert!(!CornerPin::NONE.with_corner(1, [0.1, 0.0]).is_none());
    }

    #[test]
    fn a_corner_from_a_hand_edited_file_is_held_inside_the_frame() {
        let wild = CornerPin {
            offsets: [[99.0, -99.0], [f32::NAN, 0.0], [0.0, 0.0], [0.0, 0.0]],
        }
        .clamped();
        assert_eq!(wild.offsets[0], [MAX_CORNER_REACH, -MAX_CORNER_REACH]);
        assert_eq!(wild.offsets[1], [0.0, 0.0]);
    }

    #[test]
    fn a_corner_that_is_not_there_changes_nothing() {
        let pin = CornerPin::NONE.with_corner(9, [1.0, 1.0]);
        assert!(pin.is_none());
    }
}
