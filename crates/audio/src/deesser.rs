//! The de-esser: the hiss on "s" and "t" dipped, and nothing else touched.
//!
//! A close mic makes sibilance: the 6–8 kHz burst at the front of an "s" comes
//! out two or three times louder than the vowel around it, and every later
//! stage makes it worse — the leveller lifts it, the encoder spends bits on
//! it, headphones sharpen it. A high cut would fix it by taking the top off
//! the whole voice. A de-esser instead turns down *that band, only while the
//! ess is happening*: between the esses the voice is untouched, which is why
//! it can be pushed far harder than an equaliser.
//!
//! How it works here: one band-pass filter per channel picks the sibilant band
//! out, a detector follows how loud that band is, and what comes out is the
//! original with a share of the band subtracted — none of it when the band is
//! quiet, most of it at the peak of an ess. That is a dynamic band cut without
//! recomputing filter coefficients per sample, so it cannot ring or go
//! unstable however fast the control moves.
//!
//! The detector follows the loudest channel and one gain is applied to all of
//! them, as the leveller's does, so a stereo voice dips together and its image
//! does not wander. All state carries from block to block: a second processed
//! in one block or a hundred is the same samples (§46).

use bettercut_foundation::AUDIO_SAMPLE_RATE;

/// The middle of the sibilant band. Between "s" (higher, sharper) and the
/// softer "sh" below it, which is where a voice's esses sit whoever is
/// speaking.
pub const ESS_HZ: f32 = 7_000.0;

/// How wide the band is: about an octave, so roughly 5–10 kHz at the skirts
/// and 6–8 kHz where the dip does its work. Narrower and a deep voice's esses
/// slip past it; wider and it starts to take the air off the whole voice.
const ESS_Q: f32 = 1.4;

/// The most the band is ever dipped, in dB, at full amount. Past this a voice
/// starts to lisp — the ess is gone rather than tamed.
pub const MAX_REDUCTION_DB: f32 = 18.0;

/// Quick enough to catch the front of an ess, and let go before the vowel
/// after it. Both are shorter than the leveller's: an ess is a moment.
const ATTACK_SECONDS: f32 = 0.001;
const RELEASE_SECONDS: f32 = 0.04;

fn coefficient(seconds: f32) -> f32 {
    (-1.0 / (seconds * AUDIO_SAMPLE_RATE as f32)).exp()
}

/// The RBJ band-pass with unity gain at its middle, so the band it hands back
/// is the part of the signal the dip is meant to remove — subtract all of it
/// and a tone at [`ESS_HZ`] disappears.
fn band_pass(hz: f32, q: f32) -> [f32; 5] {
    let w = 2.0 * std::f32::consts::PI * hz / AUDIO_SAMPLE_RATE as f32;
    let (cos, alpha) = (w.cos(), w.sin() / (2.0 * q));
    let a0 = 1.0 + alpha;
    [
        alpha / a0,
        0.0,
        -alpha / a0,
        -2.0 * cos / a0,
        (1.0 - alpha) / a0,
    ]
}

/// A de-esser for one clip.
pub struct DeEsser {
    band: [f32; 5],
    threshold_db: f32,
    ratio: f32,
    floor_db: f32,
    attack: f32,
    release: f32,
    /// Per channel: (x1, x2, y1, y2) of the band-pass.
    history: Vec<[f32; 4]>,
    /// The detector's current level, in dB.
    envelope_db: f32,
}

impl DeEsser {
    /// A de-esser at `amount` (0–100). `None` for no amount, which is the
    /// common case and worth not running at all.
    ///
    /// The amount moves two things at once, because the one control has to
    /// mean "de-ess harder": the level the dip starts at comes down, and how
    /// deep it goes rises.
    pub fn new(amount: f32) -> Option<Self> {
        if !amount.is_finite() || amount <= 0.0 {
            return None;
        }
        let share = (amount / 100.0).min(1.0);
        Some(Self {
            band: band_pass(ESS_HZ, ESS_Q),
            // -12 dBFS in the band is a loud ess on a well-recorded voice;
            // -36 catches a quiet one, and everything between follows the
            // control.
            threshold_db: -12.0 - 24.0 * share,
            ratio: 1.0 + 5.0 * share,
            floor_db: -(3.0 + (MAX_REDUCTION_DB - 3.0) * share),
            attack: coefficient(ATTACK_SECONDS),
            release: coefficient(RELEASE_SECONDS),
            history: Vec::new(),
            envelope_db: -120.0,
        })
    }

