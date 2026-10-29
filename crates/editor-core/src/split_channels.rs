//! Two microphones on one recording's two sides — a wireless kit's split
//! mode, a camera with a mic in each input — taken apart into two sounds:
//! the left on its own lane, the right on another, each in both ears, so
//! each voice is cut, levelled and cleaned up by itself.

use bettercut_foundation::{ClipId, LinkId, TrackId};
use bettercut_timeline::ChannelMode;

use crate::command::{ClipPayload, Command, TrackKindRepr};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Split sound clip `clip` into its left and right channels: the clip
    /// plays the left, a copy of it on the first sound lane with room (a new
    /// one if none has) plays the right, and the two are linked — to the
    /// picture too, where the clip was — so they move and cut together.
    /// One undo step; returns the new clip.
    pub fn split_left_right(&mut self, clip: ClipId) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sound = self
            .audio_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?
            .clone();
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let stereo = self
            .project()
            .media_asset(sound.media_id)
            .and_then(|asset| asset.audio_channels)
            .is_some_and(|channels| channels >= 2);
        if !stereo {
            return Err(EditorError::NotStereo);
        }
        let sequence = self
            .project()
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let free = sequence
            .audio_tracks
            .iter()
            .find(|lane| lane.id != track && lane.clips_in_range(sound.timeline).is_empty())
            .map(|lane| lane.id);
        let lanes = sequence.audio_tracks.len();

        let link = sound.link.unwrap_or_else(LinkId::new);
        let mut left = sound.clone();
        left.channels = ChannelMode::LeftToBoth;
        left.link = Some(link);
        let mut right = sound;
        right.id = ClipId::new();
        right.channels = ChannelMode::RightToBoth;
        right.link = Some(link);
        let made = right.id;

        let mut commands = Vec::new();
        let lane = match free {
            Some(lane) => lane,
            None => {
                let id = TrackId::new();
                commands.push(Command::AddTrack {
                    sequence: sequence_id,
                    kind: TrackKindRepr::Audio,
                    name: format!("A{}", lanes + 1),
                    id,
                });
                id
            }
        };
        // The clip itself is put back, as the left, rather than changed:
        // that way it takes the link along with the channel.
        commands.push(Command::RemoveClip {
            sequence: sequence_id,
            track,
            clip,
        });
        commands.push(Command::AddClip {
            sequence: sequence_id,
            track,
            clip: ClipPayload::Audio(Box::new(left)),
        });
        commands.push(Command::AddClip {
            sequence: sequence_id,
            track: lane,
            clip: ClipPayload::Audio(Box::new(right)),
        });
        // Staged rather than grouped: a lane made here has to exist before
        // the clip that goes on it is checked.
        self.staged("Split Left and Right", |editor, stage| {
            for command in commands {
                editor.stage(stage, command)?;
            }
            Ok(())
        })?;
        Ok(made)
    }
}
