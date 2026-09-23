//! Colour match as a background job: a clip's grade chosen to look like the
//! frame under the playhead.
//!
//! Two frames are needed. The **reference** is what the sequence shows at the
//! playhead, rendered the way a still is — but with the clip being matched
//! taken out, so a playhead resting on that clip matches it to whatever lies
//! beneath rather than to itself. The **clip's own frame** is decoded straight
//! from its file, from the middle of the clip, before any grade: the match
//! replaces the grade rather than stacking on it. The maths is
//! [`bettercut_timeline::colour_match`].

use std::sync::{Arc, Mutex};

use bettercut_foundation::{ClipId, SequenceId, TimelineTime};
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::{CancellationToken, FrameStorage, SeekMode};
use bettercut_playback::FrameSource;
use bettercut_project_format::Project;
use bettercut_timeline::ColorAdjust;
use bettercut_timeline::colour_match::{FrameSample, auto_grade, match_grade};

use crate::render_still;

/// A finished match waiting to be collected: the clip and its new grade.
pub type MatchedGrade = Arc<Mutex<Option<(ClipId, ColorAdjust, &'static str)>>>;

/// Less than this average luma is a reference with nothing in it — an empty
/// stretch of timeline renders black — and matching to black would only
/// crush the clip.
const EMPTY_REFERENCE: f32 = 0.002;

/// Match `clip`'s grade to the frame at `reference_at`.
pub struct ColourMatchJob {
    project: Project,
    sequence: SequenceId,
    clip: ClipId,
    /// The frame to match, or `None` for a neutral target — auto level.
    reference_at: Option<TimelineTime>,
    slot: MatchedGrade,
}

impl ColourMatchJob {
    pub fn new(
        project: Project,
        sequence: SequenceId,
        clip: ClipId,
        reference_at: TimelineTime,
    ) -> Self {
        Self {
            project,
            sequence,
            clip,
            reference_at: Some(reference_at),
            slot: MatchedGrade::default(),
        }
    }

    /// Grade `clip` to a neutral exposure and balance instead of to a frame
    /// (`bettercut_timeline::colour_match::auto_grade`).
    pub fn auto_level(project: Project, sequence: SequenceId, clip: ClipId) -> Self {
        Self {
            project,
            sequence,
            clip,
            reference_at: None,
            slot: MatchedGrade::default(),
        }
    }

    /// What the grade is for, as the undo step and the status line say it.
    pub fn label_for_grade(&self) -> &'static str {
        if self.reference_at.is_some() {
            "Match Colour"
        } else {
            "Auto Level"
        }
    }

    /// Where the grade will be once the job has finished.
    pub fn slot(&self) -> MatchedGrade {
        Arc::clone(&self.slot)
    }

    pub fn clip(&self) -> ClipId {
        self.clip
    }
}

struct JobCancel<'a>(&'a JobContext);

impl CancellationToken for JobCancel<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

/// Tightly packed RGBA rows from a decoded frame whose rows may be padded.
fn packed_rows(data: &[u8], stride: usize, width: u32, height: u32) -> Vec<u8> {
    let (width, height) = (width as usize, height as usize);
    if stride == width * 4 {
        return data[..(width * height * 4).min(data.len())].to_vec();
    }
    let mut rows = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let start = y * stride;
        if let Some(row) = data.get(start..start + width * 4) {
            rows.extend_from_slice(row);
        }
    }
    rows
}

/// The grade that makes `clip` look like the frame at `reference_at`, or why
/// there is none. Split from the job so a test can drive it without the
/// scheduler.
pub fn colour_match(
    project: &Project,
    sequence: SequenceId,
    clip: ClipId,
    reference_at: TimelineTime,
    cancel: &dyn CancellationToken,
) -> Result<ColorAdjust, String> {
    let Some(original) = project.sequence(sequence) else {
        return Err("that sequence is no longer in the project".to_owned());
    };
    let Some(track) = original
        .video_tracks
        .iter()
        .find(|t| t.get(clip).is_some())
        .map(|t| t.id)
    else {
        return Err("only a picture clip can be colour matched".to_owned());
    };

    // The reference, without the clip being matched.
    let mut without = original.clone();
    if let Some(t) = without.video_track_mut(track) {
        let _ = t.remove(clip);
    }
    let (size, rgba) =
        render_still(project, &without, reference_at, cancel).map_err(|e| e.to_string())?;
    let reference = FrameSample::from_rgba(&rgba, size.width, size.height)
        .ok_or("the frame under the playhead could not be read")?
        .stats();
    if reference.luma < EMPTY_REFERENCE {
        return Err("there is no picture under the playhead to match to".to_owned());
    }

    let sample = clip_sample(project, sequence, clip, cancel)?;
    Ok(match_grade(&sample, &reference))
}

/// The grade that brings `clip` to a neutral exposure and balance, from its
/// own middle frame — no reference, no render, so no GPU either.
pub fn auto_level(
    project: &Project,
    sequence: SequenceId,
    clip: ClipId,
    cancel: &dyn CancellationToken,
) -> Result<ColorAdjust, String> {
    let sample = clip_sample(project, sequence, clip, cancel)?;
    Ok(auto_grade(&sample))
}

/// The clip's own frame, from its middle, ungraded, sampled.
fn clip_sample(
    project: &Project,
    sequence: SequenceId,
    clip: ClipId,
    cancel: &dyn CancellationToken,
) -> Result<FrameSample, String> {
    let Some(original) = project.sequence(sequence) else {
        return Err("that sequence is no longer in the project".to_owned());
    };
    let Some(video) = original.video_tracks.iter().find_map(|t| t.get(clip)) else {
        return Err("only a picture clip can be graded this way".to_owned());
    };
    let Some(asset) = project.media_asset(video.media_id) else {
        return Err("the clip's file is not in the project".to_owned());
    };
    let middle =
        TimelineTime::from_ticks((video.timeline.start.ticks() + video.timeline.end.ticks()) / 2);
    let mut frames = FrameSource::new(2);
    let frame = frames
        .decode(
            asset,
            video.source_time_at(middle),
            SeekMode::Precise,
            cancel,
        )
        .map_err(|e| e.to_string())?;
    let FrameStorage::System { data, stride } = &frame.storage else {
        return Err("the clip's frame is not in system memory".to_owned());
    };
    let rows = packed_rows(data, *stride as usize, frame.width, frame.height);
    FrameSample::from_rgba(&rows, frame.width, frame.height)
        .ok_or_else(|| "the clip's frame could not be read".to_owned())
}

impl Task for ColourMatchJob {
    fn label(&self) -> String {
        if self.reference_at.is_some() {
            "Matching colour".to_owned()
        } else {
            "Setting levels".to_owned()
        }
    }

    fn priority(&self) -> Priority {
        Priority::Export
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let cancel = JobCancel(ctx);
        let grade = match self.reference_at {
            Some(at) => colour_match(&self.project, self.sequence, self.clip, at, &cancel)?,
            None => auto_level(&self.project, self.sequence, self.clip, &cancel)?,
        };
        if let Ok(mut slot) = self.slot.lock() {
            *slot = Some((self.clip, grade, self.label_for_grade()));
        }
        Ok(())
    }
}
