//! Loudness, the way platforms measure it (ITU-R BS.1770 / EBU R128).
//!
//! Peak normalisation (`bettercut_playback::normalise`) answers "is this clip
//! too quiet"; this answers the different question the *finished mix* is
//! judged by. Every platform now turns a video up or down on delivery to hit
//! its own target — around -14 LUFS for YouTube and Spotify, -16 for Apple,
//! -23 for broadcast — so a mix left at -8 is quietly turned down, and one at
//! -24 is turned up along with its noise floor. Knowing the number before the
//! export means the decision is the editor's.
//!
//! # How it is measured
//!
//! Two filters that stand in for a head and ears (a high shelf, then a
//! high-pass), the mean square over 400 ms blocks overlapping by three
//! quarters, and then two gates: everything below -70 LUFS is silence and does
//! not count, and everything more than 10 LU below the average of what is left
//! is a quiet passage and does not count either. What survives is the
//! integrated loudness.
//!
//! The filter coefficients are the standard's own, at 48 kHz — which is the
//! only rate anything reaches here, because §20a.3 has the decoders resample
//! everything to it.

/// The rate the coefficients below are for, and the rate §20a.3 normalises to.
pub const SAMPLE_RATE: u32 = 48_000;

/// Below this, a block is silence and is left out of the measurement.
const ABSOLUTE_GATE: f32 = -70.0;
/// And the second gate: this far below the ungated average.
const RELATIVE_GATE: f32 = -10.0;

/// A block, in seconds, and how far each one starts after the last.
const BLOCK_SECONDS: f32 = 0.4;
const STEP_SECONDS: f32 = 0.1;

/// One biquad, direct form I.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    const fn new(b: [f32; 3], a: [f32; 2]) -> Self {
        Self {
            b,
            a,
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn run(&mut self, sample: f32) -> f32 {
        let out = self.b[0] * sample + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [sample, self.x[0]];
        self.y = [out, self.y[0]];
        out
    }
}

/// The standard's "K" weighting: a high shelf standing in for the head, then a
/// high-pass standing in for how little the ear makes of the very bottom.
fn k_weighting() -> [Biquad; 2] {
    [
        // Stage 1, the shelving filter (BS.1770-4 table 1, 48 kHz).
        Biquad::new(
            [1.530_841_2, -2.650_979_7, 1.169_079_7],
            [-1.663_635_3, 0.712_699_5],
        ),
        // Stage 2, the RLB high-pass (table 2).
        Biquad::new([1.0, -2.0, 1.0], [-1.990_049_5, 0.990_072_2]),
    ]
}

/// Measures the loudness of everything pushed through it.
///
/// Mono or stereo: every channel is weighted the same way and summed, which is
/// what the standard does for the front pair.
#[derive(Debug)]
pub struct LoudnessMeter {
    filters: Vec<[Biquad; 2]>,
    /// Mean square of each 400 ms block, in the order they were measured.
    blocks: Vec<f32>,
    /// The current block's running sum and how many samples are in it.
    window: Vec<f32>,
    at: usize,
    block_samples: usize,
    step_samples: usize,
    since_block: usize,
    /// How many frames have been pushed: a block is only measured once there
    /// is a whole one behind us.
    pushed: usize,
}

impl LoudnessMeter {
    /// A meter for `channels` channels at [`SAMPLE_RATE`].
    pub fn new(channels: usize) -> Self {
        let block_samples = (BLOCK_SECONDS * SAMPLE_RATE as f32) as usize;
        Self {
            filters: (0..channels.max(1)).map(|_| k_weighting()).collect(),
            blocks: Vec::new(),
            window: vec![0.0; block_samples],
            at: 0,
            block_samples,
            step_samples: (STEP_SECONDS * SAMPLE_RATE as f32) as usize,
            since_block: 0,
            pushed: 0,
        }
    }

