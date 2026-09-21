//! The freeze-frame punch-in: the shot stops dead on a frame, the picture
//! punches in on it, and a flash marks the moment — the "record scratch" of a
//! thousand short videos. A freeze frame with two things added to it, all in
//! one undo step.

use bettercut_foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_timeline::{AnimatedParameter, Interpolation, Keyframe, Transition, TransitionKind};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

/// How much bigger the held picture ends up.
pub const PUNCH_ZOOM: f32 = 1.25;

/// How long the punch takes to land.
pub const PUNCH_TIME: MediaTime = MediaTime::from_millis(200);

/// How long the flash at the cut runs.
pub const PUNCH_FLASH: TimelineTime = TimelineTime::from_millis(300);

impl Editor {
    /// Hold `clip`'s frame under the playhead for `duration`, zoom in on it
    /// and flash into it, as one undo step. Returns the held clip.
    pub fn freeze_punch(
        &mut self,
        clip: ClipId,
        duration: TimelineTime,
    ) -> Result<ClipId, EditorError> {
        let sequence = self.active_sequence_id()?;
        let held = self.freeze_frame(clip, duration)?;
        let track = self.track_of(held).ok_or(EditorError::ClipNotFound(held))?;
        let frozen = self
            .video_clip(held)
            .ok_or(EditorError::ClipNotFound(held))?
            .clone();

        let mut commands = Vec::new();
        // The punch: from the shot's own size to a quarter bigger, fast, then
        // held there. Keys are in the held clip's own time.
        let from = frozen.source.start;
        let landed = MediaTime::from_ticks(from.ticks() + PUNCH_TIME.ticks());
        for (parameter, base) in [
            (AnimatedParameter::ScaleX, frozen.transform.scale.x),
            (AnimatedParameter::ScaleY, frozen.transform.scale.y),
        ] {
            for (at, value) in [(from, base), (landed, base * PUNCH_ZOOM)] {
                commands.push(Command::SetKeyframe {
                    sequence,
                    track,
                    clip: held,
                    parameter,
                    key: Keyframe::new(at, value, Interpolation::EaseInOut),
                });
            }
        }

        // The flash, on the shot just before the hold — when there is one.
        let before = self.active_sequence().and_then(|s| {
            s.video_tracks
                .iter()
                .find(|t| t.id == track)?
                .clips()
                .iter()
                .find(|c| c.timeline.end == frozen.timeline.start)
                .map(|c| c.id)
        });
        if let Some(before) = before {
            commands.push(Command::SetTransition {
                sequence,
                track,
                clip: before,
                transition: Some(Transition::new(TransitionKind::Flash, PUNCH_FLASH)),
            });
        }

        self.dispatch_amending(commands)?;
        Ok(held)
    }
}
