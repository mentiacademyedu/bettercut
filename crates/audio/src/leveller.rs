//! The leveller: a voice's loud and quiet words brought closer together.
//!
//! A person speaking leans towards the mic and away, laughs, trails off. The
//! leveller is a compressor that follows the voice's level and turns it down
//! whenever it climbs past a threshold — by more the harder the control is
//! pushed — then lifts the whole thing back up so the result is steadier, not
//! merely quieter. One amount, 0–100, sets both.
//!
//! The detector follows the loudest channel, so a stereo recording is turned
//! down together and its image does not wander. It reacts in a few
//! milliseconds and lets go over a tenth of a second — quick enough to catch
//! a shout, slow enough not to flutter between syllables. All state carries
//! from block to block, so the preview and the export agree (§46).

use bettercut_foundation::AUDIO_SAMPLE_RATE;

/// Where the leveller starts turning the voice down, in dBFS.
const THRESHOLD_DB: f32 = -24.0;
/// The ratio at full amount: every 6 dB over the threshold comes out as 1.
const MAX_RATIO: f32 = 6.0;
const ATTACK_SECONDS: f32 = 0.005;
const RELEASE_SECONDS: f32 = 0.12;

fn coefficient(seconds: f32) -> f32 {
    (-1.0 / (seconds * AUDIO_SAMPLE_RATE as f32)).exp()
}

/// A leveller for one clip.
pub struct Leveller {
    ratio: f32,
    makeup: f32,
    attack: f32,
    release: f32,
    /// The detector's current level, in dB.
    envelope_db: f32,
}

impl Leveller {
    /// A leveller at `amount` (0–100). `None` for no amount.
    pub fn new(amount: f32) -> Option<Self> {
        if !amount.is_finite() || amount <= 0.0 {
            return None;
        }
        let share = (amount / 100.0).min(1.0);
        let ratio = 1.0 + (MAX_RATIO - 1.0) * share;
        // Lift back half of what a voice at 0 dBFS would lose: steadier, and
        // about as loud as it was, without pushing quiet passages into noise.
        let reduction_at_full_scale = -THRESHOLD_DB * (1.0 - 1.0 / ratio);
        let makeup = 10.0_f32.powf(reduction_at_full_scale * 0.5 / 20.0);
        Some(Self {
            ratio,
            makeup,
            attack: coefficient(ATTACK_SECONDS),
            release: coefficient(RELEASE_SECONDS),
            envelope_db: -120.0,
        })
    }

    /// Level `planes` (one per channel) in place, carrying on from the last
    /// call as if the two were one block.
    pub fn process(&mut self, planes: &mut [Vec<f32>]) {
        let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
        for frame in 0..frames {
            let peak = planes
                .iter()
                .map(|plane| plane[frame].abs())
                .fold(0.0_f32, f32::max);
            let level_db = 20.0 * peak.max(1e-6).log10();
            let coefficient = if level_db > self.envelope_db {
                self.attack
            } else {
                self.release
            };
            self.envelope_db = level_db + coefficient * (self.envelope_db - level_db);

            let over = self.envelope_db - THRESHOLD_DB;
            let reduction_db = if over > 0.0 {
                over * (1.0 - 1.0 / self.ratio)
            } else {
                0.0
            };
            let gain = 10.0_f32.powf(-reduction_db / 20.0) * self.makeup;
            for plane in planes.iter_mut() {
                plane[frame] *= gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| (i as f32 * 0.07).sin() * amplitude)
            .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn no_amount_is_nothing_to_do() {
        assert!(Leveller::new(0.0).is_none());
        assert!(Leveller::new(f32::NAN).is_none());
    }

    /// A loud passage and a quiet one come out closer together than they
    /// went in, and the harder the control, the closer.
    #[test]
    fn loud_and_quiet_come_closer() {
        let frames = AUDIO_SAMPLE_RATE as usize / 2;
        let spread = |amount: f32| {
            let mut leveller = Leveller::new(amount).unwrap();
            let mut signal = tone(0.9, frames);
            signal.extend(tone(0.05, frames));
            let mut planes = vec![signal];
            leveller.process(&mut planes);
            let loud = peak(&planes[0][frames / 2..frames]);
            let quiet = peak(&planes[0][frames + frames / 2..]);
            loud / quiet
        };
        let before = 0.9 / 0.05;
        let (gentle, strong) = (spread(30.0), spread(100.0));
        assert!(
            gentle < before * 0.8,
            "a gentle leveller did nothing: {gentle} of {before}"
        );
        assert!(
            strong < gentle * 0.8,
            "pushing harder levelled less: {strong} vs {gentle}"
        );
    }

    /// A quiet voice under the threshold is only lifted, not squashed.
    #[test]
    fn a_quiet_voice_is_left_its_shape() {
        let frames = 20_000;
        let mut leveller = Leveller::new(100.0).unwrap();
        let input = tone(0.02, frames);
        let mut planes = vec![input.clone()];
        leveller.process(&mut planes);
        let ratio = planes[0][frames - 100] / input[frames - 100];
        for (out, inp) in planes[0][frames / 2..].iter().zip(&input[frames / 2..]) {
            if inp.abs() > 1e-3 {
                assert!((out / inp - ratio).abs() < 1e-3, "the quiet voice was bent");
            }
        }
        assert!(ratio > 1.0, "the quiet voice was not lifted");
    }

    /// One block or many: the same samples.
    #[test]
    fn blocks_join_seamlessly() {
        let signal: Vec<f32> = (0..9_000)
            .map(|i| (i as f32 * 0.05).sin() * if i % 3000 < 1500 { 0.8 } else { 0.1 })
            .collect();
        let mut whole = Leveller::new(70.0).unwrap();
        let mut one = vec![signal.clone(), signal.clone()];
        whole.process(&mut one);
        let mut pieces = Leveller::new(70.0).unwrap();
        let mut joined = vec![Vec::new(), Vec::new()];
        for chunk in signal.chunks(500) {
            let mut block = vec![chunk.to_vec(), chunk.to_vec()];
            pieces.process(&mut block);
            joined[0].extend_from_slice(&block[0]);
            joined[1].extend_from_slice(&block[1]);
        }
        assert_eq!(one, joined);
    }
}
