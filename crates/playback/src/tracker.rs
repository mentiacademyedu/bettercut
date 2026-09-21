//! Following a patch of the picture from frame to frame (§45's "follow").
//!
//! What it is for: putting a sticker, a title or a blurred patch over something
//! that moves, without keyframing it by hand every few frames.
//!
//! How: the patch is remembered as a small grid of brightnesses, and in the
//! next frame every nearby position is tried and the one that differs least
//! wins. Brightness rather than colour because a face passing through shade
//! changes colour far more than it changes shape; a small grid rather than the
//! pixels themselves because a hundredth of the numbers is a hundredth of the
//! work, and nothing here needs sub-pixel precision — a mask a pixel out is a
//! mask nobody can see is out.

use bettercut_foundation::MediaTime;
use bettercut_media::{FrameStorage, VideoFrame};

/// How many samples across the remembered patch is.
const GRID: usize = 16;

/// How far the patch may move between frames, as a share of the frame's
/// shorter side. A twelfth is about 90 px on a 1080p frame — faster than
/// anything an editor is asked to follow, and 1/144th of the picture to
/// search.
const SEARCH: f32 = 1.0 / 12.0;

/// A patch's middle, in 0–1 frame units, at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackPoint {
    pub at: MediaTime,
    pub center: [f32; 2],
}

/// A box in 0–1 frame units: where the patch is, and how big.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Patch {
    pub center: [f32; 2],
    /// Half-width and half-height.
    pub half: [f32; 2],
}

impl Patch {
    /// Held inside the frame, and never smaller than a few pixels across.
    pub fn sane(self) -> Self {
        let half = [self.half[0].clamp(0.01, 0.5), self.half[1].clamp(0.01, 0.5)];
        Self {
            center: [
                self.center[0].clamp(half[0], 1.0 - half[0]),
                self.center[1].clamp(half[1], 1.0 - half[1]),
            ],
            half,
        }
    }
}

/// One frame as brightness, 0–1, at its own size.
pub struct Grey {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
}

impl Grey {
    /// Read a decoded frame. `None` for a frame that is not in system memory —
    /// nothing tracks a picture that only exists on the GPU.
    pub fn of(frame: &VideoFrame) -> Option<Self> {
        let FrameStorage::System { data, stride } = &frame.storage else {
            return None;
        };
        let (width, height) = (frame.width as usize, frame.height as usize);
        let stride = *stride as usize;
        if width == 0 || height == 0 || data.len() < height * stride {
            return None;
        }
        let mut values = Vec::with_capacity(width * height);
        for row in 0..height {
            let line = &data[row * stride..row * stride + width * 4];
            for pixel in line.chunks_exact(4) {
                // The usual luma weights; the picture is sRGB-encoded here,
                // which is fine — every frame is read the same way, and only
                // the differences between them matter.
                values.push(
                    (0.2126 * f32::from(pixel[0])
                        + 0.7152 * f32::from(pixel[1])
                        + 0.0722 * f32::from(pixel[2]))
                        / 255.0,
                );
            }
        }
        Some(Self {
            width,
            height,
            values,
        })
    }

    fn at(&self, x: f32, y: f32) -> f32 {
        let px = ((x * self.width as f32).round() as isize).clamp(0, self.width as isize - 1);
        let py = ((y * self.height as f32).round() as isize).clamp(0, self.height as isize - 1);
        self.values[py as usize * self.width + px as usize]
    }

    /// The patch as a [`GRID`]×[`GRID`] grid of brightnesses.
    pub fn sample(&self, patch: Patch) -> Vec<f32> {
        let patch = patch.sane();
        let mut grid = Vec::with_capacity(GRID * GRID);
        for row in 0..GRID {
            for column in 0..GRID {
                let u = (column as f32 + 0.5) / GRID as f32;
                let v = (row as f32 + 0.5) / GRID as f32;
                grid.push(self.at(
                    patch.center[0] + (u - 0.5) * 2.0 * patch.half[0],
                    patch.center[1] + (v - 0.5) * 2.0 * patch.half[1],
                ));
            }
        }
        grid
    }
}

/// Where the remembered `grid` sits in `frame`, searching around `around`.
///
/// Returns the middle of the best match in 0–1 frame units, and how well it
/// matched (1 is identical, 0 is nothing alike).
pub fn find(frame: &Grey, grid: &[f32], around: Patch, steps: usize) -> ([f32; 2], f32) {
    let around = around.sane();
    let steps = steps.max(1) as isize;
    let step = SEARCH / steps as f32;
    let mut best = (around.center, f32::MAX);
    for dy in -steps..=steps {
        for dx in -steps..=steps {
            let center = [
                around.center[0] + dx as f32 * step,
                around.center[1] + dy as f32 * step,
            ];
            let candidate = Patch {
                center,
                half: around.half,
            }
            .sane();
            let found = frame.sample(candidate);
            let difference: f32 = found
                .iter()
                .zip(grid)
                .map(|(a, b)| (a - b).abs())
                .sum::<f32>()
                / grid.len() as f32;
            if difference < best.1 {
                best = (candidate.center, difference);
            }
        }
    }
    // A mean absolute difference of a tenth is already a poor match; turn it
    // into something that reads as a confidence.
    let confidence = (1.0 - best.1 * 10.0).clamp(0.0, 1.0);
    (best.0, confidence)
}

