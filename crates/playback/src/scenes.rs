//! Finding the cuts in footage: scene detection (Milestone 12's "Scene detection").
//!
//! A file that came off a camera in one take has no cuts in it, but a file
//! exported from somewhere else — a screen recording, a download, last year's
//! edit — usually does, and re-cutting it by hand means scrubbing for every
//! one. This finds them.
//!
//! # How a cut is recognised
//!
//! Each frame is reduced to a [`FrameDigest`]: the average brightness of the
//! cells of a coarse grid. Consecutive digests are compared, and a cut is where
//! the picture changes far more than it has been changing.
//!
//! Both halves of that matter. An absolute threshold alone marks a cut every
//! few frames in a whip pan or a handheld shot; a relative one alone marks
//! noise in a locked-off shot of a wall. A cut has to clear the floor *and*
//! stand out against its neighbours.
//!
//! The grid is what makes it a picture comparison rather than a histogram one:
//! a bar sweeping across the frame changes no histogram at all, and a pan
//! changes every cell. Cells rather than pixels because a cut is a change in
//! the *composition*, and comparing pixels would call sensor noise a cut.
//!
//! Like [`crate::silence`], what comes back is a *suggestion*: instants for the
//! interface to offer, not edits already made.

use bettercut_foundation::MediaTime;
use bettercut_media::{FrameStorage, VideoFrame};

/// Grid columns. 16 × 9 matches the usual shape of a frame, so the cells are
/// square and a move across the picture crosses the same number of them either
/// way.
pub const CELLS_X: usize = 16;
/// Grid rows.
pub const CELLS_Y: usize = 9;

/// A frame reduced to what a cut changes: where the light is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDigest {
    /// Mean luma per cell, row by row.
    pub cells: [u8; CELLS_X * CELLS_Y],
}

impl FrameDigest {
    /// How different two frames are, 0 to 255.
    pub fn difference(&self, other: &Self) -> f32 {
        let total: u32 = self
            .cells
            .iter()
            .zip(&other.cells)
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .sum();
        total as f32 / self.cells.len() as f32
    }
}

/// Reduce a decoded frame to its digest, or `None` for a frame that is already
/// on the GPU — this runs in a background job, where frames come back in system
/// memory, and reading one back off the card to analyse it would cost more than
/// the whole detection.
pub fn digest(frame: &VideoFrame) -> Option<FrameDigest> {
    let FrameStorage::System { data, stride } = &frame.storage else {
        return None;
    };
    if frame.width == 0 || frame.height == 0 {
        return None;
    }

    let stride = *stride as usize;
    let mut cells = [0_u8; CELLS_X * CELLS_Y];
    for (index, cell) in cells.iter_mut().enumerate() {
        let (cx, cy) = (index % CELLS_X, index / CELLS_X);
        // The cell's bounds in pixels, as exact thirds-of-a-pixel would be:
        // integer division either side, so cells tile the frame with no gap.
        let x0 = cx * frame.width as usize / CELLS_X;
        let x1 = ((cx + 1) * frame.width as usize / CELLS_X).max(x0 + 1);
        let y0 = cy * frame.height as usize / CELLS_Y;
        let y1 = ((cy + 1) * frame.height as usize / CELLS_Y).max(y0 + 1);

        let mut sum = 0_u64;
        let mut count = 0_u64;
        for y in y0..y1 {
            for x in x0..x1 {
                let at = y * stride + x * 4;
                let Some(pixel) = data.get(at..at + 3) else {
                    continue;
                };
                // Rec. 601 luma, in integers: a cut is a change in brightness
                // and this is only ever compared against itself.
                sum += (54 * u64::from(pixel[0])
                    + 183 * u64::from(pixel[1])
                    + 19 * u64::from(pixel[2]))
                    >> 8;
                count += 1;
            }
        }
        *cell = sum.checked_div(count).unwrap_or(0).min(255) as u8;
    }
    Some(FrameDigest { cells })
}

/// No part of the picture brighter than this, 0–255, and a frame is black.
/// Above video black (16) with room for sensor noise; below the darkest
/// corner of a night shot with anything lit in it.
pub const BLACK_LEVEL: u8 = 24;

/// Whether a frame is black all over.
pub fn is_black(frame: &FrameDigest) -> bool {
    frame.cells.iter().all(|cell| *cell <= BLACK_LEVEL)
}