    /// Push one frame: one sample per channel.
    pub fn push(&mut self, frame: &[f32]) {
        // The weighted sum across channels is what a block measures.
        let mut sum = 0.0;
        for (channel, sample) in frame.iter().enumerate() {
            let Some(filters) = self.filters.get_mut(channel) else {
                continue;
            };
            let [shelf, high_pass] = filters;
            let weighted = high_pass.run(shelf.run(*sample));
            sum += weighted * weighted;
        }
        self.window[self.at] = sum;
        self.at = (self.at + 1) % self.block_samples;
        self.since_block += 1;
        self.pushed += 1;
        // A block every step, once there is a whole block's worth behind us.
        if self.since_block >= self.step_samples && self.pushed >= self.block_samples {
            self.since_block = 0;
            let mean = self.window.iter().sum::<f32>() / self.block_samples as f32;
            self.blocks.push(mean);
        }
    }

    /// Push interleaved samples: `channels` per frame.
    pub fn push_interleaved(&mut self, samples: &[f32], channels: usize) {
        let channels = channels.max(1);
        for frame in samples.chunks(channels) {
            self.push(frame);
        }
    }

    /// The integrated loudness in LUFS, or `None` when nothing but silence was
    /// measured — or when there was less than one block of it.
    pub fn integrated(&self) -> Option<f32> {
        if self.blocks.is_empty() {
            return None;
        }
        let loudness = |mean: f32| -0.691 + 10.0 * mean.max(1e-12).log10();
        // The first gate: silence is not part of the average.
        let loud: Vec<f32> = self
            .blocks
            .iter()
            .copied()
            .filter(|mean| loudness(*mean) > ABSOLUTE_GATE)
            .collect();
        if loud.is_empty() {
            return None;
        }
        let mean = loud.iter().sum::<f32>() / loud.len() as f32;
        // And the second: the quiet passages of what is left.
        let threshold = loudness(mean) + RELATIVE_GATE;
        let kept: Vec<f32> = loud
            .into_iter()
            .filter(|mean| loudness(*mean) > threshold)
            .collect();
        if kept.is_empty() {
            return None;
        }
        Some(loudness(kept.iter().sum::<f32>() / kept.len() as f32))
    }
}

/// How much sound each step of the live meter covers: 100 ms, the standard's
/// own step.
const LIVE_STEP_SAMPLES: usize = SAMPLE_RATE as usize / 10;
/// Steps in the momentary window (400 ms) and in the short-term one (3 s).
const MOMENTARY_STEPS: usize = 4;
const SHORT_TERM_STEPS: usize = 30;

/// The meter for *playback*: what the mix is doing right now, rather than what
/// the whole edit adds up to.
///
/// [`LoudnessMeter`] answers the delivery question and keeps every block it
/// has ever seen to do it — which is right for a measurement made once, and
/// wrong for the audio thread, where §20a.2 forbids allocating at all. So this
/// keeps a fixed ring of 100 ms means: thirty of them is the three seconds the
/// short-term reading is defined over, and the last four are the 400 ms the
/// momentary one is.
///
/// Both readings are gated the same way the standard gates silence: below
/// -70 LUFS there is nothing to report, and a meter that printed "-inf" while
/// the mix was simply quiet would be read as a fault.
#[derive(Debug)]
pub struct LiveLoudness {
    filters: Vec<[Biquad; 2]>,
    /// Mean square of each finished 100 ms step, newest last by `at`.
    steps: [f32; SHORT_TERM_STEPS],
    at: usize,
    filled: usize,
    /// The step being filled.
    partial: f32,
    in_step: usize,
}

impl LiveLoudness {
    pub fn new(channels: usize) -> Self {
        Self {
            filters: (0..channels.max(1)).map(|_| k_weighting()).collect(),
            steps: [0.0; SHORT_TERM_STEPS],
            at: 0,
            filled: 0,
            partial: 0.0,
            in_step: 0,
        }
    }

