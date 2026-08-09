//! The mix graph (§20a.4).
//!
//! Summing order is specified, not incidental, because §46 requires preview and
//! export to match sample for sample:
//!
//! ```text
//! Clip sample
//! -> clip gain      (keyframeable)
//! -> clip effects
//! -> track gain -> track pan
//! -> track sum
//! -> master gain
//! -> limiter        (prevent clipping)
//! -> output
//! ```
//!
//! Everything sums in `f32`; the only clamp is at the very end. Clamping
//! earlier would make the result depend on the order tracks happen to be
//! processed in, which is exactly the kind of difference that shows up as
//! "the export sounds different from the preview".
//!
//! Clip effects are not implemented yet (Milestone 8); the stage exists in the
//! order so that adding it later does not move anything else.

/// Gain and pan for one clip on one track.
#[derive(Debug, Clone, Copy)]
pub struct MixParams {
    /// Linear, not decibels. Keyframeable later (§24).
    pub clip_gain: f32,
    pub track_gain: f32,
    /// -1.0 hard left, 0.0 centre, +1.0 hard right.
    pub track_pan: f32,
}

impl Default for MixParams {
    fn default() -> Self {
        Self {
            clip_gain: 1.0,
            track_gain: 1.0,
            track_pan: 0.0,
        }
    }
}

impl MixParams {
    /// Per-channel multipliers for a stereo bus.
    ///
    /// Constant-power panning: a centred source is `1/sqrt(2)` in both
    /// channels rather than 1.0, so panning across the image keeps a steady
    /// perceived loudness. Linear panning instead makes the centre sound
    /// noticeably quieter than the edges.
    fn stereo_gains(self) -> (f32, f32) {
        let pan = self.track_pan.clamp(-1.0, 1.0);
        let angle = (pan + 1.0) * (std::f32::consts::FRAC_PI_4); // 0..pi/2
        let base = self.clip_gain * self.track_gain;
        (base * angle.cos(), base * angle.sin())
    }
}

/// Sum one clip's planar samples into an interleaved output buffer.
///
/// * `out` is interleaved, `channels` per frame.
/// * `planes` is planar: one `Vec` per source channel (§20a.3).
/// * `at_frame` is where in `out` this clip's audio starts.
///
/// Adds rather than overwrites, because several clips and tracks land in the
/// same buffer. The caller zeroes `out` once per block.
pub fn mix_into(
    out: &mut [f32],
    channels: usize,
    planes: &[Vec<f32>],
    at_frame: usize,
    params: MixParams,
) {
    if channels == 0 || planes.is_empty() {
        return;
    }

    let out_frames = out.len() / channels;
    if at_frame >= out_frames {
        return;
    }

    let source_frames = planes.iter().map(Vec::len).min().unwrap_or(0);
    let count = source_frames.min(out_frames - at_frame);
    let (left_gain, right_gain) = params.stereo_gains();

    for frame in 0..count {
        let out_base = (at_frame + frame) * channels;

        if channels == 2 {
            // The common case: stereo out. A mono source feeds both sides.
            let left = planes[0][frame];
            let right = planes.get(1).map_or(left, |p| p[frame]);
            out[out_base] += left * left_gain;
            out[out_base + 1] += right * right_gain;
        } else {
            // Any other layout: apply the combined gain without panning, since
            // pan has no defined meaning outside stereo.
            let gain = params.clip_gain * params.track_gain;
            for channel in 0..channels {
                let sample = planes
                    .get(channel)
                    .or_else(|| planes.first())
                    .map_or(0.0, |p| p[frame]);
                out[out_base + channel] += sample * gain;
            }
        }
    }
}

