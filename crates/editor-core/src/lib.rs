//! Authoritative project state, commands, undo/redo, and events (§54–§56).
//!
//! The UI depends on this crate. This crate does not depend on the UI (§86).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod command;
pub mod editor;
pub mod error;
pub mod event;
pub mod hardware;
pub mod history;
pub mod journal;
pub mod ops;
pub mod recovery;

pub use command::{
    ClipPayload, Command, CommandGroup, EditorCommand, SettingChange, TrackFlag, TrackKindRepr,
    TrackPayload, TrimEdge,
};
pub use editor::Editor;
pub use error::EditorError;
pub use event::{Event, EventReceiver, EventSender, event_channel};
pub use hardware::HardwareProfile;
pub use history::{DEFAULT_HISTORY_LIMIT, History};
pub use journal::{Journal, RecoveryPaths};
pub use recovery::{RecoverableSession, recover, scan_unsaved};

// Re-exported so the UI crate needs one dependency for the whole core, and so
// there is one obvious place to look when the layering question comes up (§86).
pub use bettercut_foundation as foundation;
pub use bettercut_media as media;
pub use bettercut_project_format as project_format;
pub use bettercut_timeline as timeline;
