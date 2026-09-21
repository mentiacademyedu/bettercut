//! Split every clip at the markers.
//!
//! Markers dropped on the beat, or at each change of subject, are where the
//! cuts want to be. This makes every one of them a cut: each clip a marker
//! falls inside is split there, on every lane that is not locked, with a
//! picture and its sound split together (§12). One undo step.

use bettercut_foundation::TrackId;

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Split every clip at every marker — on `track` alone when given.
    /// Returns how many splits were made.
    pub fn split_at_markers(&mut self, track: Option<TrackId>) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let times: Vec<_> = self.markers().iter().map(|m| m.time).collect();
        if times.is_empty() {
            return Err(EditorError::NoMarkers);
        }
        let splits = self.staged("Split at Markers", |editor, stage| {
            let mut count = 0;
            for at in times {
                for command in editor.split_commands(sequence, at, &[]) {
                    let Command::SplitClip {
                        track: on, clip, ..
                    } = &command
                    else {
                        continue;
                    };
                    let locked = editor
                        .active_sequence()
                        .is_some_and(|s| crate::gaps::track_locked(s, *on));
                    // On one lane: that lane's clips, and whatever is tied to
                    // them, so a picture's sound is not left whole under it.
                    let wanted = track.is_none_or(|only| {
                        *on == only
                            || editor
                                .linked_with(*clip)
                                .iter()
                                .any(|c| editor.track_of(*c) == Some(only))
                    });
                    if locked || !wanted {
                        continue;
                    }
                    editor.stage(stage, command)?;
                    count += 1;
                }
            }
            Ok(count)
        })?;
        Ok(splits)
    }
}
