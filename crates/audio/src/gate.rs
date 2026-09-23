//! The noise gate: between the words, the room goes quiet.
//!
//! The voice cleaner (`crate::voice`) takes the hiss out *from under* a
//! voice, which is the hard problem. This is the easy one, and the older
//! tool: when nobody is speaking, turn the whole thing down. A fan, a fridge,
//! traffic, the mic's own hum — all of it sits below the voice, and a gate
//! that opens for the voice and closes behind it takes the lot out of the
//! pauses without touching a word.
//!
//! What makes a gate usable rather than a stutter: it opens fast (a clipped
//! first consonant is worse than a little noise), holds open for a moment
//! after the level drops (so the gaps inside a word do not flap it shut),
//! closes slowly, and never closes all the way — a pause that goes to digital
//! silence sounds like the recording stopped. One amount, 0–100, sets how
//! high the gate's threshold is and how far it closes. All state carries from
//! block to block, so the preview and the export agree (§46).

use bettercut_foundation::AUDIO_SAMPLE_RATE;

/// Where the gate's threshold sits at the gentlest and the hardest setting,
/// in dBFS. -60 catches only the quietest room; -30 will start to catch soft
/// speech, which is what the top of the control is for — a bad room.
const THRESHOLD_GENTLE_DB: f32 = -60.0;
const THRESHOLD_HARD_DB: f32 = -30.0;

/// How far the gate closes at the hardest setting, in dB. Not to nothing:
/// a pause that drops to digital silence reads as a dropout.
pub const MAX_CLOSED_DB: f32 = 36.0;

const ATTACK_SECONDS: f32 = 0.002;
const HOLD_SECONDS: f32 = 0.08;
const RELEASE_SECONDS: f32 = 0.15;

fn coefficient(seconds: f32) -> f32 {
    (-1.0 / (seconds * AUDIO_SAMPLE_RATE as f32)).exp()
}

/// A noise gate for one clip.
pub struct Gate {
    threshold_db: f32,
    /// The gain when shut, linear.
    closed: f32,
    attack: f32,
    release: f32,
    /// How many samples the gate stays open after the level drops.
    hold_samples: u32,
    /// The detector's level, in dB.
    envelope_db: f32,
    /// Samples of hold left, counting down once the level is under.
    hold_left: u32,
    /// Where the gain is right now, moving towards open or shut.
    gain: f32,
}

impl Gate {
    /// A gate at `amount` (0–100). `None` for no amount.
    pub fn new(amount: f32) -> Option<Self> {
        if !amount.is_finite() || amount <= 0.0 {
            return None;
        }
        let share = (amount / 100.0).min(1.0);
        let closed_db = -(6.0 + (MAX_CLOSED_DB - 6.0) * share);
        Some(Self {
            threshold_db: THRESHOLD_GENTLE_DB + (THRESHOLD_HARD_DB - THRESHOLD_GENTLE_DB) * share,
            closed: 10.0_f32.powf(closed_db / 20.0),
            attack: coefficient(ATTACK_SECONDS),
            release: coefficient(RELEASE_SECONDS),
            hold_samples: (HOLD_SECONDS * AUDIO_SAMPLE_RATE as f32) as u32,
            envelope_db: -120.0,
            hold_left: 0,
            gain: 1.0,
        })
    }

    /// Gate `planes` (one per channel) in place, carrying on from the last
    /// call as if the two were one block. The detector follows the loudest
    /// channel and one gain goes to all of them, so a stereo pause closes
    /// together.
    pub fn process(&mut self, planes: &mut [Vec<f32>]) {
        let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
        for frame in 0..frames {
            let peak = planes
                .iter()
                .map(|plane| plane[frame].abs())
                .fold(0.0_f32, f32::max);
            let level_db = 20.0 * peak.max(1e-6).log10();
            // The detector rises at once and falls slowly, as a meter does,
            // so a word's own dips do not read as its end.
            self.envelope_db = if level_db > self.envelope_db {
                level_db
            } else {
                level_db + self.release * (self.envelope_db - level_db)
            };

            let open = self.envelope_db > self.threshold_db;
            if open {
                self.hold_left = self.hold_samples;
            } else if self.hold_left > 0 {
                self.hold_left -= 1;
            }
            let target = if open || self.hold_left > 0 {
                1.0
            } else {
                self.closed
            };
            // Opening is fast, closing is slow: the two coefficients.
            let coefficient = if target > self.gain {
                self.attack
            } else {
                self.release
            };
            self.gain = target + coefficient * (self.gain - target);

            for plane in planes.iter_mut() {
                plane[frame] *= self.gain;
            }
        }
    }

