//! Finding the beat in a piece of music, for markers to cut on.
//!
//! Works on an amplitude *envelope* — the loudness of short buckets of audio —
//! rather than on samples: the timeline already keeps one for drawing
//! waveforms, so marking the beats of a song costs no decoding at all. A beat
//! in most music is a jump in loudness (a kick, a snare, a strum), and that is
//! visible in an envelope sampled a couple of hundred times a second.
//!
//! Three steps:
//!
//! 1. **Onsets.** How sharply the loudness rises at each moment, on a
//!    logarithmic scale so a quiet verse's beats count as well as the chorus.
//! 2. **Tempo.** The spacing at which the onsets best line up with themselves
//!    (autocorrelation), preferring spacings near a typical tempo so a song is
//!    not heard at half or double its speed.
//! 3. **Tracking.** Beat by beat from the strongest starting point, each one
//!    looked for near where the tempo predicts, so a live recording that
//!    drifts is followed rather than slowly left behind by a rigid grid.
//!
//! Positions come back as envelope indices — integers — and the caller turns
//! them into timeline ticks once. Floating point is used for the analysis and
//! nothing else (§74).

/// What was found.
#[derive(Debug, Clone, PartialEq)]
pub struct BeatGrid {
    /// Envelope indices of each beat, in order.
    pub beats: Vec<usize>,
    /// Beats per minute, for telling the user.
    pub bpm: f64,
}

/// The slowest and fastest tempos considered.
const MIN_BPM: f64 = 60.0;
const MAX_BPM: f64 = 190.0;
/// The tempo the search leans toward when two spacings score alike: the
/// middle of most pop, rock and dance music.
const PREFERRED_BPM: f64 = 118.0;

/// Find the beats in `envelope`, sampled `rate` times a second.
///
/// `None` when there is no beat to find: silence, a held tone, too short to
/// tell (under four seconds), or onsets with no regular spacing at all.
pub fn detect(envelope: &[f32], rate: u32) -> Option<BeatGrid> {
    let rate_f = f64::from(rate.max(1));
    if envelope.len() < (rate_f * 4.0) as usize {
        return None;
    }

    let onsets = onset_strength(envelope, rate);
    let total: f64 = onsets.iter().map(|&o| f64::from(o)).sum();
    if total <= f64::EPSILON {
        return None;
    }

    let period = tempo_period(&onsets, rate_f)?;
    let beats = track(&onsets, period);
    if beats.len() < 4 {
        return None;
    }
    Some(BeatGrid {
        beats,
        bpm: 60.0 * rate_f / period,
    })
}

/// How sharply loudness rises at each index, zero where it falls.
fn onset_strength(envelope: &[f32], rate: u32) -> Vec<f32> {
    // Logarithmic, so the beat of a quiet passage counts like a loud one's.
    let log: Vec<f32> = envelope
        .iter()
        .map(|&v| (1.0 + 20.0 * v.max(0.0)).ln())
        .collect();

    // A short smoothing, so one noisy bucket is not an onset. Over this bucket
    // and the one before only: averaging in the next one too would show every
    // rise a bucket before it happens, and every beat would be marked early.
    let smooth: Vec<f32> = (0..log.len())
        .map(|i| {
            let from = i.saturating_sub(1);
            log[from..=i].iter().sum::<f32>() / (i + 1 - from) as f32
        })
        .collect();

    // The first bucket rises from the silence before the audio began, so a
    // song that opens on a beat has that beat marked.
    let mut onsets: Vec<f32> = smooth
        .first()
        .copied()
        .into_iter()
        .chain(smooth.windows(2).map(|w| (w[1] - w[0]).max(0.0)))
        .collect();

    // Subtract a local average, a quarter-second either side, so a long
    // crescendo is not heard as a string of beats.
    let reach = (rate as usize / 4).max(1);
    let mut prefix = vec![0.0_f64; onsets.len() + 1];
    for (i, &o) in onsets.iter().enumerate() {
        prefix[i + 1] = prefix[i] + f64::from(o);
    }
    for (i, onset) in onsets.iter_mut().enumerate() {
        let from = i.saturating_sub(reach);
        let to = (i + reach + 1).min(prefix.len() - 1);
        let mean = (prefix[to] - prefix[from]) / (to - from) as f64;
        *onset = (f64::from(*onset) - mean).max(0.0) as f32;
    }
    onsets
}