    /// Push one frame: one sample per channel. No allocation, no branching on
    /// anything but the step boundary.
    pub fn push(&mut self, frame: &[f32]) {
        let mut sum = 0.0;
        for (channel, sample) in frame.iter().enumerate() {
            let Some(filters) = self.filters.get_mut(channel) else {
                continue;
            };
            let [shelf, high_pass] = filters;
            let weighted = high_pass.run(shelf.run(*sample));
            sum += weighted * weighted;
        }
        self.partial += sum;
        self.in_step += 1;
        if self.in_step >= LIVE_STEP_SAMPLES {
            self.steps[self.at] = self.partial / LIVE_STEP_SAMPLES as f32;
            self.at = (self.at + 1) % SHORT_TERM_STEPS;
            self.filled = (self.filled + 1).min(SHORT_TERM_STEPS);
            self.partial = 0.0;
            self.in_step = 0;
        }
    }

    pub fn push_interleaved(&mut self, samples: &[f32], channels: usize) {
        let channels = channels.max(1);
        for frame in samples.chunks(channels) {
            self.push(frame);
        }
    }

    /// The last 400 ms, in LUFS. `None` until there is that much, or when what
    /// there is counts as silence.
    pub fn momentary(&self) -> Option<f32> {
        self.window(MOMENTARY_STEPS)
    }

    /// The last three seconds, in LUFS — the reading a mixer watches, since it
    /// moves with the mix rather than with every syllable.
    pub fn short_term(&self) -> Option<f32> {
        self.window(SHORT_TERM_STEPS)
    }

    /// Forget everything: playback stopped, and the next reading is about
    /// wherever it starts again rather than about what was playing before.
    pub fn reset(&mut self) {
        self.steps = [0.0; SHORT_TERM_STEPS];
        self.at = 0;
        self.filled = 0;
        self.partial = 0.0;
        self.in_step = 0;
        for filters in &mut self.filters {
            *filters = k_weighting();
        }
    }

    fn window(&self, steps: usize) -> Option<f32> {
        let count = self.filled.min(steps);
        if count < MOMENTARY_STEPS {
            return None;
        }
        let mut sum = 0.0;
        for back in 1..=count {
            let index = (self.at + SHORT_TERM_STEPS - back) % SHORT_TERM_STEPS;
            sum += self.steps[index];
        }
        let loudness = -0.691 + 10.0 * (sum / count as f32).max(1e-12).log10();
        (loudness > ABSOLUTE_GATE).then_some(loudness)
    }
}

/// The gain, as a multiplier, that moves a mix measured at `measured` LUFS to
/// `target` LUFS.
///
/// Held to ±20 dB: a mix twenty dB from its target is not a level problem, and
/// multiplying a noise floor by ten helps nobody.
pub fn gain_for(measured: f32, target: f32) -> f32 {
    let difference = (target - measured).clamp(-20.0, 20.0);
    10.0_f32.powf(difference / 20.0)
}

