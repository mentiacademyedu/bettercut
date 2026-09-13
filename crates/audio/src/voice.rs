//! Voice clean-up: rumble taken out, and the room's noise pulled down between
//! the words.
//!
//! # What it is, and what it is not
//!
//! Two stages, both in the time domain:
//!
//! 1. **A high-pass filter at 80 Hz.** Handling noise, traffic rumble and the
//!    low end of mains hum sit below the lowest voice, and removing them
//!    changes nothing anyone says.
//! 2. **A noise-following expander.** It learns the level of the background —
//!    the quietest the signal has lately been — and turns the sound down
//!    whenever it falls back near that level: in the pauses between words and
//!    after sentences, where hiss and room tone are what you hear.
//!
//! It does not take noise out from *under* speech: that needs a spectral
//! method, and a real one is a model rather than a filter. What this removes is
//! the part of the noise people notice most — the hiss that fills every gap —
//! with nothing that can warble or smear a voice.
//!
//! # Blocks do not matter
//!
//! All state carries sample to sample, so processing a second in one block or
//! in a hundred gives the same samples. That is what lets the preview's mixer
//! and the export's, which take different block sizes, agree (§46).

use bettercut_foundation::AUDIO_SAMPLE_RATE as AUDIO_SAMPLE_RATE_HZ;

/// The top of the clean-up slider.
pub const MAX_DENOISE: f32 = 100.0;

/// The deepest the expander cuts the background, at full strength: 24 dB,
/// which takes room hiss from obvious to gone without the silence sounding
/// like a dropout.
const MAX_REDUCTION_DB: f32 = 24.0;

/// How far above the learned noise floor the signal must be to count as
/// something worth keeping: 8 dB.
const OPEN_ABOVE_FLOOR: f32 = 2.5;

/// How long a stretch the background is measured over: the quietest level in
/// the last five seconds. Long enough that a sentence does not raise it — a
/// speaker breathes well within that — short enough to follow a room getting
/// noisier.
const FLOOR_SLOTS: usize = 20;
const FLOOR_SLOT_SECONDS: f32 = 0.25;

/// A tenth of a second: several times the loudness measure's own settling.
const WARM_UP_FRAMES: usize = 4_800;

/// Every sample of a clean-up's state, per channel where it has to be.
#[derive(Debug, Clone)]
pub struct VoiceCleaner {
    amount: f32,
    /// The high-pass, one per channel: (x1, x2, y1, y2).
    history: Vec<[f32; 4]>,
    coefficients: [f32; 5],
    /// Short-term loudness (mean square over about 40 ms) of the loudest
    /// channel.
    power: f32,
    /// The quietest loudness in each recent quarter second, oldest first, and
    /// the one being measured now — the background is the lowest of them.
    slots: [f32; FLOOR_SLOTS],
    slot: usize,
    slot_minimum: f32,
    slot_frames: usize,
    /// Frames seen so far, up to [`WARM_UP_FRAMES`].
    warmed: usize,
    /// The gain applied, smoothed so it never clicks.
    gain: f32,
}

/// One-pole smoothing coefficient for a time constant in seconds.
fn pole(seconds: f32) -> f32 {
    (-1.0 / (seconds * AUDIO_SAMPLE_RATE_HZ as f32)).exp()
}

impl VoiceCleaner {
    /// A clean-up at `amount`, 0–100. `None` at zero: nothing to do, and no
    /// state to keep.
    pub fn new(amount: f32) -> Option<Self> {
        if !amount.is_finite() || amount <= 0.0 {
            return None;
        }
        // RBJ cookbook high-pass, Butterworth Q, at 80 Hz.
        let omega = 2.0 * std::f32::consts::PI * 80.0 / AUDIO_SAMPLE_RATE_HZ as f32;
        let alpha = omega.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let cos = omega.cos();
        let a0 = 1.0 + alpha;
        let coefficients = [
            (1.0 + cos) / 2.0 / a0,
            -(1.0 + cos) / a0,
            (1.0 + cos) / 2.0 / a0,
            -2.0 * cos / a0,
            (1.0 - alpha) / a0,
        ];
        Some(Self {
            amount: amount.min(MAX_DENOISE) / MAX_DENOISE,
            history: Vec::new(),
            coefficients,
            power: 0.0,
            warmed: 0,
            // Unmeasured slots count as loud, so the first moments are not
            // mistaken for silence before the background has been heard.
            slots: [f32::MAX; FLOOR_SLOTS],
            slot: 0,
            slot_minimum: f32::MAX,
            slot_frames: 0,
            gain: 1.0,
        })
    }

    /// Clean `planes` (one per channel) in place, carrying on from the last
    /// call as if the two were one block.
    pub fn process(&mut self, planes: &mut [Vec<f32>]) {
        let channels = planes.len();
        if channels == 0 {
            return;
        }
        if self.history.len() != channels {
            self.history = vec![[0.0; 4]; channels];
        }
        let frames = planes.iter().map(Vec::len).min().unwrap_or(0);
        let [b0, b1, b2, a1, a2] = self.coefficients;

        let smoothing = pole(0.040);
        let slot_length = (FLOOR_SLOT_SECONDS * AUDIO_SAMPLE_RATE_HZ as f32) as usize;
        let gain_open = pole(0.003);
        let gain_close = pole(0.120);
        let deepest = 10_f32.powf(-MAX_REDUCTION_DB * self.amount / 20.0);

        for frame in 0..frames {
            let mut loudest = 0.0_f32;
            for (channel, plane) in planes.iter_mut().enumerate() {
                let [x1, x2, y1, y2] = self.history[channel];
                let x = plane[frame];
                let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                self.history[channel] = [x, x1, y, y1];
                plane[frame] = y;
                loudest = loudest.max(y.abs());
            }

            let square = loudest * loudest;
            self.power = square + (self.power - square) * smoothing;
            let level = self.power.sqrt();

            // Not counted until the measure has settled: from silence it reads
            // as a dead-quiet room for its first few tens of milliseconds, and
            // that would be taken as the background for five seconds.
            if self.warmed < WARM_UP_FRAMES {
                self.warmed += 1;
            } else {
                self.slot_minimum = self.slot_minimum.min(level);
            }
            self.slot_frames += 1;
            if self.slot_frames >= slot_length {
                self.slots[self.slot] = self.slot_minimum;
                self.slot = (self.slot + 1) % FLOOR_SLOTS;
                self.slot_minimum = f32::MAX;
                self.slot_frames = 0;
            }
            let floor = self
                .slots
                .iter()
                .copied()
                .fold(self.slot_minimum, f32::min)
                .max(1e-6);

            // Fully open well above the floor; down to `deepest` at the floor,
            // falling with the square of how close it is.
            let threshold = floor * OPEN_ABOVE_FLOOR;
            let target = if level >= threshold {
                1.0
            } else {
                let closeness = (level / threshold).clamp(0.0, 1.0);
                deepest + (1.0 - deepest) * closeness * closeness
            };
            let coefficient = if target > self.gain {
                gain_open
            } else {
                gain_close
            };
            self.gain = target + (self.gain - target) * coefficient;

            for plane in planes.iter_mut() {
                plane[frame] *= self.gain;
            }
        }
    }
}
