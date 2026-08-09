//! Timeline errors (§50: a bad edit is an error value, never a panic).

use bettercut_foundation::{ClipId, MediaTime, SequenceId, TimelineTime, TrackId};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TimelineError {
    #[error("timeline range is empty or inverted: {start:?}..{end:?}")]
    EmptyRange {
        start: TimelineTime,
        end: TimelineTime,
    },

    #[error("source range is empty or inverted: {start:?}..{end:?}")]
    EmptySourceRange { start: MediaTime, end: MediaTime },

    #[error("clip {new} at {start:?}..{end:?} overlaps existing clip {existing} on track {track}")]
    ClipOverlap {
        track: TrackId,
        new: ClipId,
        existing: ClipId,
        start: TimelineTime,
        end: TimelineTime,
    },

    #[error("no track {0} in this sequence")]
    TrackNotFound(TrackId),

    #[error("no clip {0} on this track")]
    ClipNotFound(ClipId),

    #[error("no sequence {0} in this project")]
    SequenceNotFound(SequenceId),

    #[error("track {0} is locked")]
    TrackLocked(TrackId),

    #[error("frame rate {rate} does not divide the 960,000 tick timebase exactly")]
    UnrepresentableFrameRate { rate: String },

    #[error("timeline positions cannot be negative: {at:?}")]
    NegativePosition { at: TimelineTime },

    #[error("clip {clip} cannot extend {by_ticks} ticks before the start of its media")]
    BeyondSourceStart { clip: ClipId, by_ticks: i64 },

    #[error("clip {clip} cannot extend {by_ticks} ticks past the end of its media")]
    BeyondSourceEnd { clip: ClipId, by_ticks: i64 },

    #[error("cannot split clip {clip} at {at:?}: it spans {start:?}..{end:?}")]
    SplitOutsideClip {
        clip: ClipId,
        at: TimelineTime,
        start: TimelineTime,
        end: TimelineTime,
    },

    #[error("a clip must be at least one frame long")]
    ShorterThanOneFrame,

    #[error("nothing on the clipboard")]
    ClipboardEmpty,
}
