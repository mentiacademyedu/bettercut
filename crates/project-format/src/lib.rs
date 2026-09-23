//! The `Project` aggregate and its `.vproj` file format (§37, §38.1).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod error;
pub mod file;
pub mod project;
pub mod settings;

pub use error::ProjectError;
pub use file::{PROJECT_EXTENSION, SCHEMA_VERSION, load, save};
pub use project::{LutAsset, Project};
pub use settings::{
    MAX_PHOTO_LENGTH, MAX_TRANSITION_LENGTH, MIN_PHOTO_LENGTH, MIN_TRANSITION_LENGTH,
    PerformanceMode, ProjectSettings,
};