/// The beat spacing, in (fractional) envelope samples.
fn tempo_period(onsets: &[f32], rate: f64) -> Option<f64> {
    let shortest = (60.0 * rate / MAX_BPM).floor() as usize;
    let longest = (60.0 * rate / MIN_BPM).ceil() as usize;
    if shortest < 2 || longest + 1 >= onsets.len() {
        return None;
    }

    let score = |lag: usize| -> f64 {
        onsets
            .iter()
            .zip(&onsets[lag..])
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum::<f64>()
            / (onsets.len() - lag) as f64
    };
    let raw: Vec<f64> = (0..=longest + 1)
        .map(|lag| if lag >= shortest - 1 { score(lag) } else { 0.0 })
        .collect();

    // Weighted toward the preferred tempo on a log scale, an octave either
    // side falling to about a third: a spacing and its double otherwise score
    // nearly alike, and picking the wrong one halves or doubles every mark.
    let weight = |lag: f64| {
        let bpm = 60.0 * rate / lag;
        let octaves = (bpm / PREFERRED_BPM).log2();
        (-0.5 * (octaves / 0.9).powi(2)).exp()
    };
    let (best, best_score) = (shortest..=longest)
        .map(|lag| (lag, raw[lag] * weight(lag as f64)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    if best_score <= 0.0 {
        return None;
    }

    // Parabolic interpolation between the neighbours: the true spacing is
    // rarely a whole number of buckets, and the error adds up across a song.
    let (left, mid, right) = (raw[best - 1], raw[best], raw[best + 1]);
    let curve = left - 2.0 * mid + right;
    let shift = if curve.abs() > f64::EPSILON {
        (0.5 * (left - right) / curve).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some(best as f64 + shift)
}

/// Follow the beat through the onsets, `period` samples apart.
fn track(onsets: &[f32], period: f64) -> Vec<usize> {
    let len = onsets.len();
    let mean = onsets.iter().map(|&o| f64::from(o)).sum::<f64>() / len as f64;

    // Where the grid lines up best with the onsets: the phase whose beats,
    // summed across the whole song, are loudest.
    let steps = period.ceil() as usize;
    let phase = (0..steps)
        .max_by(|&a, &b| {
            let sum = |p: usize| {
                let mut total = 0.0;
                let mut t = p as f64;
                while (t as usize) < len {
                    total += f64::from(onsets[t as usize]);
                    t += period;
                }
                total
            };
            sum(a).total_cmp(&sum(b))
        })
        .unwrap_or(0);

    // Then beat by beat. Each is looked for within a sixth of a beat of where
    // the tempo predicts; a strong enough onset there moves it, and anything
    // weaker leaves the prediction standing, so a gap in the drums does not
    // derail the count.
    let window = (period / 6.0).max(1.0) as usize;
    let mut beats = Vec::new();
    let mut predicted = phase as f64;
    while (predicted.round() as usize) < len {
        let centre = predicted.round() as usize;
        let from = centre.saturating_sub(window);
        let to = (centre + window + 1).min(len);
        let (best, strength) = (from..to)
            .map(|i| (i, f64::from(onsets[i])))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((centre, 0.0));
        let beat = if strength > mean * 2.0 { best } else { centre };
        if beats.last().is_none_or(|&last| beat > last) {
            beats.push(beat);
        }
        // From where the beat actually was, so drift is followed.
        predicted = beat as f64 + period;
    }
    beats
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 200;

    /// A quiet, slightly noisy bed with a drum-like hit at each `(index,
    /// strength)`.
    fn envelope(len: usize, hits: &[(usize, f32)]) -> Vec<f32> {
        let mut seed: u32 = 7;
        let mut envelope: Vec<f32> = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                0.1 + (seed >> 24) as f32 / 255.0 * 0.05
            })
            .collect();
        for &(at, strength) in hits {
            // Decaying over a few buckets, like a drum.
            for (i, level) in [1.0, 0.66, 0.4, 0.22].into_iter().enumerate() {
                if let Some(v) = envelope.get_mut(at + i) {
                    *v = (level * strength).max(*v);
                }
            }
        }
        envelope
    }

    /// Hits `period` apart from `first`, all at full strength.
    fn steady(seconds: usize, period: f64, first: usize) -> (Vec<f32>, Vec<usize>) {
        let len = seconds * RATE as usize;
        let hits: Vec<usize> = (0..)
            .map(|k| (first as f64 + k as f64 * period).round() as usize)
            .take_while(|&t| t < len)
            .collect();
        let with_strength: Vec<_> = hits.iter().map(|&t| (t, 0.9)).collect();
        (envelope(len, &with_strength), hits)
    }

    fn matched(found: &[usize], expected: &[usize], tolerance: usize) -> usize {
        expected
            .iter()
            .filter(|&&e| found.iter().any(|&f| f.abs_diff(e) <= tolerance))
            .count()
    }

    #[test]
    fn a_steady_120_bpm_is_found_beat_for_beat() {
        let (envelope, hits) = steady(20, 100.0, 37);
        let grid = detect(&envelope, RATE).expect("a beat");
        assert!((grid.bpm - 120.0).abs() < 0.5, "heard {} bpm", grid.bpm);
        // On the hit itself, not a bucket either side: a mark a bucket early is
        // five milliseconds early on every beat of the song.
        let found = matched(&grid.beats, &hits, 0);
        assert!(found + 1 >= hits.len(), "matched {found} of {}", hits.len());
    }

    /// 128 bpm is 93.75 buckets a beat. Read as a whole number of buckets the
    /// tempo would be off by a third of a beat per minute.
    #[test]
    fn the_tempo_is_measured_between_buckets() {
        let period = 60.0 * f64::from(RATE) / 128.0;
        let (envelope, _) = steady(60, period, 10);
        let grid = detect(&envelope, RATE).expect("a beat");
        assert!((grid.bpm - 128.0).abs() < 0.15, "heard {} bpm", grid.bpm);
    }

    /// A kick and a softer snare, alternating: every hit is a beat. Its own
    /// spacing and twice it line up almost equally well, and only leaning
    /// toward a usual tempo keeps it from being heard at half speed.
    #[test]
    fn a_kick_and_a_softer_snare_are_both_beats() {
        let len = 30 * RATE as usize;
        let hits: Vec<(usize, f32)> = (0..)
            .map(|k: usize| (20 + k * 100, if k.is_multiple_of(2) { 0.95 } else { 0.6 }))
            .take_while(|&(t, _)| t < len)
            .collect();
        let grid = detect(&envelope(len, &hits), RATE).expect("a beat");
        assert!((grid.bpm - 120.0).abs() < 1.0, "heard {} bpm", grid.bpm);
    }

    /// A live band slowing from 120 to about 109 bpm over forty seconds. A
    /// rigid grid would be half a beat out by the end.
    #[test]
    fn a_drifting_tempo_is_followed() {
        let len = 40 * RATE as usize;
        let mut hits = Vec::new();
        let (mut t, mut period) = (15.0_f64, 100.0_f64);
        while (t as usize) < len {
            hits.push(t.round() as usize);
            t += period;
            period += 0.14;
        }
        let with_strength: Vec<_> = hits.iter().map(|&t| (t, 0.9)).collect();
        let grid = detect(&envelope(len, &with_strength), RATE).expect("a beat");
        let last = &hits[hits.len() - 8..];
        assert_eq!(
            matched(&grid.beats, last, 3),
            last.len(),
            "lost the beat by the end: {:?} against {:?}",
            &grid.beats[grid.beats.len().saturating_sub(8)..],
            last
        );
    }

    #[test]
    fn silence_and_a_held_tone_have_no_beat() {
        assert_eq!(detect(&vec![0.0; 2000], RATE), None);
        assert_eq!(detect(&vec![0.5; 2000], RATE), None);
        assert_eq!(detect(&[0.3; 100], RATE), None, "too short to tell");
    }
}
