//! Waveform analysis as a background job (§12, §19).
//!
//! Unlike a thumbnail, this reads the **whole** audio stream — there is no
//! shortcut to knowing where the quiet parts are. That makes it the one job
//! whose cost scales with media length, so it reports progress honestly and
//! checks cancellation on every packet (§48).
//!
//! It is still not `is_heavy()`: decoding audio is a fraction of the work of
//! encoding a proxy, and blocking the one heavy slot with it would delay the
//! thing the user is actually waiting for (§80).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bettercut_cache::{CacheStore, PEAKS_PER_SECOND, Peak, Waveform};
use bettercut_foundation::MediaId;
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::{FfmpegDecoder, MediaAsset, MediaDecoder};

/// Bridges the scheduler's cancellation to the media crate's token (§86).
struct JobCancellation {
    cancelled: Arc<AtomicBool>,
}

impl bettercut_media::CancellationToken for JobCancellation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Analyse one asset's audio into cached peaks.
pub struct WaveformJob {
    asset: MediaAsset,
    output: std::path::PathBuf,
    threads: u32,
}

impl WaveformJob {
    /// Prepare a job, or `None` when there is nothing to analyse.
    pub fn new(asset: &MediaAsset, cache: &CacheStore, threads: u32) -> Option<Self> {
        // No audio stream means no waveform. Checked from the probe rather than
        // by opening the file, so a library of stills queues nothing at all.
        if asset.missing || asset.audio_channels.unwrap_or(0) == 0 {
            return None;
        }
        let output = cache.layout().waveform_file(asset.id);
        if output.exists() {
            return None;
        }

        Some(Self {
            asset: asset.clone(),
            output,
            threads: threads.max(1),
        })
    }

    pub fn media(&self) -> MediaId {
        self.asset.id
    }
}

impl Task for WaveformJob {
    fn label(&self) -> String {
        format!("Analysing audio in {}", self.asset.file_name)
    }

    fn priority(&self) -> Priority {
        Priority::Background
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = JobCancellation {
            cancelled: Arc::clone(&cancelled),
        };

        let mut decoder = FfmpegDecoder::new(self.threads).map_err(|e| e.to_string())?;
        decoder.open(&self.asset).map_err(|e| e.to_string())?;

        let total_seconds = self.asset.duration.as_seconds_f64().max(0.0);
        let mut builder = PeakBuilder::new(PEAKS_PER_SECOND);

        loop {
            if ctx.is_cancelled() {
                cancelled.store(true, Ordering::Release);
                return Err("cancelled".to_owned());
            }

            let Some(buffer) = decoder.decode_audio(&token).map_err(|e| e.to_string())? else {
                break;
            };
            builder.push(&buffer);

            if total_seconds > 0.0 {
                let done = builder.seconds_consumed() / total_seconds;
                ctx.progress(done.clamp(0.0, 0.99) as f32);
            }
        }

        let waveform = builder.finish().map_err(|e| e.to_string())?;
        waveform.write(&self.output).map_err(|e| e.to_string())?;
        ctx.progress(1.0);

        tracing::debug!(
            file = %self.asset.file_name,
            peaks = waveform.peaks.len(),
            "waveform ready"
        );
        Ok(())
    }
}

/// Accumulates decoded audio into fixed-width peak buckets.
///
/// Kept separate from the job so it can be tested without a decoder — the
/// bucketing is the part with arithmetic worth getting wrong.
struct PeakBuilder {
    peaks_per_second: u32,
    peaks: Vec<Peak>,
    /// Samples folded into the bucket currently being built.
    current: Peak,
    current_count: u32,
    /// Samples that make up one bucket, from the stream's own rate.
    samples_per_bucket: u32,
    total_samples: u64,
    sample_rate: u32,
}

impl PeakBuilder {
    fn new(peaks_per_second: u32) -> Self {
        Self {
            peaks_per_second,
            peaks: Vec::new(),
            current: Peak::default(),
            current_count: 0,
            // Replaced by the real rate on the first buffer; 48 kHz is the
            // §20a.3 internal rate and what the decoder resamples to.
            samples_per_bucket: (48_000 / peaks_per_second).max(1),
            total_samples: 0,
            sample_rate: 48_000,
        }
    }

    fn seconds_consumed(&self) -> f64 {
        self.total_samples as f64 / f64::from(self.sample_rate.max(1))
    }

    fn push(&mut self, buffer: &bettercut_media::AudioBuffer) {
        if buffer.sample_rate != self.sample_rate && buffer.sample_rate > 0 {
            self.sample_rate = buffer.sample_rate;
            self.samples_per_bucket = (buffer.sample_rate / self.peaks_per_second).max(1);
        }
        let Some(first) = buffer.planes.first() else {
            return;
        };
        let channels = buffer.planes.len().max(1) as f32;

        for i in 0..first.len() {
            // Mono mixdown: the timeline draws one lane per clip, so summing
            // and dividing is what a listener would hear folded down.
            let mut sum = 0.0_f32;
            for plane in &buffer.planes {
                sum += plane.get(i).copied().unwrap_or(0.0);
            }
            let sample = (sum / channels).clamp(-1.0, 1.0);

            // Round toward zero on the negative side too, so silence stays 0.
            let scaled = (sample * 127.0) as i8;
            self.current.min = self.current.min.min(scaled);
            self.current.max = self.current.max.max(scaled);
            self.current_count += 1;
            self.total_samples += 1;

            if self.current_count >= self.samples_per_bucket {
                self.peaks.push(self.current);
                self.current = Peak::default();
                self.current_count = 0;
            }
        }
    }

