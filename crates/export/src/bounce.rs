//! Bouncing one lane to a file: every clip on it, with its effects, its fades,
//! its envelope and the lane's own level, mixed down to one piece of sound.
//!
//! # Why it is the export's mixer and not a second one
//!
//! A bounce that did not sound exactly like the lane it replaced would be a
//! bug the user only hears in the finished film. So this is
//! [`crate::export_sound`], given a copy of the project with every *other*
//! sound lane silenced — the same mixer, the same clip effects, the same
//! automation, the same §46 rule the preview and the export already share.
//!
//! # What the copy changes
//!
//! Only two things, and both because they are applied again later: the master
//! volume is set back to unity, and every other sound lane is turned off. The
//! lane's own gain, pan and volume line are *kept*, because they are what the
//! lane sounds like — the editor resets them when it puts the bounced clip
//! down, so they are applied once, here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bettercut_foundation::{SequenceId, TrackId};
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::CancellationToken;
use bettercut_project_format::Project;
use bettercut_timeline::TimelineRange;

use crate::{ExportError, ExportProgress, ExportSettings, export_sound};

/// The stretch of the timeline a lane covers: its first clip to its last.
///
/// `None` for a lane with nothing on it, which has nothing to bounce.
pub fn track_span(
    project: &Project,
    sequence: SequenceId,
    track: TrackId,
) -> Option<TimelineRange> {
    let sequence = project.sequence(sequence)?;
    let lane = sequence.audio_track(track)?;
    let first = lane.clips().first()?.timeline.start;
    let last = lane.clips().last()?.timeline.end;
    TimelineRange::new(first, last).ok()
}

/// The project as the bounce should hear it: this lane alone, at unity master.
pub fn only_this_lane(
    mut project: Project,
    sequence: SequenceId,
    track: TrackId,
) -> Option<Project> {
    let active = project.sequence_mut(sequence)?;
    active.master_volume = 1.0;
    let mut found = false;
    for lane in &mut active.audio_tracks {
        if lane.id == track {
            // Solo elsewhere in the sequence would otherwise silence the very
            // lane being bounced.
            lane.enabled = true;
            lane.solo = false;
            found = true;
        } else {
            lane.enabled = false;
            lane.solo = false;
        }
    }
    found.then_some(project)
}

/// One lane bounced to a WAV, on the job scheduler (§74: never block the UI
/// while a decoder runs).
pub struct BounceJob {
    project: Project,
    sequence: SequenceId,
    track: TrackId,
    range: TimelineRange,
    path: PathBuf,
    done: Arc<std::sync::Mutex<Option<Result<PathBuf, String>>>>,
}

impl BounceJob {
    /// `project` is taken whole and silenced down to `track`, as an export
    /// takes its own copy: editing on while the bounce runs must not change
    /// what was bounced.
    pub fn new(
        project: &Project,
        sequence: SequenceId,
        track: TrackId,
        path: PathBuf,
    ) -> Option<Self> {
        let range = track_span(project, sequence, track)?;
        let project = only_this_lane(project.clone(), sequence, track)?;
        Some(Self {
            project,
            sequence,
            track,
            range,
            path,
            done: Arc::default(),
        })
    }

    /// Which lane this is bouncing, and over what stretch.
    pub fn track(&self) -> TrackId {
        self.track
    }

    pub fn range(&self) -> TimelineRange {
        self.range
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where the answer lands once the scheduler reports the job finished.
    pub fn outcome(&self) -> Arc<std::sync::Mutex<Option<Result<PathBuf, String>>>> {
        Arc::clone(&self.done)
    }
}

impl Task for BounceJob {
    fn label(&self) -> String {
        format!(
            "Bouncing a sound track ({})",
            self.range.duration().format_timecode()
        )
    }

    fn priority(&self) -> Priority {
        Priority::Export
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let Some(sequence) = self.project.sequence(self.sequence) else {
            return Err("that sequence is no longer in the project".to_owned());
        };
        let mut settings = ExportSettings::for_sequence(self.path.clone(), sequence);
        settings.sound_only = true;
        settings.range = self.range;

        let cancel = JobCancel(ctx);
        let mut progress = |progress: ExportProgress| {
            ctx.progress(progress.fraction());
        };
        let outcome = match export_sound(&self.project, sequence, &settings, &mut progress, &cancel)
        {
            Ok(summary) => Ok(summary.path),
            Err(ExportError::Cancelled) => return Ok(()),
            Err(err) => Err(err.to_string()),
        };
        let failed = outcome.as_ref().err().cloned();
        if let Ok(mut slot) = self.done.lock() {
            *slot = Some(outcome);
        }
        match failed {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

/// The scheduler's cancellation, as the media crate asks for it.
struct JobCancel<'a>(&'a JobContext);

impl CancellationToken for JobCancel<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}
