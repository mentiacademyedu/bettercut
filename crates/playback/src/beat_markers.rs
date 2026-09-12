//! Where a sound clip's beats fall on the timeline.
//!
//! The analysis is [`bettercut_audio::beats`]; this is the translation around
//! it — from the clip's slice of its file's waveform, and back from positions
//! in that slice to instants on the timeline, through the clip's speed (§51).

use bettercut_cache::Waveform;
use bettercut_foundation::{MediaTime, TICKS_PER_SECOND, TimelineTime};
use bettercut_timeline::{AudioClip, timeline_ticks_for};

/// The timeline instants of `clip`'s beats, and the tempo, or `None` when its
/// sound has no beat to find.
///
/// Only the part of the file the clip plays is analysed, so a song trimmed to
/// its chorus is marked on the chorus's beats, and nothing lands outside the
/// clip.
pub fn beat_markers(clip: &AudioClip, waveform: &Waveform) -> Option<(Vec<TimelineTime>, f64)> {
    let rate = i64::from(waveform.peaks_per_second.max(1));
    // Envelope indices of the clip's source range. Integer division: a bucket
    // either side of an edge makes no difference to where the beats are.
    let bucket = |t: MediaTime| (t.ticks().max(0) * rate / TICKS_PER_SECOND) as usize;
    let from = bucket(clip.source.start).min(waveform.peaks.len());
    let to = bucket(clip.source.end).min(waveform.peaks.len());
    if to <= from {
        return None;
    }
    // Up to a second of the file before the clip, as context. A beat is a
    // *rise* in loudness, and one right on the clip's first frame has no
    // earlier sample to rise from — it would be missed without this.
    let lead = from.min(rate as usize);
    let envelope: Vec<f32> = waveform.peaks[from - lead..to]
        .iter()
        .map(|p| p.magnitude())
        .collect();

    let grid = bettercut_audio::beats::detect(&envelope, rate as u32)?;
    let times = grid
        .beats
        .iter()
        .filter_map(|&index| index.checked_sub(lead))
        .map(|index| {
            // Back from the slice to the source, then through the speed to the
            // timeline — the same mapping as every other one (§51).
            let into_source = MediaTime::from_ticks(index as i64 * TICKS_PER_SECOND / rate);
            clip.timeline.start
                + TimelineTime::from_ticks(timeline_ticks_for(into_source, clip.speed))
        })
        .filter(|&t| t < clip.timeline.end)
        .collect();
    Some((times, grid.bpm * clip.speed.as_f64()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_cache::Peak;
    use bettercut_foundation::{MediaId, Rational};
    use bettercut_timeline::SourceRange;

    /// A 30 s file at 200 peaks a second with a hit every half second.
    fn waveform() -> Waveform {
        let peaks = (0..6_000)
            .map(|i| {
                let into_beat = i % 100;
                let level: i8 = match into_beat {
                    0 => 120,
                    1 => 80,
                    2 => 50,
                    _ => 12,
                };
                Peak {
                    min: -level,
                    max: level,
                }
            })
            .collect();
        Waveform::new(200, peaks).unwrap()
    }

    fn clip(source_from: i64, source_to: i64, at: i64) -> AudioClip {
        AudioClip::new(
            MediaId::new(),
            TimelineTime::from_seconds(at),
            SourceRange::new(
                MediaTime::from_seconds(source_from),
                MediaTime::from_seconds(source_to),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn near(a: TimelineTime, b: TimelineTime) -> bool {
        // Two buckets: 10 ms.
        (a.ticks() - b.ticks()).abs() <= 2 * TICKS_PER_SECOND / 200
    }

    #[test]
    fn beats_land_where_the_clip_plays_them() {
        // Ten seconds from 5 s into the file, placed at 20 s on the timeline.
        let (times, bpm) = beat_markers(&clip(5, 15, 20), &waveform()).expect("a beat");
        assert!((bpm - 120.0).abs() < 1.0, "{bpm} bpm");
        assert!(times.len() >= 18, "{} beats in ten seconds", times.len());
        assert!(
            near(times[0], TimelineTime::from_seconds(20)),
            "{:?}",
            times[0]
        );
        assert!(
            near(times[1], TimelineTime::from_millis(20_500)),
            "{:?}",
            times[1]
        );
        assert!(times.iter().all(|&t| t < TimelineTime::from_seconds(30)));
    }

    /// At double speed the song's beats come twice as often on the timeline.
    #[test]
    fn a_fast_clips_beats_are_closer_together() {
        let mut fast = clip(0, 20, 0);
        fast.speed = Rational::new(2, 1).unwrap();
        fast.timeline.end = TimelineTime::from_seconds(10);

        let (times, bpm) = beat_markers(&fast, &waveform()).expect("a beat");
        assert!((bpm - 240.0).abs() < 2.0, "{bpm} bpm");
        assert!(near(times[1] - times[0], TimelineTime::from_millis(250)));
        assert!(times.iter().all(|&t| t < TimelineTime::from_seconds(10)));
    }

    #[test]
    fn a_clip_past_the_end_of_its_waveform_finds_nothing() {
        assert!(beat_markers(&clip(40, 50, 0), &waveform()).is_none());
    }
}
