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

    #[error("no media {0} in this project")]
    MediaNotFound(bettercut_foundation::MediaId),

    #[error("clip kind does not match track kind")]
    ClipKindMismatch,

    /// A keyframe is always placed at the frame the user is looking at, so
    /// there is nowhere to put one while the playhead is elsewhere (§24).
    #[error("move the playhead over the clip to add a keyframe")]
    PlayheadOffClip,

    /// §25: a crossfade reads material either side of the cut, and a clip
    /// trimmed to the edge of its file has none to read.
    #[error("not enough spare footage either side of the cut for a transition")]
    NoRoomForTransition,

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