/// Black at the ends of a stretch of footage, in source time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlackEnds {
    /// Where the picture starts, when the stretch opens on black.
    pub head: Option<MediaTime>,
    /// Where the black starts, when the stretch closes on it.
    pub tail: Option<MediaTime>,
}

impl BlackEnds {
    pub fn any(&self) -> bool {
        self.head.is_some() || self.tail.is_some()
    }
}

/// The black at either end of `frames` (in order). Footage that is black all
/// the way through has no picture to keep, so it answers nothing rather
/// than offering to trim the clip away.
pub fn black_ends(frames: &[(MediaTime, FrameDigest)]) -> BlackEnds {
    let Some(first) = frames.iter().position(|(_, d)| !is_black(d)) else {
        return BlackEnds::default();
    };
    let last = frames
        .iter()
        .rposition(|(_, d)| !is_black(d))
        .unwrap_or(first);
    BlackEnds {
        head: (first > 0).then(|| frames[first].0),
        tail: frames.get(last + 1).map(|(at, _)| *at),
    }
}

/// How a cut is recognised.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneSettings {
    /// A change smaller than this is never a cut, 0–255. The floor that keeps
    /// noise in a still shot from being marked.
    pub threshold: f32,
    /// A cut has to be this many times the recent average change. What keeps a
    /// pan — where every frame differs a lot, steadily — from being cut into
    /// pieces.
    pub ratio: f32,
    /// How many neighbouring comparisons "recent" means.
    pub window: usize,
    /// Two cuts closer together than this are one cut: the first. A flash, a
    /// camera bulb or one dropped frame otherwise reads as a shot lasting two
    /// frames.
    pub shortest: MediaTime,
}

impl Default for SceneSettings {
    fn default() -> Self {
        Self {
            threshold: 14.0,
            ratio: 2.5,
            window: 16,
            shortest: MediaTime::from_millis(500),
        }
    }
}

/// The source instants where the picture cuts, in order.
///
/// Each is the timestamp of the *first frame of the new shot*, which is where a
/// split belongs: the frame before it is the last of the old one.
///
/// `frames` must be in presentation order, as a decoder walking a file gives
/// them. Frames may be sampled — every second or third one — as long as the
/// spacing is even: the comparison is against neighbours, so an uneven walk
/// makes a big gap look like a cut.
pub fn scene_cuts(frames: &[(MediaTime, FrameDigest)], settings: SceneSettings) -> Vec<MediaTime> {
    if frames.len() < 3 {
        return Vec::new(); // nothing to compare a change against
    }

    let changes: Vec<f32> = frames
        .windows(2)
        .map(|pair| pair[0].1.difference(&pair[1].1))
        .collect();

    let mut cuts: Vec<MediaTime> = Vec::new();
    for (index, &change) in changes.iter().enumerate() {
        if change < settings.threshold {
            continue;
        }
        // The neighbours either side, this comparison excluded: a cut is not
        // evidence about how much the footage was moving.
        let from = index.saturating_sub(settings.window);
        let to = (index + settings.window + 1).min(changes.len());
        let (sum, count) = changes[from..to]
            .iter()
            .enumerate()
            .filter(|&(offset, _)| from + offset != index)
            .fold((0.0_f32, 0_u32), |(sum, count), (_, &value)| {
                (sum + value, count + 1)
            });
        let average = if count == 0 { 0.0 } else { sum / count as f32 };
        if change < average * settings.ratio {
            continue;
        }

        let at = frames[index + 1].0;
        if let Some(&last) = cuts.last()
            && at - last < settings.shortest
        {
            continue;
        }
        cuts.push(at);
    }
    cuts
}

#[cfg(test)]
mod tests {

    #[test]
    fn black_is_found_at_the_ends_and_only_there() {
        let black = FrameDigest {
            cells: [10; CELLS_X * CELLS_Y],
        };
        let mut lit = black;
        lit.cells[40] = 200;
        let at = |s: i64| MediaTime::from_seconds(s);
        let frames = [
            (at(0), black),
            (at(1), lit),
            (at(2), black),
            (at(3), lit),
            (at(4), black),
        ];
        let ends = black_ends(&frames);
        assert_eq!(ends.head, Some(at(1)));
        assert_eq!(
            ends.tail,
            Some(at(4)),
            "black in the middle is a fade, not an end"
        );
        assert!(!black_ends(&frames[1..4]).any(), "no black at either end");
        assert!(
            !black_ends(&[(at(0), black)]).any(),
            "all black keeps the clip"
        );
    }
    use super::*;