/// Apply master gain, then limit. The final stage (§20a.4).
///
/// Returns how many samples had to be limited. A non-zero count means the mix
/// is genuinely clipping and the user should see it, rather than only hearing
/// the distortion.
pub fn finish(out: &mut [f32], master_gain: f32) -> usize {
    let mut limited = 0;
    for sample in out.iter_mut() {
        let value = *sample * master_gain;
        // Clamp only here, once, as §20a.4 requires.
        let clamped = value.clamp(-1.0, 1.0);
        if clamped != value {
            limited += 1;
        }
        // NaN would propagate into the device buffer and can produce a loud
        // pop on some drivers; a single bad sample must not do that.
        *sample = if clamped.is_finite() { clamped } else { 0.0 };
    }
    limited
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono(samples: &[f32]) -> Vec<Vec<f32>> {
        vec![samples.to_vec()]
    }

    fn stereo(left: &[f32], right: &[f32]) -> Vec<Vec<f32>> {
        vec![left.to_vec(), right.to_vec()]
    }

    #[test]
    fn a_centred_mono_clip_reaches_both_channels_equally() {
        let mut out = vec![0.0; 8];
        mix_into(
            &mut out,
            2,
            &mono(&[1.0, 1.0, 1.0, 1.0]),
            0,
            MixParams::default(),
        );

        for frame in out.chunks_exact(2) {
            assert!((frame[0] - frame[1]).abs() < 1e-6, "channels differ");
            // Constant-power centre is 1/sqrt(2).
            assert!((frame[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        }
    }

    /// Constant power: the total energy is the same wherever the pan sits.
    /// Linear panning would fail this, and centred material would sound quiet.
    #[test]
    fn panning_preserves_power() {
        for pan in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let mut out = vec![0.0; 2];
            mix_into(
                &mut out,
                2,
                &mono(&[1.0]),
                0,
                MixParams {
                    track_pan: pan,
                    ..Default::default()
                },
            );
            let power = out[0] * out[0] + out[1] * out[1];
            assert!(
                (power - 1.0).abs() < 1e-5,
                "pan {pan} gave power {power}, expected 1.0"
            );
        }
    }

    #[test]
    fn hard_left_and_right_silence_the_other_channel() {
        let mut left_only = vec![0.0; 2];
        mix_into(
            &mut left_only,
            2,
            &mono(&[1.0]),
            0,
            MixParams {
                track_pan: -1.0,
                ..Default::default()
            },
        );
        assert!((left_only[0] - 1.0).abs() < 1e-5);
        assert!(left_only[1].abs() < 1e-5);

        let mut right_only = vec![0.0; 2];
        mix_into(
            &mut right_only,
            2,
            &mono(&[1.0]),
            0,
            MixParams {
                track_pan: 1.0,
                ..Default::default()
            },
        );
        assert!(right_only[0].abs() < 1e-5);
        assert!((right_only[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn clips_are_summed_not_overwritten() {
        let mut out = vec![0.0; 4];
        let params = MixParams {
            track_pan: -1.0, // full left, so the arithmetic is easy to read
            ..Default::default()
        };
        mix_into(&mut out, 2, &mono(&[0.25, 0.25]), 0, params);
        mix_into(&mut out, 2, &mono(&[0.5, 0.5]), 0, params);

        assert!(
            (out[0] - 0.75).abs() < 1e-5,
            "second clip replaced the first"
        );
        assert!((out[2] - 0.75).abs() < 1e-5);
    }

    #[test]
    fn a_clip_can_start_partway_through_the_block() {
        let mut out = vec![0.0; 8];
        mix_into(
            &mut out,
            2,
            &mono(&[1.0, 1.0]),
            2,
            MixParams {
                track_pan: -1.0,
                ..Default::default()
            },
        );

        assert_eq!(out[0], 0.0, "wrote before the clip's start");
        assert_eq!(out[2], 0.0);
        assert!((out[4] - 1.0).abs() < 1e-5);
        assert!((out[6] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_clip_running_past_the_block_is_truncated_not_panicking() {
        let mut out = vec![0.0; 4];
        mix_into(&mut out, 2, &mono(&[1.0; 100]), 1, MixParams::default());
        // Only the one remaining frame was written.
        assert_eq!(out[0], 0.0);
        assert!(out[2] > 0.0);
    }

    #[test]
    fn a_clip_starting_past_the_block_writes_nothing() {
        let mut out = vec![0.0; 4];
        mix_into(&mut out, 2, &mono(&[1.0; 10]), 99, MixParams::default());
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn stereo_sources_keep_their_channels() {
        let mut out = vec![0.0; 4];
        mix_into(
            &mut out,
            2,
            &stereo(&[1.0, 1.0], &[-1.0, -1.0]),
            0,
            MixParams::default(),
        );
        assert!(out[0] > 0.0, "left channel lost");
        assert!(out[1] < 0.0, "right channel lost");
    }

    #[test]
    fn the_limiter_clamps_and_reports() {
        let mut out = vec![0.5, -0.5, 2.0, -2.0];
        let limited = finish(&mut out, 1.0);

        assert_eq!(limited, 2);
        assert_eq!(out, vec![0.5, -0.5, 1.0, -1.0]);
    }

    #[test]
    fn master_gain_is_applied_before_the_limiter() {
        let mut out = vec![0.5];
        let limited = finish(&mut out, 4.0);
        assert_eq!(limited, 1, "2.0 should have been limited");
        assert_eq!(out[0], 1.0);
    }

    /// A single NaN reaching the device can produce a loud pop.
    #[test]
    fn non_finite_samples_become_silence() {
        let mut out = vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        finish(&mut out, 1.0);
        assert!(
            out.iter().all(|s| s.is_finite()),
            "non-finite sample survived"
        );
        assert_eq!(out[0], 0.0, "NaN should be silenced, not clamped");
    }

    #[test]
    fn silence_stays_silent() {
        let mut out = vec![0.0; 16];
        assert_eq!(finish(&mut out, 1.0), 0);
        assert!(out.iter().all(|s| *s == 0.0));
    }
}
