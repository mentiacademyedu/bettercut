//! Finding the silences in a clip (§78, "Auto Silence Removal").
//!
//! From the waveform the timeline already keeps, like [`crate::beat_markers`]:
//! a stretch whose loudness stays under a threshold for long enough is a
//! pause, and the pauses are what a talking-head edit takes out.
//!
//! What comes back is a *suggestion* — timeline ranges for the interface to
//! show before anything is cut (§78: "Show suggested cuts … User confirms").

use bettercut_cache::Waveform;
use bettercut_foundation::{MediaTime, TICKS_PER_SECOND, TimelineTime};
use bettercut_timeline::{AudioClip, TimelineRange, timeline_ticks_for};

/// How a silence is recognised.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SilenceSettings {
    /// Quieter than this is silence, in dB below full scale. The waveform is
    /// stored in 8 bits, so nothing below about -42 dB can be told apart.
    pub threshold_db: f32,
    /// A pause shorter than this is part of speech, not a silence to remove.
    pub shortest: TimelineTime,
    /// Kept either side of each cut, so words keep their breath and are not
    /// clipped at the first consonant.
    pub padding: TimelineTime,
}

impl Default for SilenceSettings {
    fn default() -> Self {
        Self {
            threshold_db: -34.0,
            shortest: TimelineTime::from_millis(600),
            padding: TimelineTime::from_millis(120),
        }
    }
}

/// The stretches of `clip` to take out, in timeline order.
///
/// Silence at the very start or end of the clip is taken out up to the edge,
/// with padding only on the side next to sound — nobody wants half a second of
/// padding before the first word.
pub fn silent_ranges(
    clip: &AudioClip,
    waveform: &Waveform,
    settings: SilenceSettings,
) -> Vec<TimelineRange> {
    let rate = i64::from(waveform.peaks_per_second.max(1));
    let bucket = |t: MediaTime| (t.ticks().max(0) * rate / TICKS_PER_SECOND) as usize;
    let from = bucket(clip.source.start).min(waveform.peaks.len());
    let to = bucket(clip.source.end).min(waveform.peaks.len());
    if to <= from {
        return Vec::new();
    }

    let threshold = 10f32.powf(settings.threshold_db / 20.0);
    let quiet: Vec<bool> = waveform.peaks[from..to]
        .iter()
        .map(|p| p.magnitude() < threshold)
        .collect();

    // A bucket index within the clip's slice, to a timeline instant — through
    // the speed, like every other source-to-timeline mapping.
    let at = |index: usize| {
        let into_source = MediaTime::from_ticks(index as i64 * TICKS_PER_SECOND / rate);
        (clip.timeline.start
            + TimelineTime::from_ticks(timeline_ticks_for(into_source, clip.speed)))
        .min(clip.timeline.end)
    };

    let mut ranges = Vec::new();
    let mut index = 0;
    while index < quiet.len() {
        if !quiet[index] {
            index += 1;
            continue;
        }
        let start = index;
        while index < quiet.len() && quiet[index] {
            index += 1;
        }
        let (begin, end) = (at(start), at(index));
        let at_start = start == 0;
        let at_end = index == quiet.len();
        let begin = if at_start {
            clip.timeline.start
        } else {
            begin + settings.padding
        };
        let end = if at_end {
            clip.timeline.end
        } else {
            end - settings.padding
        };
        if end - begin >= settings.shortest
            && let Ok(range) = TimelineRange::new(begin, end)
        {
            ranges.push(range);
        }
    }
    // Found reading the material forwards; a reversed clip plays it the other
    // way, so each pause sits mirrored across the clip, and in reverse order.
    if clip.reversed {
        ranges = ranges
            .into_iter()
            .rev()
            .map(|range| bettercut_timeline::mirror_range_in(clip.timeline, range))
            .collect();
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_cache::Peak;
    use bettercut_foundation::{MediaId, Rational};
    use bettercut_timeline::SourceRange;

    /// Speech and pauses, a bucket per 5 ms: `(seconds, loud)` in order.
    fn waveform(pattern: &[(f64, bool)]) -> Waveform {
        let peaks = pattern
            .iter()
            .flat_map(|&(seconds, loud)| {
                let level: i8 = if loud { 90 } else { 1 };
                std::iter::repeat_n(
                    Peak {
                        min: -level,
                        max: level,
                    },
                    (seconds * 200.0) as usize,
                )
            })
            .collect();
        Waveform::new(200, peaks).unwrap()
    }

    fn clip(seconds: i64) -> AudioClip {
        AudioClip::new(
            MediaId::new(),
            TimelineTime::from_seconds(10),
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(seconds)).unwrap(),
        )
        .unwrap()
    }

    fn ms(v: i64) -> TimelineTime {
        TimelineTime::from_millis(v)
    }

    #[test]
    fn a_pause_is_found_with_padding_either_side() {
        // Talk 2 s, pause 1 s, talk 2 s.
        let wave = waveform(&[(2.0, true), (1.0, false), (2.0, true)]);
        let ranges = silent_ranges(&clip(5), &wave, SilenceSettings::default());
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, TimelineTime::from_seconds(12) + ms(120));
        assert_eq!(ranges[0].end, TimelineTime::from_seconds(13) - ms(120));
    }

    #[test]
    fn a_short_breath_is_left_alone() {
        let wave = waveform(&[(2.0, true), (0.4, false), (2.0, true)]);
        assert!(silent_ranges(&clip(4), &wave, SilenceSettings::default()).is_empty());
    }

    /// Dead air before the first word and after the last goes right up to the
    /// clip's edges.
    #[test]
    fn silence_at_the_edges_is_cut_to_the_edge() {
        let wave = waveform(&[(1.5, false), (2.0, true), (1.5, false)]);
        let ranges = silent_ranges(&clip(5), &wave, SilenceSettings::default());
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].start, TimelineTime::from_seconds(10));
        assert_eq!(ranges[0].end, ms(11_500) - ms(120));
        assert_eq!(ranges[1].start, ms(13_500) + ms(120));
        assert_eq!(ranges[1].end, TimelineTime::from_seconds(15));
    }

    #[test]
    fn a_louder_threshold_hears_more_as_silence() {
        // A quiet murmur at level 8 (about -24 dB) between two loud passages.
        let mut wave = waveform(&[(2.0, true), (1.0, false), (2.0, true)]);
        for peak in &mut wave.peaks[400..600] {
            *peak = Peak { min: -8, max: 8 };
        }
        let strict = SilenceSettings::default();
        assert!(
            silent_ranges(&clip(5), &wave, strict).is_empty(),
            "a murmur is not silence"
        );
        let loose = SilenceSettings {
            threshold_db: -20.0,
            ..strict
        };
        assert_eq!(silent_ranges(&clip(5), &wave, loose).len(), 1);
    }

    #[test]
    fn a_fast_clips_pauses_are_shorter_on_the_timeline() {
        let wave = waveform(&[(2.0, true), (2.0, false), (2.0, true)]);
        let mut fast = clip(6);
        fast.speed = Rational::new(2, 1).unwrap();
        fast.timeline.end = TimelineTime::from_seconds(13);
        let ranges = silent_ranges(&fast, &wave, SilenceSettings::default());
        assert_eq!(ranges.len(), 1);
        // Source 2–4 s is timeline 11–12 s at double speed.
        assert_eq!(ranges[0].start, TimelineTime::from_seconds(11) + ms(120));
        assert_eq!(ranges[0].end, TimelineTime::from_seconds(12) - ms(120));
    }
}
