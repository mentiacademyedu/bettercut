//! Playback errors.
//!
//! §50: a media failure marks the clip unavailable and the session continues.
//! None of these are reasons to stop editing.

use bettercut_foundation::MediaTime;

#[derive(Debug, thiserror::Error)]
pub enum PlaybackError {
    #[error(transparent)]
    Media(#[from] bettercut_media::MediaError),

    #[error("no frame available at {} ticks", at.ticks())]
    NoFrame { at: MediaTime },
}
