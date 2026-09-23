//! The moments worth keeping, found in a clip's own sound.
//!
//! An hour of a birthday, a match, a gig: the parts anybody will watch are
//! the loud ones — the cheer, the chorus, the laugh — and finding them by
//! scrubbing is the work nobody does, which is why the hour never gets cut
//! at all.
//!
//! So: read the waveform that is already cached for drawing, take a rolling
//! average of how loud each moment is, and keep the peaks that stand out
//! from the clip's own middle. Its own middle, not a fixed threshold: a
//! quiet room recorded quietly has highlights too, and a level in dB that
//! suits a gig would find none of them.
//!
//! Nothing here decides anything. It hands back stretches; what to do with
//! them — cut to them, mark them, keep only them — is the editor's, and the
//! person's.

use bettercut_cache::Waveform;
use bettercut_foundation::{MediaTime, TICKS_PER_SECOND, TimelineTime};
use bettercut_timeline::{AudioClip, TimelineRange};

/// How the search is tuned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightSettings {
    /// How long each moment is kept for, at least. Shorter than this and a
    /// cut is a flash rather than a moment.
    pub least: TimelineTime,
    /// How much room to leave either side, so a moment does not start on
    /// the syllable.
    pub padding: TimelineTime,
    /// How far above the clip's own middle a moment has to be to count,
    /// as a share of the gap between the middle and the loudest part.
    /// Higher finds fewer, surer moments.
    pub above: f32,
    /// The most to hand back, loudest first.
    pub most: usize,
}

impl Default for HighlightSettings {
    fn default() -> Self {
        Self {
            least: TimelineTime::from_millis(1_500),
            padding: TimelineTime::from_millis(400),
            above: 0.45,
            most: 12,
        }
    }
}

