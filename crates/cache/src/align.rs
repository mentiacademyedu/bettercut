//! Lining two recordings up by their sound.
//!
//! Two cameras on one scene, or a camera and a separate recorder, hear the
//! same thing at the same instant. Finding that instant by hand means zooming
//! in on a clap and nudging a clip a frame at a time; here the loudness shapes
//! already cached for drawing ([`crate::Waveform`]) are slid over each other
//! and the shift that matches best is the answer.
//!
//! It is the *shapes* that are compared, not the samples: a phone's microphone
//! and a shotgun mic record very different sound from the same room, but the
//! moments that are loud are loud in both. That is also why the comparison is
//! normalised — one recording being quieter than the other says nothing about
//! where it lines up.

use crate::Waveform;

/// The buckets per second both shapes are put on before they are compared.
///
/// Fine enough to land inside a frame (20 ms at 50 Hz, half a frame at 25 fps)
/// and coarse enough that a minute of audio is three thousand numbers, so the
/// whole search is a few million multiplies rather than a few billion.
const RATE: u32 = 50;

/// The least overlap worth believing, in buckets: two seconds.
const MIN_OVERLAP: usize = 2 * RATE as usize;

/// What sliding one recording over another found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    /// How far into `reference` the other recording's start belongs, in
    /// seconds. Negative when it began first.
    pub seconds: f64,
    /// How well the two shapes agree there, 0–1: their correlation at the best
    /// shift. Below about 0.5 the two probably did not hear the same thing.
    pub confidence: f32,
}

impl Alignment {
    /// Whether this is worth acting on without asking.
    pub fn is_convincing(self) -> bool {
        self.confidence >= 0.5
    }
}

/// Where `other` lines up against `reference`, by their loudness shapes.
///
/// `None` when either is too short to say anything — under two seconds of
/// overlap at any shift, or empty.
pub fn align(reference: &Waveform, other: &Waveform) -> Option<Alignment> {
    let a = envelope(reference);
    let b = envelope(other);
    if a.len() < MIN_OVERLAP || b.len() < MIN_OVERLAP {
        return None;
    }

    // Every shift where the two still overlap by the minimum, including the
    // negative ones — the other recording may well have been rolling first.
    let first = -(b.len() as isize) + MIN_OVERLAP as isize;
    let last = a.len() as isize - MIN_OVERLAP as isize;
    let mut best: Option<(isize, f32)> = None;
    for shift in first..=last {
        let Some(score) = correlation(&a, &b, shift) else {
            continue;
        };
        if best.is_none_or(|(_, seen)| score > seen) {
            best = Some((shift, score));
        }
    }
    let (shift, confidence) = best?;
    Some(Alignment {
        seconds: shift as f64 / f64::from(RATE),
        confidence,
    })
}

/// The loudness shape at [`RATE`], with its own average taken out.
///
/// Subtracting the mean is what makes the comparison about *where the loud
/// parts are* rather than about how loud the recording is overall: room tone
/// in both would otherwise agree at every shift.
fn envelope(waveform: &Waveform) -> Vec<f32> {
    if waveform.peaks.is_empty() {
        return Vec::new();
    }
    let seconds = waveform.duration_seconds();
    let buckets = (seconds * f64::from(RATE)).floor() as usize;
    if buckets == 0 {
        return Vec::new();
    }
    let per_bucket = waveform.peaks.len() as f64 / buckets as f64;
    let mut values: Vec<f32> = (0..buckets)
        .map(|bucket| {
            let from = (bucket as f64 * per_bucket).floor() as usize;
            let to = (((bucket + 1) as f64 * per_bucket).ceil() as usize).max(from + 1);
            waveform.peak_over(from, to).magnitude()
        })
        .collect();
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    for value in &mut values {
        *value -= mean;
    }
    values
}

/// How well the two shapes agree with `b` slid `shift` buckets along `a`,
/// 0–1. `None` when they do not overlap enough to be worth comparing.
fn correlation(a: &[f32], b: &[f32], shift: isize) -> Option<f32> {
    let start = shift.max(0) as usize;
    let end = a.len().min((b.len() as isize + shift).max(0) as usize);
    if end <= start || end - start < MIN_OVERLAP {
        return None;
    }
    let overlap = end - start;
    let (mut dot, mut a_energy, mut b_energy) = (0.0_f64, 0.0_f64, 0.0_f64);
    for index in start..end {
        let left = f64::from(a[index]);
        let right = f64::from(b[(index as isize - shift) as usize]);
        dot += left * right;
        a_energy += left * left;
        b_energy += right * right;
    }
    // Silence against anything is no evidence at all.
    if a_energy <= f64::EPSILON || b_energy <= f64::EPSILON {
        return None;
    }
    let agreement = dot / (a_energy.sqrt() * b_energy.sqrt());
    // Weighted by how much of the shorter recording is actually being
    // compared. Without this, sliding until only a second of one clap and one
    // clap overlap scores a perfect match — a shift that agrees about almost
    // nothing should not beat one that agrees about everything.
    let share = overlap as f64 / a.len().min(b.len()) as f64;
    Some((agreement * share).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::Peak;

    /// A shape with bursts of sound at the given seconds, each a fifth of a
    /// second long — a clap track, in other words.
    fn claps(length_seconds: usize, at: &[usize]) -> Waveform {
        let rate = 100;
        let mut peaks = vec![Peak { min: -2, max: 2 }; length_seconds * rate];
        for second in at {
            for index in second * rate..second * rate + rate / 5 {
                if let Some(peak) = peaks.get_mut(index) {
                    *peak = Peak {
                        min: -120,
                        max: 120,
                    };
                }
            }
        }
        Waveform::new(rate as u32, peaks).expect("waveform")
    }

    #[test]
    fn the_same_sound_recorded_late_is_found_where_it_starts() {
        let reference = claps(20, &[3, 7, 12]);
        // The same claps, from a camera that started rolling five seconds in.
        let other = claps(15, &[2, 7]);

        let found = align(&reference, &other).expect("an alignment");
        assert!(
            (found.seconds - 5.0).abs() < 0.1,
            "lined up at {} rather than 5 s",
            found.seconds
        );
        assert!(found.is_convincing(), "confidence {}", found.confidence);
    }

    /// The other way round: the second recording began first, so it belongs
    /// before the reference.
    #[test]
    fn a_recording_that_started_first_lines_up_before_it() {
        let reference = claps(20, &[1, 6]);
        let other = claps(20, &[5, 10]);

        let found = align(&reference, &other).expect("an alignment");
        assert!(
            (found.seconds + 4.0).abs() < 0.1,
            "lined up at {} rather than -4 s",
            found.seconds
        );
    }

    #[test]
    fn two_recordings_of_different_things_are_not_convincing() {
        let reference = claps(20, &[1, 2, 3, 4, 5, 6, 7, 8]);
        // Nothing but room tone: there is no moment to match.
        let other = Waveform::new(100, vec![Peak { min: -2, max: 2 }; 1500]).expect("waveform");
        assert!(align(&reference, &other).is_none_or(|found| !found.is_convincing()));
    }

    #[test]
    fn something_too_short_to_judge_says_nothing() {
        let reference = claps(20, &[1, 6]);
        let blip = claps(1, &[0]);
        assert_eq!(align(&reference, &blip), None);
    }
}