    fn ms(v: i64) -> MediaTime {
        MediaTime::from_millis(v)
    }

    /// A digest whose every cell is `level`, give or take `noise` on alternate
    /// cells: a flat picture that is never quite identical frame to frame.
    fn flat(level: u8, noise: u8) -> FrameDigest {
        let mut cells = [level; CELLS_X * CELLS_Y];
        for (index, cell) in cells.iter_mut().enumerate() {
            if index % 2 == 0 {
                *cell = cell.saturating_add(noise);
            }
        }
        FrameDigest { cells }
    }

    /// A digest with a bright band `at` cells from the left: a picture with
    /// something in it, in a place.
    fn band(at: usize) -> FrameDigest {
        let mut cells = [20_u8; CELLS_X * CELLS_Y];
        for row in 0..CELLS_Y {
            for column in at..(at + 3).min(CELLS_X) {
                cells[row * CELLS_X + column] = 220;
            }
        }
        FrameDigest { cells }
    }

    /// Where the band is on frame `index` of a pan: across and back, never
    /// wrapping. A wrap would jump the band from one edge to the other, which
    /// is a cut — the thing these tests are trying not to contain.
    fn sweep(index: usize) -> usize {
        let span = CELLS_X - 3;
        let phase = index % (span * 2);
        if phase < span {
            phase
        } else {
            span * 2 - phase
        }
    }

    /// Frames every 40 ms from a list of digests.
    fn at_25fps(digests: Vec<FrameDigest>) -> Vec<(MediaTime, FrameDigest)> {
        digests
            .into_iter()
            .enumerate()
            .map(|(index, digest)| (ms(index as i64 * 40), digest))
            .collect()
    }

    #[test]
    fn three_shots_are_two_cuts() {
        let mut frames = Vec::new();
        for (level, noise) in [(30_u8, 2_u8), (150, 3), (80, 1)] {
            for index in 0..50 {
                frames.push(flat(level + (index % 3), noise));
            }
        }
        let cuts = scene_cuts(&at_25fps(frames), SceneSettings::default());
        assert_eq!(cuts, vec![ms(50 * 40), ms(100 * 40)]);
    }

    #[test]
    fn a_still_shot_has_no_cuts() {
        let frames = at_25fps((0..80).map(|index| flat(60, index % 4)).collect());
        assert!(
            scene_cuts(&frames, SceneSettings::default()).is_empty(),
            "noise in a locked-off shot was called a cut"
        );
    }

    /// The check the absolute threshold cannot make: during a pan every frame
    /// differs enormously from the last, and none of it is a cut.
    #[test]
    fn a_pan_is_not_a_series_of_cuts() {
        let frames = at_25fps((0..60).map(|index| band(sweep(index))).collect());
        let cuts = scene_cuts(&frames, SceneSettings::default());
        assert!(
            cuts.is_empty(),
            "a pan was cut into pieces at {cuts:?} — every frame differs, so \
             only standing out from the neighbours can tell them apart"
        );
    }

    /// A pan that ends in a cut: the cut is still found, and only the cut.
    #[test]
    fn a_cut_at_the_end_of_a_pan_is_found() {
        let mut digests: Vec<FrameDigest> = (0..30).map(|index| band(sweep(index))).collect();
        digests.extend((0..30).map(|index| flat(200, index % 3)));
        let cuts = scene_cuts(&at_25fps(digests), SceneSettings::default());
        assert_eq!(cuts, vec![ms(30 * 40)]);
    }

    #[test]
    fn a_flash_is_not_two_cuts() {
        let mut digests: Vec<FrameDigest> = (0..40).map(|_| flat(30, 1)).collect();
        digests.push(flat(250, 0)); // one white frame: a camera flash
        digests.extend((0..40).map(|_| flat(30, 1)));
        let cuts = scene_cuts(&at_25fps(digests), SceneSettings::default());
        assert_eq!(
            cuts.len(),
            1,
            "a flash was read as a one-frame shot: {cuts:?}"
        );
    }

    #[test]
    fn too_few_frames_to_judge() {
        let frames = at_25fps(vec![flat(0, 0), flat(255, 0)]);
        assert!(scene_cuts(&frames, SceneSettings::default()).is_empty());
    }

    #[test]
    fn a_digest_knows_where_the_light_is() {
        let left = band(1);
        let right = band(12);
        assert!(
            left.difference(&right) > 40.0,
            "the same picture moved across the frame read as unchanged"
        );
        assert_eq!(left.difference(&left), 0.0);
    }
}
