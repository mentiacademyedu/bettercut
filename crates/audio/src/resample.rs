//! Re-timing audio for §51's speed control.
//!
//! Playing a clip at 2× means consuming twice as many source samples in the
//! same stretch of time, which is a resampling problem: the output frame at
//! index *n* falls between two input frames, and its value has to come from
//! both.
//!
//! ## Linear interpolation, and what it costs
//!
//! Straight-line between the two neighbouring input frames. The honest
//! alternative is a windowed-sinc filter, which is what a mastering tool would
//! use — and the difference shows as aliasing in the top octave when speeding
//! up, because linear interpolation is a poor anti-alias filter.
//!
//! It is the right trade here for two reasons. The mix is 48 kHz (§20a.3), so
//! the artefacts sit at the edge of hearing where speech and music have almost
//! nothing; and this runs inside the block the mixer is about to hand to the
//! device, where a long filter kernel would cost far more than the difference
//! is worth. A better filter can replace this function without anything else
//! changing.
//!
//! ## Pitch
//!
//! Deliberately not corrected. Sped-up sound is higher, exactly as it is on
//! tape, which is what every short-form editor does by default and what the
//! effect is *for*. Preserving pitch is a different feature — it needs a phase
//! vocoder, not a resampler — and pretending this does it would be worse than
//! not offering it.

/// Read `out_frames` of output from `input`, advancing through it at `rate`.
///
/// `input` is planar: one `Vec` per channel, all the same length (§20a.3).
/// `rate` is the playback speed — 2.0 consumes two input frames per output
/// frame. The result always has exactly `out_frames` per channel, padded with
/// silence when the input runs out, because the mixer writes into a
/// fixed-size block and a short read would leave the tail of it holding
/// whatever was there before.
pub fn resample(input: &[Vec<f32>], out_frames: usize, rate: f64) -> Vec<Vec<f32>> {
    if input.is_empty() {
        return Vec::new();
    }
    // A non-positive rate is not slow motion, it is a stopped or reversed
    // clip. Neither is what the control means; both would index backwards.
    let rate = if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        1.0
    };

    input
        .iter()
        .map(|plane| {
            let mut out = Vec::with_capacity(out_frames);
            for index in 0..out_frames {
                out.push(sample_at(plane, index as f64 * rate));
            }
            out
        })
        .collect()
}

/// One output sample, at a fractional position in the input.
fn sample_at(plane: &[f32], at: f64) -> f32 {
    if plane.is_empty() {
        return 0.0;
    }
    let floor = at.floor();
    // Past the end is silence rather than the last sample held: a held sample
    // is a click at the end of every clip.
    if floor < 0.0 || floor >= plane.len() as f64 {
        return 0.0;
    }

    let index = floor as usize;
    let fraction = (at - floor) as f32;
    let current = plane[index];
    let next = plane.get(index + 1).copied().unwrap_or(0.0);
    current + (next - current) * fraction
}

/// How many input frames `resample` will reach for.
///
/// The caller has to decode this many before asking, and one *past* the last
/// interpolation point — the final output frame reads its neighbour.
pub fn input_frames_needed(out_frames: usize, rate: f64) -> usize {
    if out_frames == 0 {
        return 0;
    }
    let rate = if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        1.0
    };
    ((out_frames as f64 - 1.0) * rate).floor() as usize + 2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ramp, so an interpolated sample has an obvious right answer.
    fn ramp(frames: usize) -> Vec<Vec<f32>> {
        vec![(0..frames).map(|n| n as f32).collect()]
    }

    #[test]
    fn normal_speed_returns_the_input_unchanged() {
        let input = ramp(8);
        let out = resample(&input, 8, 1.0);
        assert_eq!(out[0], input[0]);
    }

    /// Double speed takes every other sample: the whole point of the feature.
    #[test]
    fn double_speed_advances_twice_as_fast() {
        let out = resample(&ramp(8), 4, 2.0);
        assert_eq!(out[0], vec![0.0, 2.0, 4.0, 6.0]);
    }

    /// Half speed interpolates a sample between each pair, rather than
    /// repeating them — a repeat is a stair-step, which is audible.
    #[test]
    fn half_speed_interpolates_between_samples() {
        let out = resample(&ramp(4), 6, 0.5);
        assert_eq!(out[0], vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5]);
    }

    /// Every channel is resampled the same way. Two channels drifting apart
    /// would be a stereo image that smears as the clip plays.
    #[test]
    fn every_channel_advances_together() {
        let input = vec![
            (0..8).map(|n| n as f32).collect::<Vec<_>>(),
            (0..8).map(|n| -(n as f32)).collect::<Vec<_>>(),
        ];
        let out = resample(&input, 4, 2.0);
        assert_eq!(out[0], vec![0.0, 2.0, 4.0, 6.0]);
        assert_eq!(out[1], vec![0.0, -2.0, -4.0, -6.0]);
    }

    /// The mixer writes into a fixed-size block, so a short read would leave
    /// the tail of it holding the previous block's audio — which repeats.
    #[test]
    fn the_output_is_always_the_length_asked_for() {
        for (available, wanted, rate) in [(2, 16, 1.0), (0, 16, 1.0), (100, 16, 8.0)] {
            let out = resample(&ramp(available), wanted, rate);
            assert_eq!(out[0].len(), wanted, "{available} frames at {rate}×");
        }
    }

    /// Past the end is silence, not the last sample held. A held sample is a
    /// click at the end of every clip.
    #[test]
    fn running_out_of_input_fades_to_silence_not_a_click() {
        let out = resample(&ramp(4), 8, 1.0);
        assert_eq!(&out[0][4..], &[0.0, 0.0, 0.0, 0.0]);
    }

    /// An empty input is not a panic. It happens whenever a decoder reaches the
    /// end of a file mid-block.
    #[test]
    fn no_input_produces_silence() {
        let out = resample(&[vec![]], 8, 2.0);
        assert_eq!(out[0], vec![0.0; 8]);
    }

    #[test]
    fn a_stopped_or_reversed_rate_falls_back_to_normal() {
        let input = ramp(8);
        for rate in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(resample(&input, 8, rate)[0], input[0], "rate {rate}");
        }
    }

    /// The caller decodes before it resamples, so it has to know how far ahead
    /// to read — including the one extra frame the last interpolation needs.
    #[test]
    fn the_input_needed_covers_the_last_interpolation() {
        assert_eq!(input_frames_needed(0, 2.0), 0);
        assert_eq!(input_frames_needed(1, 2.0), 2);
        assert_eq!(input_frames_needed(4, 2.0), 8);
        assert_eq!(input_frames_needed(8, 0.5), 5);

        // And it really is enough: asking for exactly that many produces no
        // silence at the end.
        for (wanted, rate) in [(4, 2.0), (8, 0.5), (16, 1.5), (10, 3.0)] {
            let needed = input_frames_needed(wanted, rate);
            let out = resample(&ramp(needed), wanted, rate);
            assert!(
                out[0].iter().skip(1).any(|s| *s != 0.0),
                "{wanted} frames at {rate}× came back silent with {needed} in"
            );
            assert_ne!(
                out[0][wanted - 1],
                0.0,
                "{wanted} at {rate}×: the last frame fell past the input"
            );
        }
    }
}
