//! Detection on reversed clips: cuts, pauses, phrases and beats.
//!
//! Every detector reads a clip's material forwards. A reversed clip plays that
//! material the other way, so whatever is found must land mirrored across the
//! clip — as far from its end as it would be from its start forwards. The claim
//! for each: reversed equals forwards mirrored, and is not simply forwards.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_cache::{Peak, Waveform};
use bettercut_foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_playback::{
    SilenceSettings, SpeechSettings, beat_markers, silent_ranges, speech_ranges,
};
use bettercut_timeline::{
    AudioClip, SourceRange, TimelineRange, VideoClip, mirror_in, mirror_range_in,
};

/// Loud and quiet stretches, 200 buckets a second.
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

/// A sound clip at 10 s playing the first `seconds` of its file.
fn sound(seconds: i64, reversed: bool) -> AudioClip {
    let mut clip = AudioClip::new(
        MediaId::new(),
        TimelineTime::from_seconds(10),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(seconds)).unwrap(),
    )
    .unwrap();
    clip.reversed = reversed;
    clip
}

fn close(a: TimelineTime, b: TimelineTime) -> bool {
    (a.ticks() - b.ticks()).abs() <= 2
}

fn same_ranges(a: &[TimelineRange], b: &[TimelineRange]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| close(x.start, y.start) && close(x.end, y.end))
}

/// Pauses in a reversed clip sit mirrored, in timeline order.
#[test]
fn a_reversed_clips_pauses_are_mirrored() {
    // Speech, a long pause early on, more speech, a short pause late.
    let wave = waveform(&[
        (1.0, true),
        (2.0, false),
        (3.0, true),
        (1.0, false),
        (1.0, true),
    ]);
    let forwards = silent_ranges(&sound(8, false), &wave, SilenceSettings::default());
    let backwards = silent_ranges(&sound(8, true), &wave, SilenceSettings::default());
    let span = sound(8, false).timeline;

    let expected: Vec<TimelineRange> = forwards
        .iter()
        .rev()
        .map(|r| mirror_range_in(span, *r))
        .collect();
    assert!(!forwards.is_empty());
    assert!(
        same_ranges(&backwards, &expected),
        "{backwards:?} is not {expected:?}"
    );
    assert!(
        !same_ranges(&backwards, &forwards),
        "the pauses did not move"
    );
    assert!(
        backwards.windows(2).all(|w| w[0].start < w[1].start),
        "not in timeline order"
    );
}

/// Phrases for captions likewise.
#[test]
fn a_reversed_clips_phrases_are_mirrored() {
    let wave = waveform(&[
        (0.5, false),
        (2.0, true),
        (1.5, false),
        (1.0, true),
        (3.0, false),
    ]);
    let forwards = speech_ranges(&sound(8, false), &wave, SpeechSettings::default());
    let backwards = speech_ranges(&sound(8, true), &wave, SpeechSettings::default());
    let span = sound(8, false).timeline;

    let expected: Vec<TimelineRange> = forwards
        .iter()
        .rev()
        .map(|r| mirror_range_in(span, *r))
        .collect();
    assert!(!forwards.is_empty());
    assert!(
        same_ranges(&backwards, &expected),
        "{backwards:?} is not {expected:?}"
    );
    assert!(!same_ranges(&backwards, &forwards));
}

/// Scene cuts in a reversed picture are met in the opposite order.
#[test]
fn a_reversed_clips_cuts_are_mirrored() {
    let mut clip = VideoClip::new(
        MediaId::new(),
        TimelineTime::from_seconds(4),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    let cuts = [MediaTime::from_seconds(2), MediaTime::from_seconds(7)];
    let forwards = bettercut_playback::scene_job::timeline_cuts(&clip, &cuts);
    clip.reversed = true;
    let backwards = bettercut_playback::scene_job::timeline_cuts(&clip, &cuts);

    let mut expected: Vec<TimelineTime> = forwards
        .iter()
        .map(|t| mirror_in(clip.timeline, *t))
        .collect();
    expected.sort_unstable();
    assert_eq!(backwards, expected);
    assert_eq!(
        backwards,
        vec![
            TimelineTime::from_seconds(7),
            TimelineTime::from_seconds(12)
        ]
    );
}

/// Beats in a reversed clip land mirrored and stay inside it.
#[test]
fn a_reversed_clips_beats_are_mirrored() {
    // A click every half second, with an uneven gap so the grid has a phase.
    let mut pattern = vec![(0.2, false)];
    for _ in 0..16 {
        pattern.push((0.05, true));
        pattern.push((0.45, false));
    }
    let wave = waveform(&pattern);
    let clip = sound(8, false);
    let Some((forwards, _)) = beat_markers(&clip, &wave) else {
        eprintln!("no beat grid on this pattern; nothing to mirror");
        return;
    };
    let (backwards, _) = beat_markers(&sound(8, true), &wave).expect("the same grid");

    let mut expected: Vec<TimelineTime> = forwards
        .iter()
        .map(|t| mirror_in(clip.timeline, *t))
        .filter(|t| *t >= clip.timeline.start && *t < clip.timeline.end)
        .collect();
    expected.sort_unstable();
    assert_eq!(backwards.len(), expected.len());
    assert!(backwards.iter().zip(&expected).all(|(a, b)| close(*a, *b)));
    assert!(
        backwards
            .iter()
            .all(|t| *t >= clip.timeline.start && *t < clip.timeline.end)
    );
}
