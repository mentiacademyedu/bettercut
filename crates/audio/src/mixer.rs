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

/// A clip's keyframed volume across one block of output (§24).
///
/// Two values and a length rather than the keyframes themselves: the mixer
/// runs on the audio thread and must not walk a data structure or allocate
/// there (§54). The engine, which already knows the block, evaluates the
/// envelope at each end and hands over the line between them.
///
/// A block is 10 ms at the preview's size and one frame at the export's, and
/// a straight line across either is far finer than the ear resolves — while
/// holding one value for a whole block and stepping at the boundary is exactly
/// the zipper noise that makes automation sound cheap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainRamp {
    pub from: f32,
    pub to: f32,
    /// Output frames the ramp is spread over. Zero holds `from`.
    pub frames: i64,
}

impl GainRamp {
    /// A volume that does not move.
    pub const fn steady(gain: f32) -> Self {
        Self {
            from: gain,
            to: gain,
            frames: 0,
        }
    }

    /// The volume at `frame` frames into the block.
    pub fn at(&self, frame: usize) -> f32 {
        if self.frames <= 0 {
            return self.from;
        }
        let t = (frame as f32 / self.frames as f32).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * t
    }
}

/// Gain and pan for one clip on one track.
#[derive(Debug, Clone, Copy)]
pub struct MixParams {
    /// Linear, not decibels. Keyframeable later (§24).
    pub clip_gain: f32,
    pub track_gain: f32,
    /// -1.0 hard left, 0.0 centre, +1.0 hard right.
    pub track_pan: f32,
    /// The clip's fade in and out, part of the clip-gain stage.
    pub fades: Fades,
    /// The clip's keyframed volume across this block, where it has one.
    ///
    /// When it is here it **is** the clip's volume: §24's rule everywhere else
    /// in the editor is that a keyframed value replaces the static one rather
    /// than scaling it, so the caller passes `clip_gain: 1.0` beside it. The
    /// fades still apply on top — an envelope says how the level rides, a fade
    /// says how the clip enters and leaves, and a clip can want both.
    pub automation: Option<GainRamp>,
}

impl Default for MixParams {
    fn default() -> Self {
        Self {
            clip_gain: 1.0,
            track_gain: 1.0,
            track_pan: 0.0,
            fades: Fades::default(),
            automation: None,
        }
    }
}

/// A clip's fade in and out, as seen from one block of output.
///
/// All counts are in output frames, and all of them are measured from the
/// first frame this block mixes: the envelope is a function of *where in the
/// clip* each frame falls, which is the only way a fade can come out the same
/// however the timeline is cut into blocks — and so the same in the preview's
/// 480-frame blocks as in the export's frame-sized ones (§46).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fades {
    /// Frames of the clip already heard before the first mixed frame.
    pub into_clip: i64,
    /// Frames of the clip left from the first mixed frame to its end.
    pub remaining: i64,
    /// Length of each fade in frames. Zero is no fade.
    pub fade_in: i64,
    pub fade_out: i64,
}

impl Fades {
    /// The envelope at `frame` frames into the block: 1.0 outside both fades.
    ///
    /// Quadratic, not linear. A linear ramp of *amplitude* sounds like it holds
    /// loud and then drops away at the last moment, because loudness is heard
    /// roughly logarithmically; squaring spreads the drop across the fade.
    pub fn gain_at(&self, frame: usize) -> f32 {
        let frame = frame as i64;
        let mut gain = 1.0;
        if self.fade_in > 0 {
            let into = self.into_clip + frame;
            if into < self.fade_in {
                let t = (into.max(0) as f32) / (self.fade_in as f32);
                gain *= t * t;
            }
        }
        if self.fade_out > 0 {
            let left = self.remaining - frame;
            if left < self.fade_out {
                let t = (left.max(0) as f32) / (self.fade_out as f32);
                gain *= t * t;
            }
        }
        gain
    }

