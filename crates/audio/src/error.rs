//! Audio errors.
//!
//! A machine with no working sound device must still edit (§50), so every
//! variant here is something the caller can fall back from rather than a
//! reason to stop.

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio output device")]
    NoOutputDevice,

    #[error("audio device configuration is unusable: {0}")]
    DeviceConfig(String),

    #[error("could not build the audio stream: {0}")]
    BuildStream(String),

    #[error("could not start the audio stream: {0}")]
    Play(String),
}
