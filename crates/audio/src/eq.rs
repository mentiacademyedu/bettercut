//! A three-control equaliser for one recording: low cut, high cut, presence.
//!
//! The three things a person actually reaches for on a clip. A low cut takes
//! out rumble and boom below a voice; a high cut softens hiss and harshness; a
//! presence band around 3 kHz brings a voice forward — or, turned down, sets a
//! loud one back. Anything finer is a mixing desk, and a short-form editor is
//! not one.
//!
//! Each control is an RBJ-cookbook biquad, run in series, and all state carries
//! sample to sample: a second processed in one block or in a hundred is the
//! same samples, which is what lets the preview's mixer and the export's agree
//! (§46).

use bettercut_foundation::AUDIO_SAMPLE_RATE;

/// Where the presence band sits: the middle of where consonants and a
/// voice's clarity live.
pub const PRESENCE_HZ: f32 = 3_000.0;

/// How many harmonics of the mains a hum filter takes out, counting the
/// fundamental.
///
/// Mains hum is never one tone: the fundamental at 50 or 60 Hz comes with
/// harmonics at twice and three times it, and notching only the first leaves
/// the buzz — the part people describe as "electrical" rather than "low".
/// Past the third there is little energy and a lot of voice, so it stops.
pub const HUM_HARMONICS: usize = 3;

/// How narrow each hum notch is. High, because hum is a *tone*: a wide notch
/// at 50 Hz would take the bottom out of a voice along with it.
const HUM_Q: f32 = 30.0;

/// How wide the presence band is: about an octave and a half.
const PRESENCE_Q: f32 = 1.0;

/// Butterworth: the flattest pass band, no bump before the cut.
const CUT_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// One filter's coefficients, normalised: `[b0, b1, b2, a1, a2]`.
type Coefficients = [f32; 5];

fn omega(hz: f32) -> f32 {
    2.0 * std::f32::consts::PI * hz / AUDIO_SAMPLE_RATE as f32
}

fn high_pass(hz: f32) -> Coefficients {
    let w = omega(hz);
    let (cos, alpha) = (w.cos(), w.sin() / (2.0 * CUT_Q));
    let a0 = 1.0 + alpha;
    [
        (1.0 + cos) / 2.0 / a0,
        -(1.0 + cos) / a0,
        (1.0 + cos) / 2.0 / a0,
        -2.0 * cos / a0,
        (1.0 - alpha) / a0,
    ]
}

fn low_pass(hz: f32) -> Coefficients {
    let w = omega(hz);
    let (cos, alpha) = (w.cos(), w.sin() / (2.0 * CUT_Q));
    let a0 = 1.0 + alpha;
    [
        (1.0 - cos) / 2.0 / a0,
        (1.0 - cos) / a0,
        (1.0 - cos) / 2.0 / a0,
        -2.0 * cos / a0,
        (1.0 - alpha) / a0,
    ]
}

fn peaking(hz: f32, gain_db: f32) -> Coefficients {
    let w = omega(hz);
    let (cos, alpha) = (w.cos(), w.sin() / (2.0 * PRESENCE_Q));
    let a = 10_f32.powf(gain_db / 40.0);
    let a0 = 1.0 + alpha / a;
    [
        (1.0 + alpha * a) / a0,
        -2.0 * cos / a0,
        (1.0 - alpha * a) / a0,
        -2.0 * cos / a0,
        (1.0 - alpha / a) / a0,
    ]
}

/// A notch: a narrow band removed, everything either side left alone. The
/// RBJ cookbook's band-stop, which is a peaking filter taken to nothing.
fn notch(hz: f32, q: f32) -> Coefficients {
    let w = omega(hz);
    let (cos, alpha) = (w.cos(), w.sin() / (2.0 * q));
    let a0 = 1.0 + alpha;
    [
        1.0 / a0,
        -2.0 * cos / a0,
        1.0 / a0,
        -2.0 * cos / a0,
        (1.0 - alpha) / a0,
    ]
}

/// An equaliser's filters and every sample of their state.
#[derive(Debug, Clone)]
pub struct Equalizer {
    stages: Vec<Coefficients>,
    /// Per stage, per channel: (x1, x2, y1, y2).
    history: Vec<Vec<[f32; 4]>>,
}

impl Equalizer {
    /// An equaliser with a low cut at `low_cut_hz`, a high cut at
    /// `high_cut_hz` and `presence_db` of presence; a zero (or non-finite)
    /// setting leaves that control out. `None` when all three are out: flat,
    /// with nothing to run.
    pub fn new(low_cut_hz: f32, high_cut_hz: f32, presence_db: f32) -> Option<Self> {
        Self::with_hum(low_cut_hz, high_cut_hz, presence_db, 0.0)
    }

    /// The same, with mains hum notched out at `hum_hz` and its harmonics
    /// ([`HUM_HARMONICS`]). Zero — or a rate the samples cannot hold — leaves
    /// the hum filter out.
    pub fn with_hum(
        low_cut_hz: f32,
        high_cut_hz: f32,
        presence_db: f32,
        hum_hz: f32,
    ) -> Option<Self> {
        let nyquist = AUDIO_SAMPLE_RATE as f32 / 2.0;
        let mut stages = Vec::new();
        if hum_hz.is_finite() && hum_hz > 0.0 {
            for harmonic in 1..=HUM_HARMONICS {
                let hz = hum_hz * harmonic as f32;
                if hz < nyquist * 0.9 {
                    stages.push(notch(hz, HUM_Q));
                }
            }
        }
        if low_cut_hz.is_finite() && low_cut_hz > 0.0 {
            stages.push(high_pass(low_cut_hz.min(nyquist * 0.9)));
        }
        if high_cut_hz.is_finite() && high_cut_hz > 0.0 && high_cut_hz < nyquist * 0.9 {
            stages.push(low_pass(high_cut_hz));
        }
        if presence_db.is_finite() && presence_db.abs() >= 0.05 {
            stages.push(peaking(PRESENCE_HZ, presence_db));
        }
        if stages.is_empty() {
            return None;
        }
        let history = vec![Vec::new(); stages.len()];
        Some(Self { stages, history })
    }

    /// Filter `planes` (one per channel) in place, carrying on from the last
    /// call as if the two were one block.
    pub fn process(&mut self, planes: &mut [Vec<f32>]) {
        let channels = planes.len();
        for (stage, history) in self.stages.iter().zip(self.history.iter_mut()) {
            if history.len() != channels {
                *history = vec![[0.0; 4]; channels];
            }
            let [b0, b1, b2, a1, a2] = *stage;
            for (plane, state) in planes.iter_mut().zip(history.iter_mut()) {
                let [mut x1, mut x2, mut y1, mut y2] = *state;
                for sample in plane.iter_mut() {
                    let x = *sample;
                    let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                    (x2, x1, y2, y1) = (x1, x, y1, y);
                    *sample = y;
                }
                *state = [x1, x2, y1, y2];
            }
        }
    }
}
