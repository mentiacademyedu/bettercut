//! Export as a background job (§15, §42, §74).
//!
//! §74 is blunt about this: *never block the UI while FFmpeg runs.* An export
//! is the longest FFmpeg run the program ever does, so it goes on the same job
//! scheduler as proxies and thumbnails — §15's concurrency limit is one budget
//! for the machine, not one per feature.
//!
//! ## The project is snapshotted, not borrowed
//!
//! The job owns a clone of the project taken when the user pressed Export.
//! Editing while an export runs is normal — it is the obvious thing to do with
//! the minutes it takes — and a job reading live state would render half the
//! frames from before an edit and half from after. A `Project` is references
//! and numbers, never media, so the copy costs almost nothing (§2).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bettercut_foundation::SequenceId;
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::CancellationToken;
use bettercut_project_format::Project;

use crate::{ExportSettings, export};

/// One export, queued on the scheduler.
pub struct ExportJob {
    project: Project,
    sequence: SequenceId,
    settings: ExportSettings,
    /// Where the finished summary goes, for the interface to report.
    outcome: Arc<std::sync::Mutex<Option<Outcome>>>,
}

/// What became of an export, readable once the job has finished.
#[derive(Debug, Clone)]
pub enum Outcome {
    Finished {
        path: std::path::PathBuf,
        frames: u64,
        encoder: &'static str,
        hardware: bool,
    },
    Cancelled,
    Failed(String),
}

impl ExportJob {
    pub fn new(project: &Project, sequence: SequenceId, settings: ExportSettings) -> Self {
        Self {
            project: project.clone(),
            sequence,
            settings,
            outcome: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// What the status bar should say while this is queued. The scheduler
    /// reports the same string, but the shell wants it before the job starts.
    pub fn label_for_status(&self) -> String {
        Task::label(self)
    }

    /// A handle to read the outcome from the UI thread once the scheduler
    /// reports the job finished.
    pub fn outcome(&self) -> Arc<std::sync::Mutex<Option<Outcome>>> {
        Arc::clone(&self.outcome)
    }
}

/// Bridges the scheduler's cancellation to the media crate's token.
///
/// Two traits for the same idea, in two crates that must not depend on each
/// other — §86's isolation rule costs one small adapter here, which is the
/// right trade against `jobs` knowing what FFmpeg is.
struct JobCancellation {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken for JobCancellation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl Task for ExportJob {
    fn label(&self) -> String {
        let name = self
            .settings
            .path
            .file_name()
            .map_or_else(|| "video".to_owned(), |n| n.to_string_lossy().into_owned());
        format!("Exporting {name}")
    }

    fn priority(&self) -> Priority {
        // §69 level 3: ahead of proxies and thumbnails, behind anything the
        // user is waiting on right now. An export is deliberate and long, so it
        // should not be starved — but it should not stall a scrub either.
        Priority::Export
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let Some(sequence) = self.project.sequence(self.sequence) else {
            return Err("that sequence is no longer in the project".to_owned());
        };

        let cancelled = Arc::new(AtomicBool::new(false));
        let token = JobCancellation {
            cancelled: Arc::clone(&cancelled),
        };

        // The export loop polls its own token; keep it in step with the
        // scheduler's on every progress callback, which fires per frame.
        let mut on_progress = |progress: crate::ExportProgress| {
            if ctx.is_cancelled() {
                cancelled.store(true, Ordering::Release);
            }
            ctx.progress(progress.fraction());
        };

        let result = export(
            &self.project,
            sequence,
            &self.settings,
            &mut on_progress,
            &token,
        );

        let (outcome, returned) = match result {
            Ok(summary) => (
                Outcome::Finished {
                    path: summary.path,
                    frames: summary.frames,
                    encoder: summary.encoder,
                    hardware: summary.hardware,
                },
                Ok(()),
            ),
            // §48/§50: a cancellation is expected control flow, not a failure
            // to report. The partial file is already gone.
            Err(err) if err.is_cancellation() => (Outcome::Cancelled, Ok(())),
            Err(err) => {
                let message = err.to_string();
                (Outcome::Failed(message.clone()), Err(message))
            }
        };

        if let Ok(mut slot) = self.outcome.lock() {
            *slot = Some(outcome);
        }
        returned
    }
}
