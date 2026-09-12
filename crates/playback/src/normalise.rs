//! Bringing a quiet clip up to a usable level (§20a.4's clip-gain stage).
//!
//! A clip recorded too quietly is the most ordinary problem in an edit, and the
//! fix is a number: how much louder it has to be before its loudest moment
//! reaches full scale. That number is already in the waveform the timeline
//! keeps, so this is arithmetic rather than analysis.
//!
//! # Peaks, and what that does not mean
//!
//! This normalises the **peak**, not the loudness. Two clips normalised to the
//! same peak can still sound very different — a compressed voice is far louder
//! than a sparse piano at the same peak — because perceived loudness is closer
//! to an average than to a maximum, and measuring it properly (§28-style, at
//! -14 LUFS for a platform) needs the samples themselves and a K-weighting
//! filter, not an 8-bit peak envelope.
//!
//! Peak normalisation is still the right quick fix: it is exactly what "this
//! clip is too quiet" usually wants, and it can never clip, which is more than
//! can be said for guessing at a loudness match from data that cannot support
//! one.

use bettercut_cache::Waveform;
use bettercut_foundation::{MediaTime, TICKS_PER_SECOND};
use bettercut_timeline::AudioClip;

/// Where a normalised clip's loudest moment lands, in dB below full scale.
///
/// Not zero: a peak sitting exactly at full scale clips as soon as anything is
/// mixed with it, and a lossy encoder can overshoot its own input by a fraction
/// of a dB. One dB of headroom costs nothing audible.
pub const TARGET_DB: f32 = -1.0;

/// The loudest sample in the part of its file that `clip` plays, 0.0 to 1.0.
///
/// `None` when the clip's span falls outside the waveform, which happens while
/// the analysis is still being built.
pub fn peak_of(clip: &AudioClip, waveform: &Waveform) -> Option<f32> {
    let rate = i64::from(waveform.peaks_per_second.max(1));
    let bucket = |t: MediaTime| (t.ticks().max(0) * rate / TICKS_PER_SECOND) as usize;
    let from = bucket(clip.source.start).min(waveform.peaks.len());
    let to = bucket(clip.source.end).min(waveform.peaks.len());
    if to <= from {
        return None;
    }
    // The clip's own range only: normalising a line of dialogue by the loudest
    // moment of the whole recording would leave it as quiet as it started.
    Some(
        waveform.peaks[from..to]
            .iter()
            .map(|peak| peak.magnitude())
            .fold(0.0_f32, f32::max),
    )
}

/// The gain that puts `peak` at `target_db`, or `None` when there is nothing to
/// work with.
///
/// Refuses silence rather than returning an enormous number: a clip whose
/// loudest moment is below the waveform's own floor has no signal to raise,
/// and multiplying its noise by a thousand is not what anyone meant.
pub fn gain_for(peak: f32, target_db: f32) -> Option<f32> {
    // The waveform stores peaks in 8 bits, so nothing below about -42 dB can
    // be told from silence. Anything down there is silence for this purpose.
    if !peak.is_finite() || peak <= 0.01 {
        return None;
    }
    let target = 10f32.powf(target_db / 20.0);
    let gain = target / peak;
    Some(gain.clamp(0.0, bettercut_timeline::MAX_TRACK_GAIN))
}