    /// Whether the gate is open right now, for a light beside the control.
    pub fn is_open(&self) -> bool {
        self.gain > 0.5
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
        samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn no_amount_is_nothing_to_do() {
        assert!(Gate::new(0.0).is_none());
        assert!(Gate::new(f32::NAN).is_none());
    }

    /// The whole point: a voice passes, the quiet room between words does
    /// not — but is not silenced either.
    #[test]
    fn a_voice_passes_and_the_room_between_is_turned_down() {
        let second = AUDIO_SAMPLE_RATE as usize;
        let mut gate = Gate::new(70.0).unwrap();
        let mut signal = tone(0.5, second / 2);
        signal.extend(tone(0.002, second));
        let mut planes = vec![signal];
        gate.process(&mut planes);

        let voice = peak(&planes[0][second / 4..second / 2]);
        assert!((voice - 0.5).abs() < 0.01, "the voice was touched: {voice}");
        // Well after the hold and the release: the room is down, not gone.
        let room = peak(&planes[0][second + second / 4..]);
        assert!(room < 0.002 * 0.2, "the room was not turned down: {room}");
        assert!(room > 0.0, "the room went to nothing");
    }

    /// The gate holds open for a moment after the level drops, so the gap
    /// inside a word does not shut it.
    #[test]
    fn a_short_gap_inside_a_word_does_not_shut_the_gate() {
        let mut gate = Gate::new(70.0).unwrap();
        let mut signal = tone(0.5, 4_800);
        signal.extend(vec![0.0; 1_200]); // 25 ms of nothing
        signal.extend(tone(0.5, 4_800));
        let mut planes = vec![signal];
        gate.process(&mut planes);
        // The first samples after the gap are at full level: the gate was
        // still open.
        let after = peak(&planes[0][6_000..6_200]);
        assert!(after > 0.45, "the gate shut inside the word: {after}");
    }

    /// Harder is a higher threshold and a deeper close.
    #[test]
    fn pushing_harder_closes_further() {
        let second = AUDIO_SAMPLE_RATE as usize;
        let room = |amount: f32| {
            let mut gate = Gate::new(amount).unwrap();
            let mut planes = vec![tone(0.003, second)];
            gate.process(&mut planes);
            peak(&planes[0][second * 3 / 4..])
        };
        assert!(
            room(100.0) < room(30.0),
            "{} vs {}",
            room(100.0),
            room(30.0)
        );
    }

    /// One block or many: the same samples.
    #[test]
    fn blocks_join_seamlessly() {
        let signal: Vec<f32> = (0..12_000)
            .map(|i| (i as f32 * 0.05).sin() * if i % 4_000 < 2_000 { 0.6 } else { 0.001 })
            .collect();
        let mut whole = Gate::new(60.0).unwrap();
        let mut one = vec![signal.clone(), signal.clone()];
        whole.process(&mut one);
        let mut pieces = Gate::new(60.0).unwrap();
        let mut joined = vec![Vec::new(), Vec::new()];
        for chunk in signal.chunks(700) {
            let mut block = vec![chunk.to_vec(), chunk.to_vec()];
            pieces.process(&mut block);
            joined[0].extend_from_slice(&block[0]);
            joined[1].extend_from_slice(&block[1]);
        }
        assert_eq!(one, joined);
    }
}
