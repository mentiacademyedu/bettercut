//! Where the talking is, for timing captions (§28).
//!
//! §28's automatic captions want a transcription engine, and that is a model
//! and a download away. But transcription answers two questions — *when* and
//! *what* — and only one of them needs a model. The **when** is in the waveform
//! the timeline already keeps, and it is the tedious half: typing a sentence
//! takes a moment, finding the exact moment it starts takes a scrub, a nudge
//! and another scrub, thirty times over.
//!
//! So this gives back one range per phrase. The interface turns them into empty
//! captions and the user types the words. When a real transcription provider
//! arrives — §28's `TranscriptionProvider`, a local Whisper-compatible model or
//! a cloud one — it answers in the same shape with the words filled in, and
//! everything downstream of here is already built (§46).
//!
//! # What counts as a phrase
//!
//! Loud stretches, with three rules that make the result readable rather than
//! merely accurate:
//!
//! * a gap shorter than [`SpeechSettings::shortest_pause`] does not end a
//!   phrase — the pause between words is not the pause between sentences;
//! * a burst shorter than [`SpeechSettings::shortest_phrase`] is not a phrase —
//!   a cough, a click, a chair;
//! * a phrase longer than [`SpeechSettings::longest`] is split, because a
//!   caption nobody can read before it changes is no caption at all.

use bettercut_cache::Waveform;
use bettercut_foundation::{MediaTime, TICKS_PER_SECOND, TimelineTime};
use bettercut_timeline::{AudioClip, TimelineRange, timeline_ticks_for};

/// How speech is picked out of a waveform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechSettings {
    /// Louder than this is speech, in dB below full scale.
    pub threshold_db: f32,
    /// A gap shorter than this is inside a phrase, not between two.
    pub shortest_pause: TimelineTime,
    /// A burst shorter than this is a noise, not a phrase.
    pub shortest_phrase: TimelineTime,
    /// A phrase longer than this is broken up, so each caption can be read.
    pub longest: TimelineTime,
}

impl Default for SpeechSettings {
    fn default() -> Self {
        Self {
            // A little more forgiving than silence removal's -34 dB: cutting
            // into a quiet word is a mistake the user sees immediately, while
            // a caption that starts a moment early costs nothing.
            threshold_db: -38.0,
            shortest_pause: TimelineTime::from_millis(350),
            shortest_phrase: TimelineTime::from_millis(300),
            longest: TimelineTime::from_seconds(5),
        }
    }
}