    fn finish(mut self) -> Result<Waveform, bettercut_cache::CacheError> {
        // Keep the partial bucket: dropping it would make every waveform end
        // slightly short of its clip, which shows as a gap at the right edge.
        if self.current_count > 0 {
            self.peaks.push(self.current);
        }
        Waveform::new(self.peaks_per_second, self.peaks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::MediaTime;
    use bettercut_media::AudioBuffer;

    fn buffer(samples: Vec<f32>, channels: usize) -> AudioBuffer {
        AudioBuffer {
            timestamp: MediaTime::ZERO,
            planes: vec![samples; channels],
            sample_rate: 48_000,
        }
    }

    #[test]
    fn a_full_scale_tone_reaches_the_top_of_the_range() {
        let mut builder = PeakBuilder::new(PEAKS_PER_SECOND);
        // One second of alternating +1/-1 at 48 kHz.
        let samples: Vec<f32> = (0..48_000)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        builder.push(&buffer(samples, 2));

        let waveform = builder.finish().expect("valid");
        assert_eq!(waveform.peaks.len(), PEAKS_PER_SECOND as usize);
        for peak in &waveform.peaks {
            assert_eq!(peak.max, 127);
            assert_eq!(peak.min, -127);
        }
    }

    #[test]
    fn silence_produces_flat_peaks() {
        let mut builder = PeakBuilder::new(PEAKS_PER_SECOND);
        builder.push(&buffer(vec![0.0; 48_000], 1));

        let waveform = builder.finish().expect("valid");
        assert!(!waveform.peaks.is_empty());
        for peak in &waveform.peaks {
            assert_eq!(*peak, Peak::default(), "silence was not flat");
        }
    }

    /// One second of audio must produce one second of peaks, or the waveform
    /// drifts against the clip it is drawn inside.
    #[test]
    fn the_bucket_count_matches_the_duration() {
        let mut builder = PeakBuilder::new(PEAKS_PER_SECOND);
        builder.push(&buffer(vec![0.5; 48_000 * 3], 1));

        let waveform = builder.finish().expect("valid");
        assert_eq!(waveform.peaks.len(), PEAKS_PER_SECOND as usize * 3);
        assert!((waveform.duration_seconds() - 3.0).abs() < 1e-9);
    }

    /// A trailing partial bucket must be kept, or every clip ends short.
    #[test]
    fn a_partial_final_bucket_is_kept() {
        let mut builder = PeakBuilder::new(PEAKS_PER_SECOND);
        // One and a half buckets' worth.
        let samples = (48_000 / PEAKS_PER_SECOND as usize) * 3 / 2;
        builder.push(&buffer(vec![1.0; samples], 1));

        let waveform = builder.finish().expect("valid");
        assert_eq!(waveform.peaks.len(), 2);
        assert_eq!(waveform.peaks[1].max, 127, "the tail bucket lost its audio");
    }

    /// Channels are folded down, not concatenated: opposite-phase stereo is
    /// quiet when summed, and drawing it as loud would be a lie.
    #[test]
    fn opposite_phase_stereo_folds_down_to_near_silence() {
        let mut builder = PeakBuilder::new(PEAKS_PER_SECOND);
        let left = vec![1.0_f32; 48_000];
        let right = vec![-1.0_f32; 48_000];
        builder.push(&AudioBuffer {
            timestamp: MediaTime::ZERO,
            planes: vec![left, right],
            sample_rate: 48_000,
        });

        let waveform = builder.finish().expect("valid");
        for peak in &waveform.peaks {
            assert_eq!(peak.magnitude(), 0.0, "cancelled audio drew as loud");
        }
    }

    /// A source at a different rate must still yield one second of buckets per
    /// second of audio.
    #[test]
    fn a_non_default_sample_rate_still_buckets_per_second() {
        let mut builder = PeakBuilder::new(PEAKS_PER_SECOND);
        builder.push(&AudioBuffer {
            timestamp: MediaTime::ZERO,
            planes: vec![vec![0.25_f32; 44_100]],
            sample_rate: 44_100,
        });

        let waveform = builder.finish().expect("valid");
        // 44,100 / 200 = 220 samples per bucket, so 200 full buckets plus the
        // 100-sample remainder.
        assert_eq!(waveform.peaks.len(), PEAKS_PER_SECOND as usize + 1);
    }

    #[test]
    fn an_empty_stream_produces_an_empty_waveform() {
        let builder = PeakBuilder::new(PEAKS_PER_SECOND);
        let waveform = builder.finish().expect("valid");
        assert!(waveform.peaks.is_empty());
    }
}