    fn is_none(&self) -> bool {
        self.fade_in <= 0 && self.fade_out <= 0
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
    let fading = !params.fades.is_none();
    // Applied whenever it is there, moving or not: an envelope held at half
    // volume is still half volume, and skipping a ramp because it does not
    // *move* would silently drop the level it was holding.
    let riding = params.automation;

    for frame in 0..count {
        let out_base = (at_frame + frame) * channels;
        let mut envelope = if fading {
            params.fades.gain_at(frame)
        } else {
            1.0
        };
        if let Some(ramp) = riding {
            envelope *= ramp.at(frame);
        }

        if channels == 2 {
            // The common case: stereo out. A mono source feeds both sides.
            let left = planes[0][frame];
            let right = planes.get(1).map_or(left, |p| p[frame]);
            out[out_base] += left * left_gain * envelope;
            out[out_base + 1] += right * right_gain * envelope;
        } else {
            // Any other layout: apply the combined gain without panning, since
            // pan has no defined meaning outside stereo.
            let gain = params.clip_gain * params.track_gain * envelope;
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

/// The loudest sample in each of a stereo block, for a level meter (§20a).
///
/// Peak rather than an average, because the number a meter exists to answer is
/// "is this about to clip?", and an average says no right up until it does.
/// Read after [`finish`], so the meter shows what the device is being handed
/// rather than what the mix was before the master gain and the limiter.
///
/// A mono or multi-channel block folds into the pair: channel 0 on the left,
/// channel 1 (or 0 again) on the right, and anything beyond ignored — a meter
/// with eight bars would say nothing a stereo one does not.
pub fn peaks(out: &[f32], channels: usize) -> (f32, f32) {
    if channels == 0 || out.is_empty() {
        return (0.0, 0.0);
    }
    let mut left = 0.0_f32;
    let mut right = 0.0_f32;
    for frame in out.chunks_exact(channels) {
        left = left.max(frame[0].abs());
        right = right.max(frame.get(1).copied().unwrap_or(frame[0]).abs());
    }
    // NaN is possible from a broken decode and would poison the meter for the
    // rest of the session, since a meter holds its peak.
    let sane = |value: f32| if value.is_finite() { value } else { 0.0 };
    (sane(left), sane(right))
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
    fn a_fade_in_rises_from_silence() {
        let fades = Fades {
            into_clip: 0,
            remaining: 1000,
            fade_in: 100,
            fade_out: 0,
        };
        assert_eq!(fades.gain_at(0), 0.0, "a fade in starts silent");
        assert!(
            (fades.gain_at(50) - 0.25).abs() < 1e-6,
            "quadratic at halfway"
        );
        assert_eq!(fades.gain_at(100), 1.0);
        assert_eq!(fades.gain_at(500), 1.0);
    }

    #[test]
    fn a_fade_out_reaches_silence_at_the_clips_end() {
        let fades = Fades {
            into_clip: 900,
            remaining: 100,
            fade_in: 0,
            fade_out: 100,
        };
        assert_eq!(fades.gain_at(0), 1.0);
        assert!((fades.gain_at(50) - 0.25).abs() < 1e-6);
        assert_eq!(fades.gain_at(100), 0.0, "silent by the last frame");
    }

    /// The envelope depends on where in the clip a frame is, not on where the
    /// block boundaries fall — so any block size gives the same samples.
    #[test]
    fn a_fade_is_the_same_whatever_the_block_size() {
        let fade = |into_clip: i64| Fades {
            into_clip,
            remaining: 400 - into_clip,
            fade_in: 300,
            fade_out: 0,
        };
        let whole: Vec<f32> = (0..400).map(|f| fade(0).gain_at(f)).collect();
        let blocked: Vec<f32> = (0..4)
            .flat_map(|block| (0..100).map(move |f| fade(block * 100).gain_at(f)))
            .collect();
        assert_eq!(whole, blocked);
    }

    #[test]
    fn a_fade_is_applied_to_the_mix() {
        let mut out = vec![0.0; 8];
        mix_into(
            &mut out,
            2,
            &mono(&[1.0; 4]),
            0,
            MixParams {
                track_pan: -1.0,
                fades: Fades {
                    into_clip: 0,
                    remaining: 4,
                    fade_in: 2,
                    fade_out: 0,
                },
                ..Default::default()
            },
        );
        assert_eq!(out[0], 0.0);
        assert!((out[2] - 0.25).abs() < 1e-5);
        assert!((out[4] - 1.0).abs() < 1e-5);
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

#[cfg(test)]
mod automation_tests {
    use super::*;

    fn mono(samples: &[f32]) -> Vec<Vec<f32>> {
        vec![samples.to_vec()]
    }

    #[test]
    fn a_ramp_is_a_straight_line_across_the_block() {
        let ramp = GainRamp {
            from: 1.0,
            to: 0.0,
            frames: 4,
        };
        assert_eq!(ramp.at(0), 1.0);
        assert_eq!(ramp.at(2), 0.5);
        assert_eq!(ramp.at(4), 0.0);
        // Past the end holds the last value rather than running negative: a
        // block can mix more frames than the ramp was measured over.
        assert_eq!(ramp.at(9), 0.0);
    }

    #[test]
    fn a_steady_ramp_holds_its_value() {
        assert_eq!(GainRamp::steady(0.25).at(0), 0.25);
        assert_eq!(GainRamp::steady(0.25).at(1_000), 0.25);
    }

    #[test]
    fn the_envelope_reaches_the_output() {
        let mut out = vec![0.0_f32; 8]; // 4 stereo frames
        mix_into(
            &mut out,
            2,
            &mono(&[1.0, 1.0, 1.0, 1.0]),
            0,
            MixParams {
                automation: Some(GainRamp {
                    from: 1.0,
                    to: 0.0,
                    frames: 4,
                }),
                ..MixParams::default()
            },
        );
        // Constant-power pan puts a centred mono source at 1/sqrt(2) a side.
        let centre = std::f32::consts::FRAC_1_SQRT_2;
        for (frame, wanted) in [1.0, 0.75, 0.5, 0.25].into_iter().enumerate() {
            let actual = out[frame * 2];
            assert!(
                (actual - centre * wanted).abs() < 1e-5,
                "frame {frame} came out at {actual}, wanted {}",
                centre * wanted
            );
        }
    }

    /// An envelope and a fade are different things and a clip can want both:
    /// music ducked under a voice, fading out at the end of the film.
    #[test]
    fn an_envelope_and_a_fade_both_apply() {
        let mut out = vec![0.0_f32; 8];
        mix_into(
            &mut out,
            2,
            &mono(&[1.0, 1.0, 1.0, 1.0]),
            0,
            MixParams {
                fades: Fades {
                    into_clip: 0,
                    remaining: 4,
                    fade_in: 4,
                    fade_out: 0,
                },
                automation: Some(GainRamp::steady(0.5)),
                ..MixParams::default()
            },
        );
        let centre = std::f32::consts::FRAC_1_SQRT_2;
        // Frame 2 is halfway through a quadratic fade in — 0.25 — and the
        // envelope halves it again.
        let actual = out[2 * 2];
        assert!(
            (actual - centre * 0.25 * 0.5).abs() < 1e-5,
            "the fade and the envelope did not both apply: {actual}"
        );
    }

    /// Automation is the clip's volume, not a second multiplier on top of the
    /// static one — §24's rule everywhere else in the editor.
    #[test]
    fn the_caller_is_expected_to_pass_a_gain_of_one() {
        let mut out = vec![0.0_f32; 4];
        mix_into(
            &mut out,
            2,
            &mono(&[1.0, 1.0]),
            0,
            MixParams {
                clip_gain: 1.0,
                automation: Some(GainRamp::steady(0.5)),
                ..MixParams::default()
            },
        );
        let centre = std::f32::consts::FRAC_1_SQRT_2;
        assert!((out[0] - centre * 0.5).abs() < 1e-5);
    }
}

#[cfg(test)]
mod peak_tests {
    use super::*;

    #[test]
    fn the_loudest_sample_of_each_side_is_the_peak() {
        // Interleaved stereo: left rises to 0.8, right stays at 0.2.
        let block = [0.1, 0.2, -0.8, 0.1, 0.3, -0.2];
        assert_eq!(peaks(&block, 2), (0.8, 0.2));
    }

    /// Peak, not average: the question a meter answers is "is this about to
    /// clip?", and an average says no right up until it does.
    #[test]
    fn one_loud_sample_in_a_quiet_block_still_shows() {
        let mut block = vec![0.01_f32; 960];
        block[517] = 0.99;
        let (left, right) = peaks(&block, 2);
        assert!(
            left.max(right) > 0.9,
            "a single loud sample was averaged away: {left}, {right}"
        );
    }

    #[test]
    fn a_mono_block_feeds_both_sides() {
        assert_eq!(peaks(&[0.5, -0.25, 0.4], 1), (0.5, 0.5));
    }

    #[test]
    fn silence_reads_as_silence() {
        assert_eq!(peaks(&[0.0; 64], 2), (0.0, 0.0));
        assert_eq!(peaks(&[], 2), (0.0, 0.0));
        assert_eq!(peaks(&[1.0, 1.0], 0), (0.0, 0.0));
    }

    /// A broken decode can produce NaN, and a meter holds its peak — one bad
    /// sample would otherwise pin the meter for the rest of the session.
    #[test]
    fn a_broken_sample_does_not_poison_the_meter() {
        let (left, right) = peaks(&[f32::NAN, f32::INFINITY, 0.3, 0.2], 2);
        assert!(left.is_finite() && right.is_finite(), "{left}, {right}");
    }
}
