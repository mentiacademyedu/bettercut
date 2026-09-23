//! Things done to every lane at once, and to every clip of a colour.

use bettercut_foundation::ClipId;

use crate::command::{Command, TrackFlag};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Set `flag` on every lane of every kind, as one undo step. Returns how
    /// many lanes changed; none already so is no step at all.
    pub fn set_all_tracks_flag(
        &mut self,
        flag: TrackFlag,
        value: bool,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let lanes: Vec<_> = {
            let s = self
                .active_sequence()
                .ok_or(EditorError::SequenceNotFound(sequence))?;
            s.video_tracks
                .iter()
                .map(|t| t.id)
                .chain(s.audio_tracks.iter().map(|t| t.id))
                .chain(s.text_tracks.iter().map(|t| t.id))
                .chain(s.adjustment_tracks.iter().map(|t| t.id))
                .collect()
        };
        let commands: Vec<Command> = lanes
            .into_iter()
            .filter(|track| self.track_flag(*track, flag) != value)
            .map(|track| Command::SetTrackFlag {
                sequence,
                track,
                flag,
                value,
            })
            .collect();
        if commands.is_empty() {
            return Ok(0);
        }
        let count = commands.len();
        let label = match (flag, value) {
            (TrackFlag::Locked, true) => "Lock All Lanes",
            (TrackFlag::Locked, false) => "Unlock All Lanes",
            (TrackFlag::Enabled, true) => "Show All Lanes",
            (TrackFlag::Enabled, false) => "Hide All Lanes",
            _ => "Every Lane",
        };
        self.dispatch_group(label, commands)?;
        Ok(count)
    }

    /// Every clip on the active sequence carrying `label`, any lane.
    pub fn clips_with_colour(&self, label: bettercut_timeline::ColorLabel) -> Vec<ClipId> {
        let Some(sequence) = self.active_sequence() else {
            return Vec::new();
        };
        sequence
            .clip_spans()
            .map(|span| span.clip)
            .filter(|clip| self.color_label(*clip) == Some(label))
            .collect()
    }
}
