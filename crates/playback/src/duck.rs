//! Ducking music under a voice (§20a.4's clip-gain stage, §24's keyframes).
//!
//! The most common mix note in the world: *the music is too loud under the
//! talking*. Done by hand it is a keyframe either side of every sentence, and
//! the sentences are where [`crate::speech`] already found them — so what is
//! left is the shape of the dip, which is what this is.
//!
//! Four keys per duck: full level before it, down at the start of the speech,
//! held to the end, and back up after. The times either side are what make it
//! sound deliberate rather than broken — a music bed that drops instantly is
//! heard as a fault, and one that takes two seconds to come back sounds like
//! someone asleep at the fader.

use bettercut_foundation::TimelineTime;
use bettercut_timeline::{AudioClip, TimelineRange};

/// How the dip is shaped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuckSettings {
    /// The level under speech, as a fraction of the clip's own. 0.25 is about
    /// 12 dB down, which is where a bed sits under a voice without vanishing.
    pub depth: f32,
    /// How long the drop takes. Short, but not instant: an instant drop is
    /// heard as a fault in the recording.
    pub attack: TimelineTime,
    /// How long the recovery takes. Longer than the attack, because coming
    /// back up is the part a listener notices.
    pub release: TimelineTime,
    /// Speech separated by less than this stays one duck. Between two
    /// sentences the music must not surge back for half a second and drop
    /// again — that pumping is worse than leaving it down.
    pub shortest_gap: TimelineTime,
}

impl Default for DuckSettings {
    fn default() -> Self {
        Self {
            depth: 0.25,
            attack: TimelineTime::from_millis(150),
            release: TimelineTime::from_millis(500),
            shortest_gap: TimelineTime::from_millis(1_500),
        }
    }
}

