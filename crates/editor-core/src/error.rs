//! Editor errors.

use bettercut_foundation::{ClipId, SequenceId, TrackId};
use bettercut_media::MediaError;
use bettercut_project_format::ProjectError;
use bettercut_timeline::TimelineError;

#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    #[error(transparent)]
    Timeline(#[from] TimelineError),

    #[error(transparent)]
    Project(#[from] ProjectError),

    #[error(transparent)]
    Media(#[from] MediaError),

    #[error("no sequence {0} in this project")]
    SequenceNotFound(SequenceId),

    #[error("no track {0} in this sequence")]
    TrackNotFound(TrackId),

    #[error("no clip {0} on that track")]
    ClipNotFound(ClipId),

    #[error("clip kind does not match track kind")]
    ClipKindMismatch,

    #[error("nothing to undo")]
    NothingToUndo,

    #[error("nothing to redo")]
    NothingToRedo,

    #[error("project has never been saved — use Save As")]
    NoProjectPath,

    /// A command was undone without having been executed. A bug in the history,
    /// not in user input.
    #[error("command was undone before it was executed")]
    NotExecuted,
}