/// The targets worth offering, and what each is for.
pub const LOUDNESS_TARGETS: [(&str, f32); 4] = [
    ("YouTube, Spotify", -14.0),
    ("Apple Music", -16.0),
    ("TikTok, Instagram", -14.0),
    ("Broadcast (EBU R128)", -23.0),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine at `amplitude`, `seconds` long, through the meter.
    fn sine(amplitude: f32, seconds: f32) -> Option<f32> {
        let mut meter = LoudnessMeter::new(1);
        let samples = (seconds * SAMPLE_RATE as f32) as usize;
        for n in 0..samples {
            let t = n as f32 / SAMPLE_RATE as f32;
            meter.push(&[amplitude * (std::f32::consts::TAU * 1000.0 * t).sin()]);
        }
        meter.integrated()
    }

    #[test]
    fn silence_measures_nothing() {
        let mut meter = LoudnessMeter::new(2);
        for _ in 0..48_000 {
            meter.push(&[0.0, 0.0]);
        }
        assert_eq!(meter.integrated(), None);
    }

    #[test]
    fn too_little_to_measure_says_so() {
        // A tenth of a second: less than one block.
        assert_eq!(sine(0.5, 0.1), None);
    }

    /// Twice the amplitude is six more LU, whatever the level.
    #[test]
    fn doubling_the_level_adds_six_lu() {
        let quiet = sine(0.1, 3.0).expect("a measurement");
        let loud = sine(0.2, 3.0).expect("a measurement");
        assert!(
            (loud - quiet - 6.02).abs() < 0.2,
            "{quiet} then {loud}, a difference of {}",
            loud - quiet
        );
    }

    /// A 1 kHz sine at -20 dBFS measures about -23.2 LUFS: the standard's own
    /// -0.691 offset, the 3 dB a sine's mean square sits below its peak, and
    /// the half a dB the K-weighting's shelf adds at 1 kHz.
    #[test]
    fn a_known_tone_measures_where_the_standard_says() {
        let measured = sine(0.1, 3.0).expect("a measurement");
        assert!(
            (measured + 23.2).abs() < 0.4,
            "a -20 dBFS tone measured {measured} LUFS"
        );
    }

    /// A tone through the live meter reads what the same tone reads through
    /// the measuring one: two meters that disagreed would be two answers to
    /// one question.
    #[test]
    fn the_live_meter_agrees_with_the_measured_one() {
        let mut live = LiveLoudness::new(1);
        let samples = (4.0 * SAMPLE_RATE as f32) as usize;
        for n in 0..samples {
            let t = n as f32 / SAMPLE_RATE as f32;
            live.push(&[0.1 * (std::f32::consts::TAU * 1000.0 * t).sin()]);
        }
        let short = live.short_term().expect("a reading");
        let momentary = live.momentary().expect("a reading");
        assert!((short + 23.2).abs() < 0.4, "short term read {short}");
        assert!((momentary + 23.2).abs() < 0.4, "momentary read {momentary}");
    }

    /// The momentary reading follows the sound; the short-term one holds its
    /// nerve. Half a second of silence after four seconds of tone should move
    /// the first much further than the second.
    #[test]
    fn the_momentary_reading_moves_first() {
        let mut live = LiveLoudness::new(1);
        for n in 0..(4.0 * SAMPLE_RATE as f32) as usize {
            let t = n as f32 / SAMPLE_RATE as f32;
            live.push(&[0.5 * (std::f32::consts::TAU * 1000.0 * t).sin()]);
        }
        let (was_short, was_momentary) = (
            live.short_term().expect("a reading"),
            live.momentary().expect("a reading"),
        );
        for _ in 0..(0.5 * SAMPLE_RATE as f32) as usize {
            live.push(&[0.0]);
        }
        let short = live.short_term().expect("a reading");
        assert_eq!(
            live.momentary(),
            None,
            "half a second of silence has no level (it read {was_momentary} before)"
        );
        assert!(
            (short - was_short).abs() < 2.0,
            "the short-term reading jumped: {was_short} then {short}"
        );
    }

    #[test]
    fn a_meter_with_nothing_in_it_reads_nothing() {
        let mut live = LiveLoudness::new(2);
        assert_eq!(live.short_term(), None, "no sound is no reading");
        // Half a second of silence is still silence, not a level.
        live.push_interleaved(&vec![0.0; SAMPLE_RATE as usize], 2);
        assert_eq!(live.momentary(), None);

        // And a reset puts it back to knowing nothing.
        for n in 0..SAMPLE_RATE as usize {
            let t = n as f32 / SAMPLE_RATE as f32;
            live.push(&[0.5 * (std::f32::consts::TAU * 1000.0 * t).sin(), 0.0]);
        }
        assert!(live.momentary().is_some());
        live.reset();
        assert_eq!(live.momentary(), None);
    }

    #[test]
    fn the_gain_moves_a_mix_onto_its_target() {
        // Six dB up is a doubling, near enough.
        let gain = gain_for(-20.0, -14.0);
        assert!((gain - 2.0).abs() < 0.01, "{gain}");
        // And a mix already on target is left alone.
        assert!((gain_for(-14.0, -14.0) - 1.0).abs() < 1e-6);
        // Nothing is multiplied by more than ten.
        assert!(gain_for(-90.0, -14.0) <= 10.0);
    }
}
