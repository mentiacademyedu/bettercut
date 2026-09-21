//! The cover: the frame chosen to stand for a video — a thumbnail for where it
//! is posted — saved as a picture beside every export of it.

use std::path::{Path, PathBuf};

use bettercut_foundation::TimelineTime;

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

/// Where the cover picture goes for an export written to `export`: beside it,
/// named after it — `Holiday.mp4` gets `Holiday cover.png`.
pub fn cover_path_for(export: &Path) -> PathBuf {
    let stem = export
        .file_stem()
        .map_or_else(|| "video".to_owned(), |s| s.to_string_lossy().into_owned());
    export.with_file_name(format!("{stem} cover.png"))
}

impl Editor {
    /// Choose the frame at `at` as the cover of the sequence on screen, or
    /// clear it with `None`, as one undo step.
    pub fn set_cover_frame(&mut self, at: Option<TimelineTime>) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        if self.active_sequence().and_then(|s| s.cover_frame) == at {
            return Ok(());
        }
        self.dispatch(Command::SetCoverFrame { sequence, at })
    }

    /// The cover frame of the sequence on screen, if one was chosen.
    pub fn cover_frame(&self) -> Option<TimelineTime> {
        self.active_sequence()?.cover_frame
    }
}