/// How much louder normalising would make `clip`, as a multiplier.
///
/// `None` when the clip is silent, still being analysed, or already within a
/// whisker of the target — the last of which is worth distinguishing, because
/// "it is already right" and "it cannot be done" are different answers.
pub fn normalise(clip: &AudioClip, waveform: &Waveform) -> Option<f32> {
    let gain = gain_for(peak_of(clip, waveform)?, TARGET_DB)?;
    // Within a quarter of a dB is already there. Writing a gain of 1.003 into
    // the project would be an undo step the user cannot hear.
    (!(0.97..=1.03).contains(&gain)).then_some(gain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_cache::Peak;
    use bettercut_foundation::{MediaId, TimelineTime};
    use bettercut_timeline::SourceRange;

    /// A waveform whose level is `level` throughout, except for one louder
    /// moment `at` seconds in.
    fn waveform(level: i8, at: Option<(f64, i8)>) -> Waveform {
        let mut peaks = vec![
            Peak {
                min: -level,
                max: level
            };
            2_000
        ];
        if let Some((seconds, loud)) = at {
            let index = (seconds * 200.0) as usize;
            peaks[index] = Peak {
                min: -loud,
                max: loud,
            };
        }
        Waveform::new(200, peaks).unwrap()
    }

    /// A clip playing `[from, to)` seconds of its file.
    fn clip(from: i64, to: i64) -> AudioClip {
        AudioClip::new(
            MediaId::new(),
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::from_seconds(from), MediaTime::from_seconds(to)).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn a_quiet_clip_is_brought_up() {
        // A peak of about 0.3 of full scale: roughly -10 dB.
        let wave = waveform(38, None);
        let gain = normalise(&clip(0, 10), &wave).expect("something to raise");

        // The target, -1 dB, is 0.891 of full scale.
        assert!(
            (gain - 0.891 / 0.30).abs() < 0.15,
            "expected about 3x, got {gain:.2}"
        );
    }

    /// Normalising sets the peak *to* the target, which means bringing a hot
    /// clip down as readily as bringing a quiet one up: a clip peaking at full
    /// scale has no headroom, and the moment anything is mixed with it, it
    /// clips.
    #[test]
    fn a_clip_with_no_headroom_is_brought_down() {
        let wave = waveform(38, Some((5.0, 126)));
        let gain = normalise(&clip(0, 10), &wave).expect("something to change");
        assert!(
            gain < 1.0,
            "a clip peaking at full scale was not brought down: {gain:.2}"
        );
    }

    /// The peak decides it, and it is the peak of *what the clip plays*: the
    /// same recording trimmed away from its loudest moment is quiet again.
    #[test]
    fn only_the_part_the_clip_plays_is_measured() {
        let wave = waveform(38, Some((5.0, 126)));
        let gain = normalise(&clip(6, 10), &wave).expect("the loud moment is outside this clip");
        assert!(
            gain > 2.0,
            "the trimmed clip was measured by a moment it does not play: {gain:.2}"
        );
    }

    /// A clip far below the target is raised as far as the mixer's ceiling
    /// allows, rather than refusing because it cannot get all the way.
    #[test]
    fn a_very_quiet_clip_is_raised_as_far_as_it_can_go() {
        let wave = waveform(2, None); // about -36 dB
        let gain = normalise(&clip(0, 10), &wave).expect("something to raise");
        assert_eq!(gain, bettercut_timeline::MAX_TRACK_GAIN);
    }

    /// Silence has nothing to raise. Multiplying a noise floor by a thousand is
    /// not what "normalise" means.
    #[test]
    fn silence_is_refused() {
        assert!(normalise(&clip(0, 10), &waveform(1, None)).is_none());
        assert_eq!(gain_for(0.0, TARGET_DB), None);
        assert_eq!(gain_for(f32::NAN, TARGET_DB), None);
    }

    /// A clip already at the target is left alone: an undo step the user cannot
    /// hear is worse than no change at all.
    #[test]
    fn a_clip_already_at_the_target_is_left_alone() {
        // 113/127 is about -1 dB.
        assert!(normalise(&clip(0, 10), &waveform(113, None)).is_none());
    }

    /// The gain a clip can be given is bounded, like every other gain in the
    /// mixer: a barely-audible clip cannot ask for a hundredfold.
    #[test]
    fn the_gain_is_bounded() {
        let gain = gain_for(0.011, TARGET_DB).expect("just above the floor");
        assert_eq!(gain, bettercut_timeline::MAX_TRACK_GAIN);
    }

    /// A clip whose waveform has not been built yet says so, rather than
    /// claiming silence.
    #[test]
    fn a_clip_outside_the_waveform_has_no_answer() {
        let wave = waveform(50, None); // ten seconds of peaks
        assert_eq!(peak_of(&clip(30, 40), &wave), None);
    }
}
