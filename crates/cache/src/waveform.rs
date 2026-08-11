//! Cached audio peaks (§19).
//!
//! An audio clip drawn as a flat block tells the user nothing. What they need
//! is the shape — where the speech is, where the silence is, where the beat
//! lands — and that has to be readable while scrolling a long timeline, so it
//! cannot be computed from the samples on demand.
//!
//! # Peaks, not samples
//!
//! A minute of 48 kHz stereo is 5.8 million samples per channel. At any zoom a
//! screen pixel covers hundreds of them, and the only thing that survives is
//! the extremes: drawing the *mean* of a loud passage gives a thin line,
//! because audio is symmetric around zero. So each bucket stores the minimum
//! and maximum it saw, which is what produces the familiar mirrored shape.
//!
//! [`PEAKS_PER_SECOND`] buckets per second, one byte each for min and max.
//! A minute costs 24 KB; an hour costs 1.4 MB.

use std::io::{Read, Write};
use std::path::Path;

use crate::error::CacheError;

/// `BCW1` — magic plus version.
const MAGIC: [u8; 4] = *b"BCW1";
const HEADER_BYTES: usize = 12;

/// Buckets per second of audio.
///
/// 200 is chosen against the zoom ladder rather than picked round: the closest
/// zoom level is 500 ticks/pixel, which is 1,920 pixels per second, so even
/// fully zoomed in there are ~9.6 pixels per bucket. Past that the drawing
/// interpolates, which is honest — the alternative is storing samples.
pub const PEAKS_PER_SECOND: u32 = 200;

/// One bucket: the extremes of the samples it covers, as signed bytes.
///
/// `i8` rather than `f32` because the result is drawn a few pixels tall. Eight
/// bits is over 100 dB of range for display purposes and makes the file a
/// quarter the size, which matters when scrolling a long timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Peak {
    pub min: i8,
    pub max: i8,
}

impl Peak {
    /// Loudness of this bucket, 0..=1, for a simple bar-style draw.
    pub fn magnitude(self) -> f32 {
        let low = f32::from(self.min).abs();
        let high = f32::from(self.max).abs();
        low.max(high) / 127.0
    }
}

/// Peaks for one asset, mixed down to mono.
///
/// Mono because the timeline draws one lane per clip, not one per channel;
/// keeping both would double the file to show something never displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waveform {
    pub peaks_per_second: u32,
    pub peaks: Vec<Peak>,
}

impl Waveform {
    pub fn new(peaks_per_second: u32, peaks: Vec<Peak>) -> Result<Self, CacheError> {
        if peaks_per_second == 0 {
            return Err(CacheError::MalformedWaveform {
                detail: "zero peaks per second".to_owned(),
            });
        }
        Ok(Self {
            peaks_per_second,
            peaks,
        })
    }

    /// Seconds of audio this covers.
    pub fn duration_seconds(&self) -> f64 {
        self.peaks.len() as f64 / f64::from(self.peaks_per_second)
    }

    /// The loudest bucket in a half-open range, for one column of pixels.
    ///
    /// Ranges are clamped rather than rejected: a clip can be trimmed past the
    /// end of its own peaks when the media is shorter than the project claims,
    /// and that should draw flat, not panic.
    pub fn peak_over(&self, from: usize, to: usize) -> Peak {
        let from = from.min(self.peaks.len());
        let to = to.clamp(from, self.peaks.len());
        let slice = &self.peaks[from..to];

        // An empty slice means the column fell between buckets; the nearest
        // one still describes it better than silence.
        if slice.is_empty() {
            return self.peaks.get(from).copied().unwrap_or_default();
        }

        slice.iter().fold(Peak::default(), |acc, p| Peak {
            min: acc.min.min(p.min),
            max: acc.max.max(p.max),
        })
    }

