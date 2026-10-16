//! Remove flicker: a brightness gain for every frame of a clip, evening out
//! the pulse of a time-lapse or of lights on mains power — CapCut's "Remove
//! flicker".
//!
//! Flicker is the picture's overall brightness going up and down from one
//! frame to the next, faster than anything in the scene moves. So each
//! frame's brightness is compared with the average of the frames around it,
//! over about half a second, and the frame is brightened or darkened by the
//! difference. A real change of light — a cloud, a pan towards a window —
//! is slower than the window and carries the average with it, so it stays.
//!
//! The gains are worked out once, by decoding the clip (a job in the
//! playback crate), and kept on the clip; playing it only reads them, as a
//! multiply on the clip's brightness.

use bettercut_foundation::MediaTime;
use serde::{Deserialize, Serialize};

/// The most one frame is brightened, and (as its inverse) darkened: room for
/// a deep pulse, not enough to make a frame that really is dark into a
/// grey one.
pub const MAX_GAIN: f32 = 1.6;

/// How long the window each frame is compared against lasts, in seconds:
/// several cycles of a fast flicker, too short to swallow a real change of
/// light.
pub const WINDOW_SECONDS: f64 = 0.5;

/// A frame darker than this (as linear luminance) is left alone: black has
/// no brightness to even out, and a gain worked out from almost nothing is
/// mostly noise.
const DARKEST: f32 = 0.003;

/// One brightness gain per frame of the source, from `start`, a frame
/// (`step`) apart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlickerGains {
    /// Where in the source the first gain belongs.
    pub start: MediaTime,
    /// The source time from one gain to the next: a frame.
    pub step: MediaTime,
    pub gains: Vec<f32>,
}

impl FlickerGains {
    /// The gains that even out `brightness`: each frame's source time and
    /// its average linear luminance, in order. `None` with fewer than three
    /// frames, where there is nothing to compare a frame with.
    pub fn from_brightness(brightness: &[(MediaTime, f32)]) -> Option<Self> {
        let (first, last) = (brightness.first()?, brightness.last()?);
        let count = brightness.len();
        if count < 3 || last.0 <= first.0 {
            return None;
        }
        let step = MediaTime::from_ticks((last.0.ticks() - first.0.ticks()) / (count as i64 - 1));
        if step.ticks() <= 0 {
            return None;
        }
        // Frames either side of each one in its window.
        let reach = ((WINDOW_SECONDS / 2.0) / step.as_seconds_f64())
            .round()
            .max(1.0) as usize;
        let levels: Vec<f32> = brightness.iter().map(|(_, level)| level.max(0.0)).collect();
        // Running sums, so each window's average costs nothing however long
        // the clip.
        let mut sums = Vec::with_capacity(count + 1);
        sums.push(0.0_f64);
        for level in &levels {
            sums.push(sums.last().copied().unwrap_or(0.0) + f64::from(*level));
        }
        let gains = (0..count)
            .map(|index| {
                let level = levels[index];
                if level < DARKEST {
                    return 1.0;
                }
                let (from, to) = (index.saturating_sub(reach), (index + reach + 1).min(count));
                let average = ((sums[to] - sums[from]) / (to - from) as f64) as f32;
                (average / level).clamp(1.0 / MAX_GAIN, MAX_GAIN)
            })
            .collect();
        Some(Self {
            start: first.0,
            step,
            gains,
        })
    }

    /// The gain for the frame at `at` in the source: read between the two
    /// frames either side, so a clip slowed down or played at another rate
    /// still meets a gain at every instant; held at the ends.
    pub fn gain_at(&self, at: MediaTime) -> f32 {
        let Some(last) = self.gains.len().checked_sub(1) else {
            return 1.0;
        };
        let position = (at.ticks() - self.start.ticks()) as f64 / self.step.ticks().max(1) as f64;
        if position <= 0.0 {
            return self.gains[0];
        }
        let below = position.floor() as usize;
        if below >= last {
            return self.gains[last];
        }
        let along = (position - below as f64) as f32;
        self.gains[below] + (self.gains[below + 1] - self.gains[below]) * along
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame every 1/30 s.
    fn at(frame: i64) -> MediaTime {
        MediaTime::from_ticks(frame * MediaTime::from_seconds(1).ticks() / 30)
    }

    #[test]
    fn a_pulse_is_evened_out() {
        // Every other frame a fifth brighter: the flicker of lights on the
        // wrong shutter speed.
        let frames: Vec<(MediaTime, f32)> = (0..90)
            .map(|frame| (at(frame), if frame % 2 == 0 { 0.30 } else { 0.36 }))
            .collect();
        let gains = FlickerGains::from_brightness(&frames).unwrap();
        for (frame, (_, level)) in frames.iter().enumerate().skip(10).take(70) {
            let evened = level * gains.gain_at(at(frame as i64));
            assert!(
                (evened - 0.33).abs() < 0.004,
                "frame {frame} came out {evened}"
            );
        }
    }

    #[test]
    fn a_slow_change_of_light_is_kept() {
        // Brightening steadily over three seconds: a cloud moving off.
        let frames: Vec<(MediaTime, f32)> = (0..90)
            .map(|frame| (at(frame), 0.2 + 0.2 * frame as f32 / 89.0))
            .collect();
        let gains = FlickerGains::from_brightness(&frames).unwrap();
        // Away from the ends, where the window is cut short, nothing moves.
        for frame in 10..80 {
            let gain = gains.gain_at(at(frame));
            assert!((gain - 1.0).abs() < 0.01, "frame {frame} got {gain}");
        }
    }

    #[test]
    fn black_is_left_alone_and_gains_are_held_in_bounds() {
        let frames: Vec<(MediaTime, f32)> = (0..30)
            .map(|frame| {
                let level = match frame {
                    15 => 0.0,
                    16 => 0.02,
                    _ => 0.5,
                };
                (at(frame), level)
            })
            .collect();
        let gains = FlickerGains::from_brightness(&frames).unwrap();
        assert_eq!(gains.gain_at(at(15)), 1.0, "black was brightened");
        assert_eq!(
            gains.gain_at(at(16)),
            MAX_GAIN,
            "a dim frame was pushed past the limit"
        );
    }

    #[test]
    fn between_frames_the_gain_is_read_across() {
        let gains = FlickerGains {
            start: at(10),
            step: at(1),
            gains: vec![1.0, 1.2, 0.8],
        };
        assert_eq!(gains.gain_at(at(0)), 1.0, "held before the start");
        assert_eq!(gains.gain_at(at(11)), 1.2);
        let half = MediaTime::from_ticks((at(11).ticks() + at(12).ticks()) / 2);
        assert!((gains.gain_at(half) - 1.0).abs() < 1e-4);
        assert_eq!(gains.gain_at(at(50)), 0.8, "held after the end");
    }

    #[test]
    fn too_few_frames_say_nothing() {
        assert!(FlickerGains::from_brightness(&[(at(0), 0.3), (at(1), 0.4)]).is_none());
    }
}
