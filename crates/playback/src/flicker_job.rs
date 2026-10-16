//! Measuring a clip's flicker, off the interface thread
//! ([`bettercut_timeline::deflicker`]).
//!
//! Every frame of the clip's stretch of the file is decoded and its overall
//! brightness measured; the gains that even the brightness out are what the
//! job leaves in its [`FlickerReport`]. Like finding cuts, it decodes the
//! footage, so it is a job: cancellable at every frame and never on the
//! interface thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bettercut_foundation::{ClipId, MediaTime};
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::{
    FfmpegDecoder, FrameStorage, MediaAsset, MediaDecoder, MediaKind, SeekMode, VideoFrame,
};
use bettercut_timeline::deflicker::FlickerGains;

struct JobCancellation {
    cancelled: Arc<AtomicBool>,
}

impl bettercut_media::CancellationToken for JobCancellation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Where a finished measurement leaves its answer: the gains, or why there
/// are none. Shared with the running job, read by the interface when the job
/// reports finished.
#[derive(Debug, Clone, Default)]
pub struct FlickerReport {
    answer: Arc<Mutex<Option<Result<FlickerGains, String>>>>,
}

impl FlickerReport {
    /// The answer, taken: `None` while the job is still running, and after
    /// it has been taken once.
    pub fn take(&self) -> Option<Result<FlickerGains, String>> {
        self.answer.lock().ok()?.take()
    }

    /// Record the answer. Public so a test can play the worker's part.
    pub fn finish(&self, answer: Result<FlickerGains, String>) {
        if let Ok(mut held) = self.answer.lock() {
            *held = Some(answer);
        }
    }
}

/// A frame's average linear luminance, measured on a grid of samples: the
/// overall brightness, which is all flicker changes. Linear because the gain
/// it becomes multiplies linear light.
pub fn brightness(frame: &VideoFrame) -> Option<f32> {
    let FrameStorage::System { data, stride } = &frame.storage else {
        return None;
    };
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    // sRGB to linear, once per level.
    let linear: [f32; 256] = std::array::from_fn(|level| {
        let v = level as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    let stride = *stride as usize;
    // About 64 × 36 samples whatever the size: plenty for an average.
    let (step_x, step_y) = (
        (frame.width as usize / 64).max(1),
        (frame.height as usize / 36).max(1),
    );
    let mut sum = 0.0_f64;
    let mut count = 0_u32;
    for y in (step_y / 2..frame.height as usize).step_by(step_y) {
        for x in (step_x / 2..frame.width as usize).step_by(step_x) {
            let at = y * stride + x * 4;
            let Some(pixel) = data.get(at..at + 3) else {
                continue;
            };
            let luma = 0.2126 * linear[usize::from(pixel[0])]
                + 0.7152 * linear[usize::from(pixel[1])]
                + 0.0722 * linear[usize::from(pixel[2])];
            sum += f64::from(luma);
            count += 1;
        }
    }
    (count > 0).then(|| (sum / f64::from(count)) as f32)
}

/// Measure the flicker in one stretch of one file, for one clip.
pub struct FlickerJob {
    clip: ClipId,
    asset: MediaAsset,
    from: MediaTime,
    to: MediaTime,
    threads: u32,
    report: FlickerReport,
}

impl FlickerJob {
    /// Prepare a job over `[from, to)` of `asset` for `clip`, or `None` when
    /// there are no moving pictures to measure.
    pub fn new(
        clip: ClipId,
        asset: &MediaAsset,
        from: MediaTime,
        to: MediaTime,
        threads: u32,
    ) -> Option<(Self, FlickerReport)> {
        if asset.kind != MediaKind::Video || asset.missing || to <= from {
            return None;
        }
        let report = FlickerReport::default();
        Some((
            Self {
                clip,
                asset: asset.clone(),
                from,
                to,
                threads: threads.max(1),
                report: report.clone(),
            },
            report,
        ))
    }

    pub fn clip(&self) -> ClipId {
        self.clip
    }

    fn measure(&self, ctx: &JobContext) -> Result<FlickerGains, String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = JobCancellation {
            cancelled: Arc::clone(&cancelled),
        };
        let mut decoder = FfmpegDecoder::new(self.threads).map_err(|e| e.to_string())?;
        decoder.open(&self.asset).map_err(|e| e.to_string())?;
        if !self.from.is_zero() {
            decoder
                .seek(self.from, SeekMode::Precise)
                .map_err(|e| e.to_string())?;
        }
        let span = (self.to - self.from).ticks().max(1);
        let mut frames = Vec::new();
        while let Some(frame) = decoder.decode_frame(&token).map_err(|e| e.to_string())? {
            if ctx.is_cancelled() {
                cancelled.store(true, Ordering::Release);
                return Err("cancelled".to_owned());
            }
            if frame.timestamp < self.from {
                continue;
            }
            if frame.timestamp >= self.to {
                break;
            }
            if let Some(level) = brightness(&frame) {
                frames.push((frame.timestamp, level));
            }
            if frames.len().is_multiple_of(30) {
                ctx.progress((frame.timestamp - self.from).ticks() as f32 / span as f32);
            }
        }
        FlickerGains::from_brightness(&frames)
            .ok_or_else(|| "the clip is too short to measure flicker in".to_owned())
    }
}

impl Task for FlickerJob {
    fn label(&self) -> String {
        format!("Removing flicker from {}", self.asset.file_name)
    }

    fn priority(&self) -> Priority {
        Priority::UserRequested
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let answer = self.measure(ctx);
        let failed = answer.as_ref().err().cloned();
        self.report.finish(answer);
        ctx.progress(1.0);
        match failed {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}
