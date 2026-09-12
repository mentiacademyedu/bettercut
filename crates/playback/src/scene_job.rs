//! Running [`crate::scenes`] over a file, off the interface thread.
//!
//! Detection has to decode the footage, which is the one thing that must never
//! happen while the user is holding a mouse button. So it is a job: submitted
//! like a proxy or a filmstrip, cancellable at every frame (§48), and it leaves
//! its answer in a [`SceneReport`] the interface reads when the job finishes.
//!
//! The answer is *source* instants. Turning them into timeline instants is the
//! caller's job, because that mapping goes through the clip's speed (§51) and
//! this does not know which clip asked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bettercut_foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::{FfmpegDecoder, MediaAsset, MediaDecoder, MediaKind, SeekMode};

use crate::scenes::{FrameDigest, SceneSettings, digest, scene_cuts};

/// Where a file's cuts fall on the timeline, for one clip that plays it.
///
/// The same translation [`crate::beat_markers`] does for sound: from an instant
/// in the file to an instant on the timeline, through the clip's speed (§51) —
/// so a cut in footage slowed to half speed is offered at the place it is
/// actually seen, not at the place it was shot.
///
/// Only cuts strictly inside the clip come back. A clip trimmed to one shot
/// contains none of the file's cuts, and offering the user a split on a clip
/// edge would make a zero-length piece.
pub fn timeline_cuts(
    clip: &bettercut_timeline::VideoClip,
    cuts: &[MediaTime],
) -> Vec<TimelineTime> {
    // A held frame shows one instant for its whole length (§51's
    // `NoMotionToRetime` case): there is no cut inside one picture.
    if clip.frozen {
        return Vec::new();
    }
    cuts.iter()
        .filter(|at| **at > clip.source.start && **at < clip.source.end)
        .map(|at| {
            clip.timeline.start
                + TimelineTime::from_ticks(bettercut_timeline::timeline_ticks_for(
                    *at - clip.source.start,
                    clip.speed,
                ))
        })
        .filter(|at| *at > clip.timeline.start && *at < clip.timeline.end)
        .collect()
}

struct JobCancellation {
    cancelled: Arc<AtomicBool>,
}

impl bettercut_media::CancellationToken for JobCancellation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Where a finished detection leaves its cuts.
///
/// Cloneable and shared with the running job: the interface keeps one, hands
/// the other to the scheduler, and reads it when the job reports finished.
#[derive(Debug, Clone, Default)]
pub struct SceneReport(Arc<Mutex<Option<Vec<MediaTime>>>>);

impl SceneReport {
    /// The cuts, or `None` while the job is still running.
    ///
    /// A file with no cuts in it answers `Some(vec![])` — "there are none" and
    /// "not finished" are different answers and the interface says different
    /// things about them.
    pub fn cuts(&self) -> Option<Vec<MediaTime>> {
        self.0.lock().ok()?.clone()
    }

    /// Record the answer. Called by the job when it has one — and by tests,
    /// which is why it is public: a window that waits for a worker can only be
    /// tested by something that can play the worker's part.
    pub fn finish(&self, cuts: Vec<MediaTime>) {
        if let Ok(mut held) = self.0.lock() {
            *held = Some(cuts);
        }
    }
}

/// Find the cuts in one stretch of one file.
pub struct SceneJob {
    asset: MediaAsset,
    from: MediaTime,
    to: MediaTime,
    settings: SceneSettings,
    /// Digest every nth decoded frame. Decoding is the cost, not the digest,
    /// so this is about keeping the comparison a sensible distance apart on a
    /// high frame rate file rather than about speed.
    every: usize,
    threads: u32,
    report: SceneReport,
}

impl SceneJob {
    /// Prepare a job over `[from, to)` of `asset`, or `None` when there is
    /// nothing to look at.
    pub fn new(
        asset: &MediaAsset,
        from: MediaTime,
        to: MediaTime,
        settings: SceneSettings,
        threads: u32,
    ) -> Option<(Self, SceneReport)> {
        // A still image is one picture and audio has none: neither can cut.
        if asset.kind != MediaKind::Video || asset.missing || to <= from {
            return None;
        }
        let report = SceneReport::default();
        Some((
            Self {
                asset: asset.clone(),
                from,
                to,
                settings,
                every: usize::from(asset.frame_rate.is_some_and(|rate| rate.as_f64() > 40.0)) + 1,
                threads: threads.max(1),
                report: report.clone(),
            },
            report,
        ))
    }