/// Follow `patch` through `frames`, each read at its own instant.
///
/// The patch is re-read from every frame as it goes, so it follows something
/// that turns or changes size slowly; what it cannot do is find something again
/// after it has left the frame, which is what `confidence` is for — a caller
/// that sees it collapse should stop rather than follow noise.
pub fn follow<'a>(
    frames: impl IntoIterator<Item = (MediaTime, &'a Grey)>,
    patch: Patch,
) -> Vec<TrackPoint> {
    let mut path = Vec::new();
    let mut current = patch.sane();
    let mut grid: Option<Vec<f32>> = None;
    for (at, frame) in frames {
        match &grid {
            // The first frame is where the patch was put: it is the answer,
            // and what everything after it is compared against.
            None => {
                grid = Some(frame.sample(current));
                path.push(TrackPoint {
                    at,
                    center: current.center,
                });
            }
            Some(remembered) => {
                let (center, confidence) = find(frame, remembered, current, 8);
                if confidence <= 0.0 {
                    break; // lost it; better to stop than to wander
                }
                current = Patch {
                    center,
                    half: current.half,
                }
                .sane();
                path.push(TrackPoint {
                    at,
                    center: current.center,
                });
                // Follow slow changes without letting one bad frame become
                // what everything after it is matched against.
                let seen = frame.sample(current);
                grid = Some(
                    remembered
                        .iter()
                        .zip(&seen)
                        .map(|(old, new)| old * 0.8 + new * 0.2)
                        .collect(),
                );
            }
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_media::ColorMetadata;

    /// A frame with a white square of `size` pixels at `(x, y)`.
    fn frame_with_square(x: u32, y: u32, size: u32) -> VideoFrame {
        let (width, height) = (320_u32, 180_u32);
        let mut data = vec![20_u8; (width * height * 4) as usize];
        for row in y..(y + size).min(height) {
            for column in x..(x + size).min(width) {
                let at = ((row * width + column) * 4) as usize;
                data[at..at + 4].copy_from_slice(&[240, 240, 240, 255]);
            }
        }
        VideoFrame {
            timestamp: MediaTime::ZERO,
            width,
            height,
            color: ColorMetadata::srgb(),
            storage: FrameStorage::System {
                data,
                stride: width * 4,
            },
        }
    }

    fn grey(x: u32, y: u32) -> Grey {
        Grey::of(&frame_with_square(x, y, 24)).expect("system frame")
    }

    #[test]
    fn a_patch_is_found_again_where_it_moved_to() {
        let first = grey(100, 60);
        let second = grey(124, 72);
        // The square's middle in the first frame, in frame units.
        let patch = Patch {
            center: [112.0 / 320.0, 72.0 / 180.0],
            half: [16.0 / 320.0, 16.0 / 180.0],
        };
        let grid = first.sample(patch);

        let (center, confidence) = find(&second, &grid, patch, 8);
        assert!(confidence > 0.5, "confidence {confidence}");
        assert!(
            (center[0] - 136.0 / 320.0).abs() < 0.02 && (center[1] - 84.0 / 180.0).abs() < 0.03,
            "found at {center:?}"
        );
    }

    #[test]
    fn following_a_square_across_frames_gives_its_path() {
        let frames: Vec<(MediaTime, Grey)> = (0..6)
            .map(|step| {
                (
                    MediaTime::from_ticks(step as i64 * 32_000),
                    grey(100 + step * 8, 60),
                )
            })
            .collect();
        let patch = Patch {
            center: [112.0 / 320.0, 72.0 / 180.0],
            half: [16.0 / 320.0, 16.0 / 180.0],
        };

        let path = follow(frames.iter().map(|(at, grey)| (*at, grey)), patch);
        assert_eq!(path.len(), 6);
        // It should end up about 40 px to the right of where it began.
        let moved = (path[5].center[0] - path[0].center[0]) * 320.0;
        assert!((moved - 40.0).abs() < 6.0, "moved {moved} px");
        // And never sideways: the square only ever went right.
        for pair in path.windows(2) {
            assert!(pair[1].center[0] >= pair[0].center[0] - 0.01);
            assert!((pair[1].center[1] - pair[0].center[1]).abs() < 0.05);
        }
    }

    #[test]
    fn a_patch_is_kept_inside_the_frame() {
        let patch = Patch {
            center: [1.4, -0.2],
            half: [0.1, 0.1],
        }
        .sane();
        assert!(patch.center[0] <= 0.9 && patch.center[1] >= 0.1);
    }
}