/// The volume envelope that ducks `music` under `speech`.
///
/// Points are timeline instants and levels, in order, for
/// `Editor::set_gain_envelope`. Empty when no speech touches the clip, which is
/// the right answer rather than a flat envelope of ones: a clip with nothing to
/// duck under should be left alone entirely.
///
/// `speech` may be in any order and may overlap; it is sorted and merged here,
/// because it usually comes from several clips on several tracks.
pub fn duck_envelope(
    music: &AudioClip,
    speech: &[TimelineRange],
    settings: DuckSettings,
) -> Vec<(TimelineTime, f32)> {
    let span = music.timeline;
    let depth = settings.depth.clamp(0.0, 1.0);

    // Only the speech this clip is playing under, merged so that the music
    // does not surge between sentences.
    let mut ranges: Vec<TimelineRange> = speech
        .iter()
        .filter(|range| range.start < span.end && range.end > span.start)
        .copied()
        .collect();
    ranges.sort_by_key(|range| range.start.ticks());

    let mut merged: Vec<TimelineRange> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start - last.end < settings.shortest_gap => {
                last.end = last.end.max(range.end);
            }
            _ => merged.push(range),
        }
    }
    if merged.is_empty() {
        return Vec::new();
    }

    let mut points: Vec<(TimelineTime, f32)> = Vec::new();
    let push = |at: TimelineTime, level: f32, points: &mut Vec<(TimelineTime, f32)>| {
        // Inside the clip, and never behind the key before it: a duck starting
        // in the first moments of the clip has nowhere to put its attack, and
        // two keys at one instant would be one key at whichever level landed
        // last.
        let at = at.clamp(span.start, span.end);
        match points.last_mut() {
            Some((last_at, last_level)) if *last_at >= at => *last_level = level,
            _ => points.push((at, level)),
        }
    };

    for range in merged {
        push(range.start - settings.attack, 1.0, &mut points);
        push(range.start, depth, &mut points);
        push(range.end, depth, &mut points);
        push(range.end + settings.release, 1.0, &mut points);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::{MediaId, MediaTime};
    use bettercut_timeline::SourceRange;

    fn ms(v: i64) -> TimelineTime {
        TimelineTime::from_millis(v)
    }

    /// Thirty seconds of music from timeline zero.
    fn music() -> AudioClip {
        AudioClip::new(
            MediaId::new(),
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(30)).unwrap(),
        )
        .unwrap()
    }

    fn range(from: i64, to: i64) -> TimelineRange {
        TimelineRange::new(ms(from), ms(to)).unwrap()
    }

    /// Points as (milliseconds, level), for readable assertions.
    fn shape(points: &[(TimelineTime, f32)]) -> Vec<(i64, f32)> {
        points
            .iter()
            .map(|(at, level)| (at.ticks() / 960, *level))
            .collect()
    }

    #[test]
    fn one_sentence_is_four_keys() {
        let points = duck_envelope(&music(), &[range(5_000, 8_000)], DuckSettings::default());
        assert_eq!(
            shape(&points),
            vec![
                (4_850, 1.0),  // full level, an attack before the words
                (5_000, 0.25), // down as they start
                (8_000, 0.25), // held to the end of them
                (8_500, 1.0),  // and back up over the release
            ]
        );
    }

    /// Between two sentences the music must not surge back and drop again.
    #[test]
    fn a_short_gap_stays_ducked() {
        let points = duck_envelope(
            &music(),
            &[range(5_000, 8_000), range(8_800, 12_000)],
            DuckSettings::default(),
        );
        assert_eq!(
            shape(&points),
            vec![(4_850, 1.0), (5_000, 0.25), (12_000, 0.25), (12_500, 1.0)],
            "the music came back up for 800 ms between two sentences"
        );
    }

    /// A real pause is worth coming back up for.
    #[test]
    fn a_long_gap_comes_back_up() {
        let points = duck_envelope(
            &music(),
            &[range(5_000, 8_000), range(14_000, 16_000)],
            DuckSettings::default(),
        );
        assert_eq!(shape(&points).len(), 8, "{:?}", shape(&points));
        assert_eq!(shape(&points)[3], (8_500, 1.0));
        assert_eq!(shape(&points)[4], (13_850, 1.0));
    }

    /// Speech somewhere else on the timeline is not this clip's business.
    #[test]
    fn speech_outside_the_clip_is_ignored() {
        let mut music = music();
        music.timeline = TimelineRange::new(ms(20_000), ms(30_000)).unwrap();
        let points = duck_envelope(&music, &[range(1_000, 3_000)], DuckSettings::default());
        assert!(points.is_empty(), "{:?}", shape(&points));
    }

    #[test]
    fn nothing_to_duck_under_leaves_no_envelope() {
        assert!(duck_envelope(&music(), &[], DuckSettings::default()).is_empty());
    }

    /// Speech starting before the clip does: the attack has nowhere to go, so
    /// the clip simply starts ducked rather than opening at full level for an
    /// instant and slamming down.
    #[test]
    fn a_duck_at_the_very_start_has_no_room_to_ramp() {
        let points = duck_envelope(&music(), &[range(0, 4_000)], DuckSettings::default());
        assert_eq!(
            shape(&points),
            vec![(0, 0.25), (4_000, 0.25), (4_500, 1.0)],
            "the clip opened at full level and slammed down"
        );
    }

    /// Points have to come out in order and never twice at one instant, or the
    /// later key silently replaces the earlier one and the shape is not the
    /// shape that was asked for.
    #[test]
    fn the_points_are_ordered_and_distinct() {
        let points = duck_envelope(
            &music(),
            &[
                range(500, 1_000),
                range(3_000, 3_200),
                range(20_000, 29_900),
            ],
            DuckSettings::default(),
        );
        assert!(
            points.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "out of order or doubled: {:?}",
            shape(&points)
        );
        assert!(
            points
                .iter()
                .all(|(at, _)| *at >= TimelineTime::ZERO && *at <= TimelineTime::from_seconds(30)),
            "a key landed outside the clip: {:?}",
            shape(&points)
        );
    }
}
