//! Project format errors.

use std::path::PathBuf;

use bettercut_foundation::MediaId;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("project file not found: {0}")]
    NotFound(PathBuf),

    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not serialize project: {0}")]
    Serialize(#[source] serde_json::Error),

    #[error("could not parse project file: {0}")]
    Deserialize(#[source] serde_json::Error),

    #[error("project file has no version field")]
    MissingVersion,

    #[error(
        "project file is version {found}, but this build supports up to {supported} — update the application"
    )]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("no migration path from schema version {from}")]
    NoMigrationPath { from: u32 },

    #[error("media {0} is still used by a clip")]
    MediaInUse(MediaId),

    #[error("no media {0} in this project")]
    MediaNotFound(MediaId),
}
