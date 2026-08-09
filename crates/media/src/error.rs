//! Media errors.
//!
//! §74: "Silently ignore FFmpeg failures" is on the prohibited list. Every
//! variant here names what failed and where, so a swallowed error is visible in
//! review as a discarded `Result` rather than an empty log line.

use std::path::PathBuf;

use bettercut_foundation::MediaId;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media file not found: {0}")]
    FileNotFound(PathBuf),

    #[error("unsupported or unrecognized format: {0}")]
    UnsupportedFormat(PathBuf),

    #[error("no decodable stream in {0}")]
    NoStream(PathBuf),

    #[error("decoder is not open")]
    NotOpen,

    #[error("media {0} has no decoder")]
    NoDecoder(MediaId),

    #[error("seek to {timestamp} failed: {reason}")]
    SeekFailed { timestamp: String, reason: String },

    #[error("decode failed: {0}")]
    DecodeFailed(String),

    /// Not an error condition — a cancelled job (§48) unwinding cleanly.
    #[error("operation cancelled")]
    Cancelled,

    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl MediaError {
    /// A cancellation is expected control flow, not a failure to report to the
    /// user or count against §50's "mark clip unavailable" behaviour.
    pub fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}