    /// De-ess `planes` (one per channel) in place, carrying on from the last
    /// call as if the two were one block.
    pub fn process(&mut self, planes: &mut [Vec<f32>]) {
        let channels = planes.len();
        if self.history.len() != channels {
            self.history = vec![[0.0; 4]; channels];
        }
        let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
        let [b0, b1, b2, a1, a2] = self.band;
        let mut band = vec![0.0_f32; channels];

        for frame in 0..frames {
            // The sibilant band of this frame, channel by channel.
            for (channel, plane) in planes.iter().enumerate() {
                let [x1, x2, y1, y2] = self.history[channel];
                let x = plane[frame];
                let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                self.history[channel] = [x, x1, y, y1];
                band[channel] = y;
            }

            let peak = band.iter().fold(0.0_f32, |most, y| most.max(y.abs()));
            let level_db = 20.0 * peak.max(1e-6).log10();
            let coefficient = if level_db > self.envelope_db {
                self.attack
            } else {
                self.release
            };
            self.envelope_db = level_db + coefficient * (self.envelope_db - level_db);

            let over = self.envelope_db - self.threshold_db;
            let reduction_db = if over > 0.0 {
                (over * (1.0 - 1.0 / self.ratio)).min(-self.floor_db)
            } else {
                0.0
            };
            if reduction_db <= 0.0 {
                continue;
            }
            // What is left of the band after the dip, taken off the signal:
            // at the middle of the band that is the dip itself, and away from
            // it there is nothing to take.
            let share = 1.0 - 10.0_f32.powf(-reduction_db / 20.0);
            for (plane, y) in planes.iter_mut().zip(band.iter()) {
                plane[frame] -= share * y;
            }
        }
    }