    pub fn write(&self, path: &Path) -> Result<(), CacheError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|s| io(parent, s))?;
        }

        let temp = path.with_extension("part");
        {
            let mut file = std::fs::File::create(&temp).map_err(|s| io(&temp, s))?;
            let mut bytes = Vec::with_capacity(HEADER_BYTES + self.peaks.len() * 2);
            bytes.extend_from_slice(&MAGIC);
            bytes.extend_from_slice(&self.peaks_per_second.to_le_bytes());
            bytes.extend_from_slice(&(self.peaks.len() as u32).to_le_bytes());
            for peak in &self.peaks {
                bytes.push(peak.min as u8);
                bytes.push(peak.max as u8);
            }
            file.write_all(&bytes).map_err(|s| io(&temp, s))?;
            file.flush().map_err(|s| io(&temp, s))?;
        }
        std::fs::rename(&temp, path).map_err(|s| io(path, s))?;
        Ok(())
    }

    pub fn read(path: &Path) -> Result<Self, CacheError> {
        let mut file = std::fs::File::open(path).map_err(|s| io(path, s))?;

        let mut header = [0_u8; HEADER_BYTES];
        file.read_exact(&mut header).map_err(|s| io(path, s))?;
        if header[..4] != MAGIC {
            return Err(CacheError::MalformedWaveform {
                detail: "wrong magic; not a bettercut waveform".to_owned(),
            });
        }

        let peaks_per_second = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        let count = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;

        if peaks_per_second == 0 {
            return Err(CacheError::MalformedWaveform {
                detail: "zero peaks per second".to_owned(),
            });
        }
        // 24 hours at the stated rate. A corrupted count must not become a
        // multi-gigabyte allocation.
        let ceiling = peaks_per_second as usize * 60 * 60 * 24;
        if count > ceiling {
            return Err(CacheError::MalformedWaveform {
                detail: format!("{count} peaks is implausible"),
            });
        }

        let mut raw = vec![0_u8; count * 2];
        file.read_exact(&mut raw).map_err(|s| io(path, s))?;

        let peaks = raw
            .chunks_exact(2)
            .map(|c| Peak {
                min: c[0] as i8,
                max: c[1] as i8,
            })
            .collect();

        Ok(Self {
            peaks_per_second,
            peaks,
        })
    }
}

fn io(path: &Path, source: std::io::Error) -> CacheError {
    CacheError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Waveform {
        Waveform::new(
            PEAKS_PER_SECOND,
            (0..500)
                .map(|i| Peak {
                    min: -(i % 100) as i8,
                    max: (i % 100) as i8,
                })
                .collect(),
        )
        .expect("valid")
    }

    #[test]
    fn a_waveform_round_trips_through_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("w.bin");
        let original = sample();

        original.write(&path).expect("write");
        assert_eq!(Waveform::read(&path).expect("read"), original);
    }

    #[test]
    fn duration_follows_the_peak_count() {
        let waveform = sample();
        assert!((waveform.duration_seconds() - 2.5).abs() < 1e-9);
    }

    /// The point of storing min and max: a column must report the extremes it
    /// covers, not an average that would flatten every loud passage.
    #[test]
    fn a_column_reports_the_extremes_it_covers() {
        let waveform = Waveform::new(
            10,
            vec![
                Peak { min: -5, max: 5 },
                Peak { min: -100, max: 90 },
                Peak { min: -2, max: 2 },
            ],
        )
        .expect("valid");

        let peak = waveform.peak_over(0, 3);
        assert_eq!(peak.min, -100);
        assert_eq!(peak.max, 90);
    }

    /// A clip trimmed past the end of its peaks must draw flat, not panic.
    #[test]
    fn a_range_past_the_end_is_clamped() {
        let waveform = sample();
        let peak = waveform.peak_over(100_000, 200_000);
        assert_eq!(peak, Peak::default());
    }

    #[test]
    fn a_reversed_range_is_not_a_panic() {
        let waveform = sample();
        let _ = waveform.peak_over(200, 100);
    }

    #[test]
    fn magnitude_is_normalised() {
        assert_eq!(Peak { min: 0, max: 0 }.magnitude(), 0.0);
        assert!((Peak { min: -127, max: 0 }.magnitude() - 1.0).abs() < 1e-6);
        assert!((Peak { min: 0, max: 127 }.magnitude() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_file_without_the_magic_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("w.bin");
        std::fs::write(&path, b"definitely not a waveform").expect("write");

        let err = Waveform::read(&path).expect_err("should refuse");
        assert!(format!("{err}").contains("magic"), "{err}");
    }

    #[test]
    fn a_truncated_file_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("w.bin");
        sample().write(&path).expect("write");

        let full = std::fs::read(&path).expect("read");
        std::fs::write(&path, &full[..full.len() - 7]).expect("truncate");

        assert!(Waveform::read(&path).is_err());
    }

    #[test]
    fn an_implausible_count_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("w.bin");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&PEAKS_PER_SECOND.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, &bytes).expect("write");

        assert!(Waveform::read(&path).is_err());
    }

    /// An empty waveform is legitimate — a silent file still has peaks of zero,
    /// and refusing it would mean regenerating it on every launch.
    #[test]
    fn an_empty_waveform_is_valid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("w.bin");
        let empty = Waveform::new(PEAKS_PER_SECOND, Vec::new()).expect("valid");

        empty.write(&path).expect("write");
        assert_eq!(Waveform::read(&path).expect("read"), empty);
    }
}
