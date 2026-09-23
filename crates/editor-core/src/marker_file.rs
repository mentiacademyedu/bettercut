//! Writing the markers out as a file (`bettercut_timeline::marker_file`):
//! the sequence's own rate, start timecode and name go with them.

use std::path::Path;

use bettercut_timeline::{MarkerFileFormat, marker_file, parse_csv};

use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Write the active sequence's markers to `path`, as a CSV or, for a
    /// `.edl`, an EDL. Returns how many were written; refuses when there are
    /// none, rather than writing a file with nothing in it.
    pub fn export_markers(&self, path: &Path) -> Result<usize, EditorError> {
        let Some(sequence) = self.active_sequence().filter(|s| !s.markers.is_empty()) else {
            return Err(EditorError::NoMarkers);
        };
        let text = marker_file(
            &sequence.markers,
            MarkerFileFormat::of(path),
            sequence.frame_rate,
            sequence.start_timecode,
            &sequence.name,
        );
        std::fs::write(path, text).map_err(|e| EditorError::Io(e.to_string()))?;
        Ok(sequence.markers.len())
    }

    /// Read markers from a CSV at `path` and add them to the active
    /// sequence's, as one undo step. The markers already there stay; one
    /// imported onto the same instant as an existing one merges into it
    /// (`bettercut_timeline::marker::normalized`). Returns how many were
    /// added; a file with no marker it can read adds none and is not an
    /// error, so a wrong file costs nothing.
    pub fn import_markers(&mut self, path: &Path) -> Result<usize, EditorError> {
        let text = std::fs::read_to_string(path).map_err(|e| EditorError::Io(e.to_string()))?;
        let (rate, start) = {
            let sequence = self.active_sequence().ok_or(EditorError::NoMarkers)?;
            (sequence.frame_rate, sequence.start_timecode)
        };
        let imported = parse_csv(&text, rate, start);
        if imported.is_empty() {
            return Ok(0);
        }
        let before = self.markers().len();
        let mut markers = self.markers().to_vec();
        markers.extend(imported);
        let markers = bettercut_timeline::marker::normalized(markers);
        let added = markers.len().saturating_sub(before);
        if added > 0 {
            self.replace_markers(markers)?;
        }
        Ok(added)
    }
}
