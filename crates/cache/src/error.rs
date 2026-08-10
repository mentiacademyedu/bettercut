//! Cache errors.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A deletion was aimed outside the cache.
    ///
    /// §2 and §74: source media is never modified or removed. Reaching this
    /// error means a bug was caught before it destroyed something the user
    /// cannot get back.
    #[error("refusing to delete {path}: it is outside the cache root")]
    OutsideCacheRoot { path: PathBuf },

    /// A cached thumbnail is not one, or does not match its own header.
    ///
    /// Always recoverable: the cache is regenerable by definition, so the
    /// caller deletes the file and asks for it again rather than failing.
    #[error("malformed thumbnail: {detail}")]
    MalformedThumbnail { detail: String },
}
