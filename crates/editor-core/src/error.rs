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

    #[error(transparent)]
    Captions(#[from] bettercut_captions::CaptionError),

    #[error("no sequence {0} in this project")]
    SequenceNotFound(SequenceId),

    #[error("no track {0} in this sequence")]
    TrackNotFound(TrackId),

    #[error("no clip {0} on that track")]
    ClipNotFound(ClipId),

    #[error("no media {0} in this project")]
    MediaNotFound(bettercut_foundation::MediaId),

    /// §26: every sequence gets a text track, so this only happens to a project
    /// whose tracks were all deleted.
    #[error("this sequence has no text track to put a title on")]
    NoTextTrack,

    /// A video file needs a video track and an audio file needs an audio one.
    /// Reachable only from a sequence whose tracks have all been deleted.
    #[error("this sequence has no track that can hold that media")]
    NoTrackForMedia,

    /// §12: an unlink with nothing to unlink.
    #[error("that clip is not linked to anything")]
    NothingLinked,

    #[error("clip kind does not match track kind")]
    ClipKindMismatch,

    /// Re-timing applies to *footage*. A held frame and a photo are one
    /// picture, so there is nothing to play faster — and deriving their length
    /// from their source range, as a speed change does, would collapse a hold
    /// to a single frame.
    #[error("a held frame and a photo have no motion to re-time")]
    NoMotionToRetime,

    /// A split-screen layout takes one picture clip per cell.
    #[error("this layout needs {wanted} picture clips selected, and {given} are")]
    SplitScreenCount { wanted: usize, given: usize },

    /// A `.cube` file that could not be imported, with the reason.
    #[error("{0}")]
    Lut(String),

    /// A generated animation (a shake) would replace keys the user set by
    /// hand. Refused rather than overwritten: those keys are work that cannot
    /// be guessed back.
    #[error("this clip's {0} is already animated — clear those keyframes first")]
    AlreadyAnimated(&'static str),

    /// A speed ramp needs at least a frame of the clip for each of its pieces;
    /// fewer frames than that and some pieces would be empty.
    #[error("the clip is too short for a {pieces}-step speed ramp")]
    TooShortToRamp { pieces: usize },

    /// A keyframe is always placed at the frame the user is looking at, so
    /// there is nowhere to put one while the playhead is elsewhere (§24).
    #[error("move the playhead over the clip to add a keyframe")]
    PlayheadOffClip,

    /// §25: a crossfade reads material either side of the cut, and a clip
    /// trimmed to the edge of its file has none to read.
    #[error("not enough spare footage either side of the cut for a transition")]
    NoRoomForTransition,

    /// §31: what the user put in a slot does not fit it.
    #[error("{slot}: {reason}")]
    TemplateFill { slot: String, reason: &'static str },

    /// A replacement file without what the clip shows or plays: a picture
    /// clip given a sound file, or a sound clip given a photo.
    #[error("that file has no {0} to put in this clip")]
    ReplacementLacks(&'static str),

    /// A replacement file shorter than the footage the clip uses.
    #[error(
        "that file is too short: this clip uses {needed:.1} s of footage and it has {available:.1} s"
    )]
    ReplacementTooShort { needed: f64, available: f64 },

    /// A track renamed to nothing.
    #[error("a track needs a name")]
    EmptyTrackName,

    /// A track put in with the id of one already there.
    #[error("track {0:?} is already in the sequence")]
    TrackAlreadyExists(TrackId),

    /// Grouping asked for with fewer than two things to group.
    #[error("select at least two clips to group")]
    NothingToGroup,

    /// A swap asked for with nothing on that side of the clip.
    #[error("there is no clip on that side to swap with")]
    NoNeighbour,

    /// A swap that would move a linked partner onto another clip.
    #[error("the clips' linked sound has no room to trade places, so nothing moved")]
    NoRoomToSwap,

    /// Closing a gap, with no gap where it was asked for.
    #[error("there is no gap there to close")]
    NoGapThere,

    /// Closing the gap would pull a clip's linked sound or picture into
    /// another clip, or onto a locked track — so it is left open.
    #[error("a linked clip has no room to move left, so the gap stays open")]
    GapBlocked,

    /// A template's tracks are occupied where it wants to go.
    #[error("there are clips in the way where the template would go")]
    NoRoomForTemplate,

    /// §33's slideshow, asked for with no pictures. Refused rather than
    /// quietly making an empty one: an undo step that changed nothing is worse
    /// than being told the selection was empty.
    #[error("select some pictures to make a slideshow from")]
    NoPicturesForSlideshow,

    #[error("nothing to undo")]
    NothingToUndo,

    #[error("nothing to redo")]
    NothingToRedo,

    #[error("project has never been saved — use Save As")]
    NoProjectPath,

    /// Save a Copy aimed at the project's own file.
    #[error(
        "that is this project's own file — use Save to save it, or pick another name for the copy"
    )]
    CopyOverOriginal,

    /// A command was undone without having been executed. A bug in the history,
    /// not in user input.
    #[error("command was undone before it was executed")]
    NotExecuted,
}