/// The stretches of `clip` that have speech in them, in timeline order.
pub fn speech_ranges(
    clip: &AudioClip,
    waveform: &Waveform,
    settings: SpeechSettings,
) -> Vec<TimelineRange> {
    let rate = i64::from(waveform.peaks_per_second.max(1));
    let bucket = |t: MediaTime| (t.ticks().max(0) * rate / TICKS_PER_SECOND) as usize;
    let from = bucket(clip.source.start).min(waveform.peaks.len());
    let to = bucket(clip.source.end).min(waveform.peaks.len());
    if to <= from {
        return Vec::new();
    }

    let threshold = 10f32.powf(settings.threshold_db / 20.0);
    let loud: Vec<bool> = waveform.peaks[from..to]
        .iter()
        .map(|peak| peak.magnitude() >= threshold)
        .collect();

    // A bucket index within the clip's slice, to a timeline instant — through
    // the speed, like every other source-to-timeline mapping (§51).
    let at = |index: usize| {
        let into_source = MediaTime::from_ticks(index as i64 * TICKS_PER_SECOND / rate);
        (clip.timeline.start
            + TimelineTime::from_ticks(timeline_ticks_for(into_source, clip.speed)))
        .min(clip.timeline.end)
    };
    let buckets = |span: TimelineTime| {
        // Timeline durations are compared in source buckets, so a clip playing
        // at double speed needs twice as much source to make the same gap.
        let source = MediaTime::from_ticks(clip.speed.scale(span.ticks()));
        ((source.ticks() * rate) / TICKS_PER_SECOND).max(0) as usize
    };
    let pause = buckets(settings.shortest_pause);

    // Group the loud buckets, joining any two separated by a short gap.
    let mut phrases: Vec<(usize, usize)> = Vec::new();
    let mut index = 0;
    while index < loud.len() {
        if !loud[index] {
            index += 1;
            continue;
        }
        let start = index;
        let mut end = index;
        while index < loud.len() {
            if loud[index] {
                end = index + 1;
                index += 1;
                continue;
            }
            // A run of quiet: part of the phrase if it is short and something
            // is said after it, the end of the phrase otherwise.
            let quiet_from = index;
            while index < loud.len() && !loud[index] {
                index += 1;
            }
            if index - quiet_from > pause || index >= loud.len() {
                break;
            }
        }
        phrases.push((start, end));
    }

    let shortest = settings.shortest_phrase;
    let longest = settings.longest.max(TimelineTime::from_millis(500));
    let mut ranges = Vec::new();
    for (start, end) in phrases {
        let (begin, finish) = (at(start), at(end));
        if finish - begin < shortest {
            continue;
        }
        // Break a long phrase into equal pieces rather than at an arbitrary
        // five seconds: two captions of four seconds read better than one of
        // five and one of three.
        let span = (finish - begin).ticks();
        let limit = longest.ticks().max(1);
        let pieces = ((span + limit - 1) / limit).max(1);
        for piece in 0..pieces {
            let piece_start = TimelineTime::from_ticks(begin.ticks() + span * piece / pieces);
            let piece_end = TimelineTime::from_ticks(begin.ticks() + span * (piece + 1) / pieces);
            if let Ok(range) = TimelineRange::new(piece_start, piece_end) {
                ranges.push(range);
            }
        }
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

    /// A clip starting ten seconds along the timeline.
    fn clip(seconds: f64) -> AudioClip {
        AudioClip::new(
            MediaId::new(),
            TimelineTime::from_seconds(10),
            SourceRange::new(
                MediaTime::ZERO,
                MediaTime::from_millis((seconds * 1000.0) as i64),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn ms(v: i64) -> TimelineTime {
        TimelineTime::from_millis(v)
    }

    /// Timeline instants as milliseconds from the clip's start, for readable
    /// assertions.
    fn into_clip(ranges: &[TimelineRange]) -> Vec<(i64, i64)> {
        ranges
            .iter()
            .map(|range| {
                let base = TimelineTime::from_seconds(10);
                (
                    (range.start - base).ticks() / 960,
                    (range.end - base).ticks() / 960,
                )
            })
            .collect()
    }

    #[test]
    fn two_sentences_with_a_pause_between_them() {
        let wave = waveform(&[(2.0, true), (1.0, false), (2.0, true)]);
        let ranges = speech_ranges(&clip(5.0), &wave, SpeechSettings::default());
        assert_eq!(into_clip(&ranges), vec![(0, 2000), (3000, 5000)]);
    }

    /// The gap between words is not the gap between sentences.
    #[test]
    fn a_breath_does_not_start_a_new_caption() {
        let wave = waveform(&[(1.0, true), (0.2, false), (1.0, true)]);
        let ranges = speech_ranges(&clip(2.2), &wave, SpeechSettings::default());
        assert_eq!(
            into_clip(&ranges),
            vec![(0, 2200)],
            "a 200 ms breath split one sentence into two captions"
        );
    }

    #[test]
    fn a_click_is_not_a_phrase() {
        let wave = waveform(&[(0.5, false), (0.1, true), (2.0, false)]);
        assert!(
            speech_ranges(&clip(2.6), &wave, SpeechSettings::default()).is_empty(),
            "a 100 ms noise became a caption"
        );
    }

    /// A caption has to be readable before it changes, so a long unbroken
    /// stretch is divided evenly.
    #[test]
    fn a_long_stretch_is_broken_into_readable_pieces() {
        let wave = waveform(&[(12.0, true)]);
        let ranges = speech_ranges(&clip(12.0), &wave, SpeechSettings::default());
        assert_eq!(
            into_clip(&ranges),
            vec![(0, 4000), (4000, 8000), (8000, 12000)],
            "twelve seconds should be three even captions, not two of five and one of two"
        );
    }

    #[test]
    fn silence_has_nothing_to_caption() {
        let wave = waveform(&[(4.0, false)]);
        assert!(speech_ranges(&clip(4.0), &wave, SpeechSettings::default()).is_empty());
    }

    /// The pause threshold is a *timeline* duration, so it has to be measured
    /// in the speed the clip plays at. Half a second of source at double speed
    /// is a quarter-second gap to the viewer — shorter than the 350 ms that
    /// ends a phrase, so this is one caption, not two.
    #[test]
    fn a_gap_is_judged_at_the_speed_it_is_heard() {
        let wave = waveform(&[(1.0, true), (0.5, false), (1.0, true)]);
        let mut clip = clip(2.5);
        clip.speed = Rational::new(2, 1).unwrap();
        clip.timeline = TimelineRange::new(TimelineTime::from_seconds(10), ms(11_250)).unwrap();

        let ranges = speech_ranges(&clip, &wave, SpeechSettings::default());
        assert_eq!(
            into_clip(&ranges),
            vec![(0, 1250)],
            "a gap heard as 250 ms was treated as a pause between sentences"
        );
    }

    /// §51: the clip's speed maps source to timeline, here as everywhere. At
    /// double speed a two-second sentence is heard in one second.
    #[test]
    fn speed_moves_the_captions() {
        let wave = waveform(&[(2.0, true), (1.0, false), (2.0, true)]);
        let mut clip = clip(5.0);
        clip.speed = Rational::new(2, 1).unwrap();
        clip.timeline = TimelineRange::new(TimelineTime::from_seconds(10), ms(12_500)).unwrap();

        let ranges = speech_ranges(&clip, &wave, SpeechSettings::default());
        assert_eq!(into_clip(&ranges), vec![(0, 1000), (1500, 2500)]);
    }
}
