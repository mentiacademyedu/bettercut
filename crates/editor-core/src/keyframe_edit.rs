//! Editing a keyframe that is already there: moving one and deleting one.
//!
//! The inspector's ◇ buttons put a key at the playhead and take it away again;
//! these are what a graph of the curve needs, where a key is dragged to another
//! time or another value and deleted where it stands.

use bettercut_foundation::{ClipId, MediaTime};
use bettercut_timeline::{AnimatedParameter, Interpolation, Keyframe};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// The keys on one of a clip's parameters, earliest first.
    pub fn keyframes_of(&self, clip: ClipId, parameter: AnimatedParameter) -> Vec<Keyframe> {
        let keys = self
            .video_clip(clip)
            .map(|c| &c.keyframes)
            .or_else(|| self.audio_clip(clip).map(|c| &c.keyframes));
        keys.and_then(|keys| keys.track(parameter))
            .map(|track| track.keys().to_vec())
            .unwrap_or_default()
    }

    /// Move the key at `from` to `to`, and give it `value` — what dragging a
    /// point on the curve does. The value is held to the parameter's limits.
    ///
    /// One undo step: the key is removed from where it was and set where it is
    /// going, which also replaces whatever key was already at `to`.
    pub fn move_keyframe(
        &mut self,
        clip: ClipId,
        parameter: AnimatedParameter,
        from: MediaTime,
        to: MediaTime,
        value: f32,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let existing = self
            .keyframes_of(clip, parameter)
            .into_iter()
            .find(|key| key.time == from)
            .ok_or(EditorError::NoKeyframeThere)?;
        let value = parameter.clamp(value);
        if to == from && (existing.value - value).abs() < f32::EPSILON {
            return Ok(());
        }
        let key = Keyframe::new(to, value, existing.interpolation);
        if to == from {
            return self.dispatch(Command::SetKeyframe {
                sequence,
                track,
                clip,
                parameter,
                key,
            });
        }
        self.dispatch_group(
            format!("Move {} Keyframe", parameter.label()),
            vec![
                Command::RemoveKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter,
                    time: from,
                },
                Command::SetKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter,
                    key,
                },
            ],
        )
    }

    /// Delete the key at `at`, leaving the rest of the parameter's curve.
    pub fn remove_keyframe_at(
        &mut self,
        clip: ClipId,
        parameter: AnimatedParameter,
        at: MediaTime,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        if !self
            .keyframes_of(clip, parameter)
            .iter()
            .any(|key| key.time == at)
        {
            return Err(EditorError::NoKeyframeThere);
        }
        self.dispatch(Command::RemoveKeyframe {
            sequence,
            track,
            clip,
            parameter,
            time: at,
        })
    }

    /// How a key leaves for the next one. Set on the key at `at`.
    pub fn set_keyframe_interpolation(
        &mut self,
        clip: ClipId,
        parameter: AnimatedParameter,
        at: MediaTime,
        interpolation: Interpolation,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let existing = self
            .keyframes_of(clip, parameter)
            .into_iter()
            .find(|key| key.time == at)
            .ok_or(EditorError::NoKeyframeThere)?;
        if existing.interpolation == interpolation {
            return Ok(());
        }
        self.dispatch(Command::SetKeyframe {
            sequence,
            track,
            clip,
            parameter,
            key: Keyframe::new(at, existing.value, interpolation),
        })
    }
}
