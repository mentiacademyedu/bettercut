//! Pitch shift without a change of speed: the voice changer.
//!
//! Speeding a clip up raises its pitch because the same waveform is played
//! faster. To change the pitch alone, the sound is read back through a short
//! delay whose length slides steadily — a delay that shrinks plays its
//! contents faster, raising the pitch, and one that grows plays them slower —
//! and before the delay runs out it jumps back. Two such taps, half a window
//! apart and each faded in and out with a raised-cosine, cover each other's
//! jumps, so the result is continuous.
//!
//! It is the classic rotating-tape pitch shifter: a little grainy on sustained
//! notes, entirely natural for the thing it is for — a chipmunk or a deep
//! voice on speech. Floating point but deterministic: every step depends only on
//! the samples and the settings, and all state carries from block to block, so
//! the preview and the export agree (§46).

use bettercut_foundation::AUDIO_SAMPLE_RATE;

/// The furthest the control shifts either way, in semitones: an octave.
pub const MAX_SEMITONES: f32 = 12.0;

/// The window each tap sweeps across. Long enough for the lowest voice's
/// cycles to fit several times, short enough that the grains do not echo.
const WINDOW_SECONDS: f32 = 0.05;

/// One channel's delay line and tap position.
struct Channel {
    buffer: Vec<f32>,
    at: usize,
    /// Where tap one is across the window, 0–1.
    phase: f32,
}

impl Channel {
    fn new(window: usize) -> Self {
        Self {
            // Room for the whole window plus the interpolation's neighbour.
            buffer: vec![0.0; window + 2],
            at: 0,
            phase: 0.0,
        }
    }

    /// The sample `delay` (fractional) samples ago.
    fn read(&self, delay: f32) -> f32 {
        let len = self.buffer.len();
        let whole = delay.floor() as usize;
        let frac = delay - whole as f32;
        let newest = (self.at + len - 1) % len;
        let a = self.buffer[(newest + len - whole % len) % len];
        let b = self.buffer[(newest + len - (whole + 1) % len) % len];
        a + (b - a) * frac
    }
}

/// A pitch shifter for one clip.
pub struct PitchShifter {
    /// How fast each tap's delay changes per sample, in windows.
    step: f32,
    window: f32,
    channels: Vec<Channel>,
}

impl PitchShifter {
    /// A shift of `semitones`, held to ±[`MAX_SEMITONES`]. `None` for no
    /// shift worth doing.
    pub fn new(semitones: f32) -> Option<Self> {
        if !semitones.is_finite() {
            return None;
        }
        let semitones = semitones.clamp(-MAX_SEMITONES, MAX_SEMITONES);
        if semitones.abs() < 0.05 {
            return None;
        }
        let ratio = 2.0_f32.powf(semitones / 12.0);
        let window = (WINDOW_SECONDS * AUDIO_SAMPLE_RATE as f32).round();
        Some(Self {
            // The delay shrinks by (ratio - 1) samples every sample: that is
            // what plays its contents `ratio` times as fast.
            step: (1.0 - ratio) / window,
            window,
            channels: Vec::new(),
        })
    }

    /// Shift `planes` (one per channel) in place, carrying on from the last
    /// call as if the two were one block.
    pub fn process(&mut self, planes: &mut [Vec<f32>]) {
        let window = self.window as usize;
        if self.channels.len() != planes.len() {
            self.channels = (0..planes.len()).map(|_| Channel::new(window)).collect();
        }
        let span = self.window - 1.0;
        for (plane, channel) in planes.iter_mut().zip(self.channels.iter_mut()) {
            for sample in plane.iter_mut() {
                let len = channel.buffer.len();
                channel.buffer[channel.at] = *sample;
                channel.at = (channel.at + 1) % len;

                let one = channel.phase;
                let two = (one + 0.5).fract();
                // Raised-cosine gains, each at zero where its tap jumps: the
                // two always sum to one.
                let gain_one = (std::f32::consts::PI * one).sin().powi(2);
                let gain_two = 1.0 - gain_one;
                *sample = channel.read(one * span) * gain_one + channel.read(two * span) * gain_two;

                channel.phase = (channel.phase + self.step).rem_euclid(1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| {
                (2.0 * std::f32::consts::PI * hz * i as f32 / AUDIO_SAMPLE_RATE as f32).sin() * 0.5
            })
            .collect()
    }

    /// The strongest frequency in a stretch, by trying candidates: the one
    /// whose sine the signal correlates with most.
    fn loudest(signal: &[f32], candidates: &[f32]) -> f32 {
        let power = |hz: f32| {
            let (mut re, mut im) = (0.0_f64, 0.0_f64);
            for (i, s) in signal.iter().enumerate() {
                let t = 2.0 * std::f64::consts::PI * f64::from(hz) * i as f64
                    / AUDIO_SAMPLE_RATE as f64;
                re += f64::from(*s) * t.cos();
                im += f64::from(*s) * t.sin();
            }
            re * re + im * im
        };
        candidates
            .iter()
            .copied()
            .max_by(|a, b| power(*a).total_cmp(&power(*b)))
            .unwrap()
    }

    #[test]
    fn no_shift_is_nothing_to_do() {
        assert!(PitchShifter::new(0.0).is_none());
        assert!(PitchShifter::new(f32::NAN).is_none());
        assert!(PitchShifter::new(3.0).is_some());
    }

    /// An octave up doubles the pitch and an octave down halves it, and the
    /// length of the sound is untouched.
    #[test]
    fn a_shift_moves_the_pitch_and_keeps_the_length() {
        let frames = AUDIO_SAMPLE_RATE as usize;
        for (semitones, expected) in [(12.0, 880.0), (-12.0, 220.0), (7.0, 659.3)] {
            let mut shifter = PitchShifter::new(semitones).unwrap();
            let mut planes = vec![sine(440.0, frames)];
            shifter.process(&mut planes);
            assert_eq!(planes[0].len(), frames, "the length changed");
            let settled = &planes[0][frames / 4..];
            let found = loudest(settled, &[220.0, 330.0, 440.0, 659.3, 880.0]);
            assert_eq!(
                found, expected,
                "{semitones} semitones came out at {found} Hz"
            );
        }
    }

    /// One block or many: the same samples.
    #[test]
    fn blocks_join_seamlessly() {
        let signal = sine(300.0, 12_000);
        let mut whole = PitchShifter::new(-5.0).unwrap();
        let mut one = vec![signal.clone()];
        whole.process(&mut one);

        let mut pieces = PitchShifter::new(-5.0).unwrap();
        let mut joined = Vec::new();
        for chunk in signal.chunks(777) {
            let mut block = vec![chunk.to_vec()];
            pieces.process(&mut block);
            joined.extend_from_slice(&block[0]);
        }
        assert_eq!(one[0], joined);
    }

    /// The level holds: a shifted tone is about as loud as the original.
    #[test]
    fn the_level_holds() {
        let frames = AUDIO_SAMPLE_RATE as usize / 2;
        let mut shifter = PitchShifter::new(5.0).unwrap();
        let mut planes = vec![sine(440.0, frames)];
        shifter.process(&mut planes);
        let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
        let (before, after) = (
            rms(&sine(440.0, frames)[frames / 4..]),
            rms(&planes[0][frames / 4..]),
        );
        assert!(
            (after / before - 1.0).abs() < 0.25,
            "level {before} became {after}"
        );
    }
}