    pub fn media(&self) -> MediaId {
        self.asset.id
    }
}

impl Task for SceneJob {
    fn label(&self) -> String {
        format!("Finding cuts in {}", self.asset.file_name)
    }

    fn priority(&self) -> Priority {
        // The user asked for this and is waiting for the answer, so it outranks
        // the proxies and filmstrips building themselves in the background.
        Priority::UserRequested
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = JobCancellation {
            cancelled: Arc::clone(&cancelled),
        };

        let mut decoder = FfmpegDecoder::new(self.threads).map_err(|e| e.to_string())?;
        decoder.open(&self.asset).map_err(|e| e.to_string())?;
        // `Precise` rather than `Scrub`: starting at the keyframe before the
        // range would compare frames from before it and could report a cut
        // that is not inside what was asked about.
        if !self.from.is_zero() {
            decoder
                .seek(self.from, SeekMode::Precise)
                .map_err(|e| e.to_string())?;
        }

        let span = (self.to - self.from).ticks().max(1);
        let mut frames: Vec<(MediaTime, FrameDigest)> = Vec::new();
        let mut decoded = 0_usize;
        while let Some(frame) = decoder.decode_frame(&token).map_err(|e| e.to_string())? {
            if ctx.is_cancelled() {
                cancelled.store(true, Ordering::Release);
                return Err("cancelled".to_owned());
            }
            if frame.timestamp < self.from {
                continue; // a seek can land early; those frames are not ours
            }
            if frame.timestamp >= self.to {
                break;
            }
            if decoded.is_multiple_of(self.every)
                && let Some(digest) = digest(&frame)
            {
                frames.push((frame.timestamp, digest));
            }
            decoded += 1;
            if decoded.is_multiple_of(30) {
                ctx.progress((frame.timestamp - self.from).ticks() as f32 / span as f32);
            }
        }

        let cuts = scene_cuts(&frames, self.settings);
        tracing::debug!(
            file = %self.asset.file_name,
            frames = frames.len(),
            cuts = cuts.len(),
            "scene detection finished"
        );
        self.report.finish(cuts);
        ctx.progress(1.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::{MediaId, Rational};
    use bettercut_timeline::{SourceRange, VideoClip};

    /// A clip playing `[from, to)` of a file, starting ten seconds in.
    fn clip(from: i64, to: i64) -> VideoClip {
        VideoClip::new(
            MediaId::new(),
            TimelineTime::from_seconds(10),
            SourceRange::new(MediaTime::from_seconds(from), MediaTime::from_seconds(to)).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn a_cut_lands_where_it_is_seen() {
        let clip = clip(0, 20);
        assert_eq!(
            timeline_cuts(&clip, &[MediaTime::from_seconds(3)]),
            vec![TimelineTime::from_seconds(13)]
        );
    }

    /// §51: at half speed the footage takes twice as long to reach the cut, so
    /// the offered split is twice as far into the clip.
    #[test]
    fn speed_moves_the_cut() {
        let mut clip = clip(0, 20);
        clip.speed = Rational::new(1, 2).unwrap();
        clip.timeline = bettercut_timeline::TimelineRange::new(
            TimelineTime::from_seconds(10),
            TimelineTime::from_seconds(50),
        )
        .unwrap();
        assert_eq!(
            timeline_cuts(&clip, &[MediaTime::from_seconds(3)]),
            vec![TimelineTime::from_seconds(16)]
        );
    }

    /// A clip trimmed to the middle of one shot contains none of the file's
    /// cuts, however many the file has.
    #[test]
    fn cuts_outside_the_trimmed_range_are_not_offered() {
        let clip = clip(30, 40);
        let cuts = [
            MediaTime::from_seconds(5),
            MediaTime::from_seconds(30), // its own in-point
            MediaTime::from_seconds(40), // its own out-point
            MediaTime::from_seconds(55),
        ];
        assert!(timeline_cuts(&clip, &cuts).is_empty());
    }

    #[test]
    fn a_trimmed_clip_measures_from_its_own_start() {
        let clip = clip(30, 40);
        assert_eq!(
            timeline_cuts(&clip, &[MediaTime::from_seconds(34)]),
            vec![TimelineTime::from_seconds(14)]
        );
    }

    #[test]
    fn a_held_frame_has_no_cuts_in_it() {
        let mut clip = clip(0, 20);
        clip.frozen = true;
        assert!(
            timeline_cuts(&clip, &[MediaTime::from_seconds(3)]).is_empty(),
            "a cut was offered inside a single held picture"
        );
    }
}
