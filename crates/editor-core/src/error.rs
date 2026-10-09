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

    /// A key moved on a clip whose position is not keyed: there is no path
    /// for it to be a point of.
    #[error("that clip's position is not keyed, so there is no path to move")]
    NotAnimated,

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

    /// A voice effect was asked of clips with no sound in or linked to them.
    #[error("select a sound clip, or a video with sound, for a voice effect")]
    NoSoundToChange,

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

    /// A key was dragged or deleted that is no longer where it was — the clip
    /// changed underneath the graph.
    #[error("there is no keyframe there any more")]
    NoKeyframeThere,

    /// A compound needs clips to fold in; titles stay where they are, so a
    /// selection of nothing but titles asks for a compound of nothing.
    #[error("select the picture or sound clips to fold into a compound clip")]
    NothingToCompound,

    /// Break Apart on a clip that is not a compound.
    #[error("that clip is not a compound clip")]
    NotACompound,

    /// Break Apart on a compound that was trimmed or re-timed: its inside
    /// would have to be cut to fit, which nobody asked for.
    #[error("this compound was trimmed or re-timed; open it instead, or undo the trim first")]
    CompoundTrimmed,

    /// Lining two recordings up would put one of them before the start of the
    /// timeline. Moving the other one is what fixes it.
    #[error("these line up before the start of the timeline — move the other clip later first")]
    NoRoomToSync,

    /// A track that found nothing to write: the two clips do not overlap where
    /// the playhead is, or the thing followed left the frame at once.
    #[error("nothing to follow here — put the playhead where both clips are playing")]
    NothingTracked,

    /// A loudness match asked for on a mix with nothing audible in it.
    #[error("there is nothing to hear in this edit yet")]
    NothingToHear,

    /// A sticker with no character in it.
    #[error("pick a sticker to add")]
    NoSticker,

    /// Steadying this shot would mean cropping more of it away than the shake
    /// costs. Refused rather than done quietly.
    #[error("this shot moves too much to steady without cropping most of it away")]
    TooShakyToSteady,

    /// A multicam needs cameras: two clips at least, and a lane each to put
    /// them on.
    #[error("a multicam clip needs at least two picture clips, and a lane for each")]
    NotEnoughAngles,

    /// An angle asked of a clip that is not a multicam.
    #[error("that clip is not a multicam clip")]
    NotMulticam,

    #[error("this multicam has {count} angle(s), so there is no angle {angle}")]
    NoSuchAngle { angle: usize, count: usize },

    /// A slip plays a different stretch of a file; a photo, a colour or a
    /// held frame is one picture, so there is no other stretch to play.
    #[error("only video and sound clips can be slipped — this one has no footage either side")]
    NothingToSlip,

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

    /// Removing the only sequence a project has.
    #[error("a project needs at least one sequence")]
    LastSequence,

    /// A sequence put in with the id of one already there.
    #[error("sequence {0:?} is already in the project")]
    SequenceAlreadyExists(bettercut_foundation::SequenceId),

    /// A track renamed to nothing.
    #[error("a track needs a name")]
    EmptyTrackName,

    /// A sequence renamed to nothing at all.
    #[error("give the sequence a name")]
    EmptySequenceName,

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

    /// Saving as a template, refused with the reason.
    #[error("could not save as a template: {0}")]
    TemplateNotSaved(String),

    /// A lower third with no name to show.
    #[error("type a name for the lower third")]
    LowerThirdNeedsName,

    /// Photos to the beat with fewer than two usable markers.
    #[error("mark the music's beats first — right-click a sound clip and choose Mark Beats")]
    NotEnoughBeats,

    /// Photos to the beat with no photos to place.
    #[error("select some photos to cut to the beat")]
    NothingToCutToBeats,

    /// A shape crop on a file whose picture size was never learned.
    #[error("this clip's picture size is not known, so it cannot be cropped to a shape")]
    UnknownPictureSize,

    /// A voiceover with no sound in it.
    #[error("nothing was recorded")]
    NothingRecorded,

    /// A loop of fewer than two plays, or more than the editor makes.
    #[error("a loop plays a clip between 2 and 20 times")]
    LoopCountOutOfRange,

    /// A shuffle asked of fewer than two clips side by side on one lane.
    #[error("select two or more clips next to each other on one lane to shuffle")]
    NothingToShuffle,

    /// A shuffle that would move linked sound onto another clip.
    #[error("the clips' linked sound has no room to be reordered, so nothing moved")]
    NoRoomToShuffle,

    /// Fit to fill asked for a speed past the speed control's limits, or an
    /// end that is not after the clip's start.
    #[error("the clip would have to play slower than 0.1× or faster than 10× to fit there")]
    FillOutOfRange,

    /// Closing a gap, with no gap where it was asked for.
    #[error("there is no gap there to close")]
    NoGapThere,

    /// Closing the gap would pull a clip's linked sound or picture into
    /// another clip, or onto a locked track — so it is left open.
    #[error("a linked clip has no room to move left, so the gap stays open")]
    GapBlocked,

    /// A lane riding along with a ripple edit (§10's sync lock) has a clip
    /// the edit runs through, or nowhere for one to land.
    #[error(
        "a sync-locked track has a clip across the edit — cut it there, or take its sync lock off"
    )]
    SyncBlocked,

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

    /// Split at Markers, or writing the markers out, with no markers.
    #[error("there are no markers; press M to add one")]
    NoMarkers,

    /// A colour change asked of a file rather than a colour clip.
    #[error("that clip is not a colour clip")]
    NotAColourClip,

    /// Lift or extract with no in and out marks set.
    #[error("set in and out marks around the part to take out first (I and O)")]
    NoMarkedRange,

    /// An extract would pull the edit past a locked track.
    #[error("a locked track has clips after the marks; unlock it to close the gap, or use Lift")]
    LockedTrackInTheWay,

    /// Fit Music on a shot's own sound.
    #[error("this sound belongs to a shot — fit a music clip instead")]
    MusicHasPicture,

    /// Fit Music with no pictures to fit to, or a song after all of them.
    #[error("there are no pictures for the music to end with")]
    NothingToFitTo,

    /// A split into more pieces than the clip has room, or pieces too short.
    #[error("that would cut the clip into too many pieces, or pieces too short to keep")]
    SplitTooFine,

    /// A file could not be read or written, with the reason.
    #[error("could not read or write a file: {0}")]
    Io(String),

    /// Save a Copy aimed at the project's own file.
    #[error(
        "that is this project's own file — use Save to save it, or pick another name for the copy"
    )]
    CopyOverOriginal,

    /// Extending a clip into the gap after it, with no gap after it.
    #[error(
        "nothing to fill: the next clip on this lane starts right where this one ends, or there is none"
    )]
    NothingToExtendTo,
    /// Extending a clip whose footage ends where it does.
    #[error("the clip has no more footage after its end to fill the gap with")]
    NoFootageToExtend,
    /// A marker grid so fine the ruler would be solid markers.
    #[error(
        "that would be more than a thousand markers — pick a longer gap or mark a shorter range"
    )]
    TooManyMarkers,
    /// Merging a lane into one whose clips it would land on.
    #[error("the lane below has clips where this one does, so they cannot share it")]
    LanesOverlap,
    /// A command was undone without having been executed. A bug in the history,
    /// not in user input.
    #[error("command was undone before it was executed")]
    NotExecuted,
}