    /// How much the band is being dipped right now, in dB (0 when it is not).
    /// For a meter beside the control: a de-esser that never moves is doing
    /// nothing, and one that never lets go is set too hard.
    pub fn reduction_db(&self) -> f32 {
        let over = self.envelope_db - self.threshold_db;
        if over <= 0.0 {
            return 0.0;
        }
        (over * (1.0 - 1.0 / self.ratio)).min(-self.floor_db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A steady tone at `hz`, `frames` long.
    fn tone(hz: f32, amplitude: f32, frames: usize) -> Vec<f32> {
        let step = 2.0 * std::f32::consts::PI * hz / AUDIO_SAMPLE_RATE as f32;
        (0..frames)
            .map(|i| (i as f32 * step).sin() * amplitude)
            .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0_f32, |most, s| most.max(s.abs()))
    }

    /// The peak of the second half, once the detector has settled.
    fn settled(planes: &[Vec<f32>]) -> f32 {
        peak(&planes[0][planes[0].len() / 2..])
    }

    fn through(amount: f32, signal: Vec<f32>) -> Vec<Vec<f32>> {
        let mut planes = vec![signal];
        if let Some(mut de_esser) = DeEsser::new(amount) {
            de_esser.process(&mut planes);
        }
        planes
    }

    #[test]
    fn no_amount_is_nothing_to_do() {
        assert!(DeEsser::new(0.0).is_none());
        assert!(DeEsser::new(-5.0).is_none());
        assert!(DeEsser::new(f32::NAN).is_none());
    }

    /// The whole point: a loud ess is turned down and the voice under it is
    /// not.
    #[test]
    fn an_ess_is_dipped_and_the_voice_is_not() {
        let frames = AUDIO_SAMPLE_RATE as usize / 4;
        let ess = settled(&through(100.0, tone(ESS_HZ, 0.6, frames)));
        let voice = settled(&through(100.0, tone(300.0, 0.6, frames)));

        assert!(ess < 0.6 * 0.5, "the ess was not dipped: {ess}");
        assert!(
            voice > 0.6 * 0.95,
            "the voice under the ess was touched: {voice}"
        );
    }

    /// And the harder the control, the deeper the dip — but never past the
    /// point where the ess would vanish altogether.
    #[test]
    fn pushing_harder_dips_further_but_not_away() {
        let frames = AUDIO_SAMPLE_RATE as usize / 4;
        let gentle = settled(&through(25.0, tone(ESS_HZ, 0.6, frames)));
        let hard = settled(&through(100.0, tone(ESS_HZ, 0.6, frames)));

        assert!(
            hard < gentle,
            "pushing harder dipped less: {hard}, {gentle}"
        );
        let floor = 0.6 * 10.0_f32.powf(-MAX_REDUCTION_DB / 20.0) * 0.8;
        assert!(
            hard > floor,
            "the ess was removed rather than tamed: {hard}"
        );
    }

    /// A quiet ess is under the threshold, and a de-esser that dips those is
    /// just a high cut.
    #[test]
    fn a_quiet_ess_is_left_alone() {
        let frames = AUDIO_SAMPLE_RATE as usize / 4;
        let quiet = settled(&through(50.0, tone(ESS_HZ, 0.004, frames)));
        assert!(
            quiet > 0.004 * 0.95,
            "a quiet ess was dipped: {quiet} of 0.004"
        );
    }

    /// It lets go: the ess is dipped, the vowel after it is not.
    #[test]
    fn the_dip_lets_go_after_the_ess() {
        let frames = AUDIO_SAMPLE_RATE as usize / 10;
        let mut signal = tone(ESS_HZ, 0.7, frames);
        signal.extend(tone(600.0, 0.7, frames));
        let planes = through(100.0, signal);

        let after = peak(&planes[0][frames + frames / 2..]);
        assert!(
            after > 0.7 * 0.95,
            "the voice after the ess was still held down: {after}"
        );
    }

    /// Two channels dip by the same amount, so a stereo voice does not lean to
    /// one side every time it says "so".
    #[test]
    fn stereo_dips_together() {
        let frames = AUDIO_SAMPLE_RATE as usize / 4;
        let loud = tone(ESS_HZ, 0.6, frames);
        let quiet: Vec<f32> = loud.iter().map(|s| s * 0.5).collect();
        let mut planes = vec![loud, quiet];
        DeEsser::new(100.0).unwrap().process(&mut planes);

        let (left, right) = (settled(&planes), peak(&planes[1][frames / 2..]));
        let ratio = left / right.max(1e-9);
        assert!(
            (ratio - 2.0).abs() < 0.15,
            "the channels came out {ratio}:1, not 2:1"
        );
    }

    /// One block or many: the same samples.
    #[test]
    fn blocks_join_seamlessly() {
        let signal: Vec<f32> = (0..9_000)
            .map(|i| {
                let ess = (i as f32 * 0.9).sin() * 0.7;
                let voice = (i as f32 * 0.04).sin() * 0.4;
                if i % 3_000 < 1_500 {
                    ess + voice
                } else {
                    voice
                }
            })
            .collect();

        let mut whole = DeEsser::new(70.0).unwrap();
        let mut one = vec![signal.clone(), signal.clone()];
        whole.process(&mut one);

        let mut pieces = DeEsser::new(70.0).unwrap();
        let mut joined = vec![Vec::new(), Vec::new()];
        for chunk in signal.chunks(500) {
            let mut block = vec![chunk.to_vec(), chunk.to_vec()];
            pieces.process(&mut block);
            joined[0].extend_from_slice(&block[0]);
            joined[1].extend_from_slice(&block[1]);
        }
        assert_eq!(one, joined);
    }

    /// The meter reads nothing until there is an ess, and something while
    /// there is one.
    #[test]
    fn the_meter_reads_the_dip() {
        let frames = AUDIO_SAMPLE_RATE as usize / 10;
        let mut de_esser = DeEsser::new(100.0).unwrap();
        let mut quiet = vec![tone(300.0, 0.5, frames)];
        de_esser.process(&mut quiet);
        assert_eq!(de_esser.reduction_db(), 0.0);

        let mut loud = vec![tone(ESS_HZ, 0.6, frames)];
        de_esser.process(&mut loud);
        assert!(
            de_esser.reduction_db() > 1.0,
            "the meter did not move: {}",
            de_esser.reduction_db()
        );
    }
}
