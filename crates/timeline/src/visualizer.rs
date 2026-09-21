//! An audio visualizer: a row of bars across the frame that jump with a
//! sound's loudness.
//!
//! The loudness is measured once, when the visualizer is made, and kept in the
//! project — the frame plan has no sound to listen to, and a preview and an
//! export that measured separately could disagree (§46). The bars move from
//! that one measurement, each with its own wobble, so they read as a spectrum
//! without needing one.

use bettercut_foundation::TimelineTime;
use serde::{Deserialize, Serialize};

/// How many loudness samples a visualizer keeps per second.
pub const LEVELS_PER_SECOND: u32 = 30;

/// The most bars, and the tallest a visualizer may be (share of the height).
pub const MAX_BARS: u32 = 64;
pub const MAX_VISUALIZER_HEIGHT: f32 = 0.5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Visualizer {
    /// Loudness, 0–1, [`LEVELS_PER_SECOND`] a second from `start`.
    pub levels: Vec<f32>,
    /// Where on the timeline the first level is.
    pub start: TimelineTime,
    /// How many bars across the frame.
    pub bars: u32,
    /// How tall the loudest bar is, as a share of the frame's height.
    pub height: f32,
    /// The bars' colour, sRGB.
    pub colour: [u8; 3],
    /// Hanging from the top rather than standing on the bottom.
    #[serde(default)]
    pub top: bool,
}

impl Visualizer {
    /// A visualizer of `levels` from `start`, in the default look: white bars
    /// along the bottom.
    pub fn new(levels: Vec<f32>, start: TimelineTime) -> Self {
        Self {
            levels,
            start,
            bars: 32,
            height: 0.2,
            colour: [255, 255, 255],
            top: false,
        }
    }

    /// Brought into range, as every value a project file can carry is.
    pub fn clamped(mut self) -> Self {
        self.bars = self.bars.clamp(1, MAX_BARS);
        self.height = if self.height.is_finite() {
            self.height.clamp(0.0, MAX_VISUALIZER_HEIGHT)
        } else {
            0.0
        };
        for level in &mut self.levels {
            *level = if level.is_finite() {
                level.clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
        self
    }

    /// Where the levels end on the timeline.
    pub fn end(&self) -> TimelineTime {
        self.start
            + TimelineTime::from_ticks(
                self.levels.len() as i64 * bettercut_foundation::TICKS_PER_SECOND
                    / i64::from(LEVELS_PER_SECOND),
            )
    }

    /// The loudness at `at`, between the two nearest levels; zero outside.
    pub fn level_at(&self, at: TimelineTime) -> f32 {
        let into = (at.ticks() - self.start.ticks()) as f64
            / bettercut_foundation::TICKS_PER_SECOND as f64
            * f64::from(LEVELS_PER_SECOND);
        if into < 0.0 || self.levels.is_empty() {
            return 0.0;
        }
        let index = into.floor() as usize;
        let Some(&here) = self.levels.get(index) else {
            return 0.0;
        };
        let next = self.levels.get(index + 1).copied().unwrap_or(here);
        let t = (into - into.floor()) as f32;
        here + (next - here) * t
    }

    /// Each bar's height at `at`, as a share of the frame's height.
    ///
    /// The loudness times a shape that sways differently for every bar, a
    /// little taller in the middle — so a loud moment is a busy skyline, and a
    /// quiet one flattens to nothing.
    pub fn bar_heights(&self, at: TimelineTime) -> Vec<f32> {
        let look = self.clone().clamped();
        let level = look.level_at(at);
        let seconds = at.ticks() as f32 / bettercut_foundation::TICKS_PER_SECOND as f32;
        let bars = look.bars as usize;
        (0..bars)
            .map(|i| {
                let x = i as f32 / bars.max(1) as f32;
                let sway = 0.5
                    + 0.5 * (i as f32 * 1.7 + seconds * 7.0 + (i as f32 * 0.9).sin() * 2.0).sin();
                let middle = 1.0 - 0.4 * (2.0 * x - 1.0).powi(2);
                (level * (0.3 + 0.7 * sway) * middle * look.height).clamp(0.0, look.height)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loud_moments_make_tall_bars_and_silence_none() {
        let mut levels = vec![0.0; 30];
        levels.extend(vec![1.0; 30]);
        let v = Visualizer::new(levels, TimelineTime::from_seconds(10));
        assert_eq!(v.end(), TimelineTime::from_seconds(12));

        let quiet = v.bar_heights(TimelineTime::from_millis(10_500));
        assert!(quiet.iter().all(|h| *h == 0.0));
        let loud = v.bar_heights(TimelineTime::from_millis(11_500));
        assert_eq!(loud.len(), 32);
        assert!(loud.iter().all(|h| *h <= 0.2 + 1e-6));
        assert!(loud.iter().any(|h| *h > 0.1));
        // Not all the same height: it moves like a spectrum.
        let (low, high) = loud
            .iter()
            .fold((f32::MAX, 0.0_f32), |(l, h), v| (l.min(*v), h.max(*v)));
        assert!(high - low > 0.02);

        // Before and after the sound, nothing.
        assert!(
            v.bar_heights(TimelineTime::from_seconds(5))
                .iter()
                .all(|h| *h == 0.0)
        );
        assert!(
            v.bar_heights(TimelineTime::from_seconds(13))
                .iter()
                .all(|h| *h == 0.0)
        );
    }
}