/// The loud stretches of `clip`, in timeline order, at most
/// [`HighlightSettings::most`] of them.
///
/// Empty for a clip with no waveform worth reading, or one that is loud all
/// the way through — a moment that lasts the whole hour is not a moment.
pub fn highlights(
    clip: &AudioClip,
    waveform: &Waveform,
    settings: HighlightSettings,
) -> Vec<TimelineRange> {
    let rate = i64::from(waveform.peaks_per_second.max(1));
    let bucket = |t: MediaTime| (t.ticks().max(0) * rate / TICKS_PER_SECOND) as usize;
    let from = bucket(clip.source.start).min(waveform.peaks.len());
    let to = bucket(clip.source.end).min(waveform.peaks.len());
    if to.saturating_sub(from) < 8 {
        return Vec::new();
    }
    let levels: Vec<f32> = waveform.peaks[from..to]
        .iter()
        .map(|peak| peak.magnitude())
        .collect();

    // Smoothed over about a second, so one drum hit is not a highlight and
    // a chorus is.
    let window = (rate as usize).max(2);
    let smoothed = rolling_mean(&levels, window);
    let middle = median(&smoothed);
    let loudest = smoothed.iter().copied().fold(0.0_f32, f32::max);
    if loudest <= middle + 1e-4 {
        return Vec::new();
    }
    let threshold = middle + (loudest - middle) * settings.above.clamp(0.05, 0.95);

    // The stretches above that line, as bucket ranges.
    let mut runs: Vec<(usize, usize, f32)> = Vec::new();
    let mut start: Option<usize> = None;
    for (index, level) in smoothed.iter().enumerate() {
        match (start, *level >= threshold) {
            (None, true) => start = Some(index),
            (Some(at), false) => {
                runs.push((at, index, peak_of(&smoothed[at..index])));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(at) = start {
        runs.push((at, smoothed.len(), peak_of(&smoothed[at..])));
    }
    if runs.is_empty() {
        return Vec::new();
    }

    // Loudest first, so cutting the list short keeps the best of it.
    runs.sort_by(|a, b| b.2.total_cmp(&a.2));
    runs.truncate(settings.most);

    let at = |index: usize| {
        let into_source = MediaTime::from_ticks(index as i64 * TICKS_PER_SECOND / rate);
        let ticks = bettercut_timeline::timeline_ticks_for(into_source, clip.speed);
        (clip.timeline.start + TimelineTime::from_ticks(ticks)).min(clip.timeline.end)
    };

    let mut found: Vec<TimelineRange> = runs
        .into_iter()
        .filter_map(|(a, b, _)| {
            let start = TimelineTime::from_ticks(
                (at(a).ticks() - settings.padding.ticks()).max(clip.timeline.start.ticks()),
            );
            let end = TimelineTime::from_ticks(
                (at(b).ticks() + settings.padding.ticks()).min(clip.timeline.end.ticks()),
            );
            let end =
                TimelineTime::from_ticks(end.ticks().max(start.ticks() + settings.least.ticks()))
                    .min(clip.timeline.end);
            TimelineRange::new(start, end).ok()
        })
        .collect();

    // Back into timeline order, and merged where the padding made two
    // moments touch: two cuts with nothing between them is one moment.
    found.sort_by_key(|range| range.start.ticks());
    let mut merged: Vec<TimelineRange> = Vec::with_capacity(found.len());
    for range in found {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => {
                if range.end > last.end {
                    last.end = range.end;
                }
            }
            _ => merged.push(range),
        }
    }
    // A "highlight" covering the whole clip says nothing.
    if merged.len() == 1
        && merged[0].start <= clip.timeline.start
        && merged[0].end >= clip.timeline.end
    {
        return Vec::new();
    }
    merged
}

fn peak_of(levels: &[f32]) -> f32 {
    levels.iter().copied().fold(0.0, f32::max)
}

/// The mean of each `window` of `levels`, centred, edges included as they are.
fn rolling_mean(levels: &[f32], window: usize) -> Vec<f32> {
    if window <= 1 || levels.len() <= window {
        return levels.to_vec();
    }
    let half = window / 2;
    let mut sums = Vec::with_capacity(levels.len() + 1);
    sums.push(0.0_f64);
    for level in levels {
        let last = *sums.last().unwrap_or(&0.0);
        sums.push(last + f64::from(*level));
    }
    (0..levels.len())
        .map(|index| {
            let from = index.saturating_sub(half);
            let to = (index + half + 1).min(levels.len());
            ((sums[to] - sums[from]) / (to - from) as f64) as f32
        })
        .collect()
}

/// The middle level, which is what "the clip's own quiet" means here.
fn median(levels: &[f32]) -> f32 {
    if levels.is_empty() {
        return 0.0;
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[sorted.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_cache::Peak;
    use bettercut_foundation::MediaId;
    use bettercut_timeline::SourceRange;

    /// Twenty seconds of quiet with two loud stretches in it.
    fn clip_and_waveform() -> (AudioClip, Waveform) {
        let rate = 10;
        let mut peaks = Vec::new();
        for second in 0..20 {
            for _ in 0..rate {
                let loud = (6..9).contains(&second) || (14..16).contains(&second);
                let level: i8 = if loud { 114 } else { 10 };
                peaks.push(Peak {
                    min: -level,
                    max: level,
                });
            }
        }
        let waveform = Waveform::new(rate as u32, peaks).unwrap();
        let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(20)).unwrap();
        let clip = AudioClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
        (clip, waveform)
    }

    #[test]
    fn the_loud_stretches_are_found_in_order() {
        let (clip, waveform) = clip_and_waveform();
        let found = highlights(&clip, &waveform, HighlightSettings::default());
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].start < found[1].start, "not in order: {found:?}");
        // Around 6–9 s and 14–16 s, with the padding either side.
        assert!(found[0].start <= TimelineTime::from_seconds(6), "{found:?}");
        assert!(found[0].end >= TimelineTime::from_seconds(9), "{found:?}");
        assert!(
            found[1].start <= TimelineTime::from_seconds(14),
            "{found:?}"
        );
        assert!(found[1].end >= TimelineTime::from_seconds(16), "{found:?}");
    }

    #[test]
    fn a_clip_that_is_all_one_level_has_no_highlights() {
        let rate = 10;
        let peaks = vec![Peak { min: -64, max: 64 }; 20 * rate];
        let waveform = Waveform::new(rate as u32, peaks).unwrap();
        let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(20)).unwrap();
        let clip = AudioClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
        assert!(highlights(&clip, &waveform, HighlightSettings::default()).is_empty());
    }

    #[test]
    fn the_list_is_cut_to_the_most_asked_for_keeping_the_loudest() {
        let (clip, waveform) = clip_and_waveform();
        let settings = HighlightSettings {
            most: 1,
            ..HighlightSettings::default()
        };
        let found = highlights(&clip, &waveform, settings);
        assert_eq!(found.len(), 1, "{found:?}");
        // The first stretch is the longer and no quieter, so it is the one
        // kept: both peak at the same level and ties keep the earlier.
        assert!(found[0].start <= TimelineTime::from_seconds(6), "{found:?}");
    }
}
