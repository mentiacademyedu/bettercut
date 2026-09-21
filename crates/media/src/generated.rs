//! Pictures made rather than read: a colour clip's solid fill or gradient.
//!
//! A generated entry sits in the media list like any file, so it goes on a
//! picture lane under the footage, trims, fades and grades like a photo. It has
//! no file behind it, which is why decoding, the missing-file check and the
//! background jobs all ask [`crate::MediaAsset::generated`] first.

use bettercut_foundation::{MediaTime, SequenceId};
use serde::{Deserialize, Serialize};

use crate::color::ColorMetadata;
use crate::decoder::{FrameStorage, VideoFrame};

/// What a generated entry draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Generated {
    /// `top` fading to `bottom` down the frame; the same colour twice is a
    /// flat fill. sRGB bytes.
    Colour { top: [u8; 3], bottom: [u8; 3] },
    /// A compound clip: several clips folded into a sequence of their own,
    /// played on the timeline as one.
    ///
    /// Nothing decodes this. The player and the export expand the clip into
    /// the sequence's own layers (`bettercut_playback::compound`); the frame
    /// below is only what something that ignored that would draw, and a dark
    /// grey reads as "nothing here" rather than as a black picture.
    Compound { sequence: SequenceId },
}

/// The shortest a generated frame is, in rows: enough steps that a gradient
/// stretched up to the output height shows no bands.
const MIN_ROWS: u32 = 144;

impl Generated {
    /// A flat fill.
    pub const fn solid(rgb: [u8; 3]) -> Self {
        Self::Colour {
            top: rgb,
            bottom: rgb,
        }
    }

    /// Whether this is one colour rather than two.
    pub fn is_solid(self) -> bool {
        match self {
            Self::Colour { top, bottom } => top == bottom,
            Self::Compound { .. } => false,
        }
    }

    /// The sequence a compound clip plays, if this is one.
    pub fn compound(self) -> Option<SequenceId> {
        match self {
            Self::Compound { sequence } => Some(sequence),
            Self::Colour { .. } => None,
        }
    }

    /// A name for the media list: "Colour #1E90FF" or "Gradient #FF0000 to
    /// #0000FF".
    pub fn name(self) -> String {
        let hex = |c: [u8; 3]| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
        match self {
            Self::Colour { top, bottom } if top == bottom => format!("Colour {}", hex(top)),
            Self::Colour { top, bottom } => format!("Gradient {} to {}", hex(top), hex(bottom)),
            Self::Compound { .. } => "Compound clip".to_owned(),
        }
    }

    /// The frame this draws, for a picture of `width` × `height`.
    ///
    /// Small, but the same shape as the picture: layers are fitted to the
    /// frame by their aspect, so a frame of the wrong shape would letterbox.
    /// The texture sampler smooths it up to the output size.
    pub fn frame(self, width: u32, height: u32) -> VideoFrame {
        let (w, h) = frame_size(width, height);
        let (top, bottom) = match self {
            Self::Colour { top, bottom } => (top, bottom),
            // See the variant's own note: this is never what the viewer sees.
            Self::Compound { .. } => ([32, 34, 38], [32, 34, 38]),
        };
        let mut data = Vec::with_capacity(w as usize * h as usize * 4);
        for row in 0..h {
            let t = if h > 1 {
                row as f32 / (h - 1) as f32
            } else {
                0.0
            };
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            let pixel = [
                mix(top[0], bottom[0]),
                mix(top[1], bottom[1]),
                mix(top[2], bottom[2]),
                255,
            ];
            for _ in 0..w {
                data.extend_from_slice(&pixel);
            }
        }
        VideoFrame {
            timestamp: MediaTime::ZERO,
            width: w,
            height: h,
            color: ColorMetadata::default(),
            storage: FrameStorage::System {
                data,
                stride: w * 4,
            },
        }
    }
}

/// The picture's shape at the smallest whole size, grown until it has at
/// least [`MIN_ROWS`] rows: 1920 × 1080 is 16 × 9 is 256 × 144.
pub fn frame_size(width: u32, height: u32) -> (u32, u32) {
    let (width, height) = (width.max(1), height.max(1));
    let divisor = gcd(width, height);
    let (w, h) = (width / divisor, height / divisor);
    let grow = MIN_ROWS.div_ceil(h).max(1);
    // Never larger than the picture itself: an odd shape that does not
    // reduce is already its own size.
    if h * grow > height {
        (width, height)
    } else {
        (w * grow, h * grow)
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_keep_the_picture_shape() {
        assert_eq!(frame_size(1920, 1080), (256, 144));
        assert_eq!(frame_size(1080, 1920), (81, 144));
        assert_eq!(frame_size(1080, 1080), (144, 144));
        // Does not reduce, and is smaller than the rows wanted.
        assert_eq!(frame_size(101, 67), (101, 67));
    }

    #[test]
    fn a_gradient_runs_top_to_bottom() {
        let frame = Generated::Colour {
            top: [255, 0, 0],
            bottom: [0, 0, 255],
        }
        .frame(1920, 1080);
        let FrameStorage::System { data, stride } = &frame.storage else {
            panic!("system frame");
        };
        assert_eq!(*stride, frame.width * 4);
        assert_eq!(&data[0..4], &[255, 0, 0, 255]);
        let last = data.len() - 4;
        assert_eq!(&data[last..], &[0, 0, 255, 255]);
        let middle = (frame.height as usize / 2) * *stride as usize;
        assert!(data[middle] > 100 && data[middle + 2] > 100, "blended");
    }

    #[test]
    fn names_say_what_it_is() {
        assert_eq!(Generated::solid([30, 144, 255]).name(), "Colour #1E90FF");
        assert!(Generated::solid([1, 2, 3]).is_solid());
        assert_eq!(
            Generated::Colour {
                top: [255, 0, 0],
                bottom: [0, 0, 255]
            }
            .name(),
            "Gradient #FF0000 to #0000FF"
        );
    }
}
