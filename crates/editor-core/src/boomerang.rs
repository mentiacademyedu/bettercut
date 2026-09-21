//! Boomerang: a shot that plays forwards and then straight back.
//!
//! The loop every phone camera offers, made from what the editor already has:
//! room is opened after the clip, as a freeze frame opens it, and a reversed
//! copy of the clip — and of its sound, still tied to its picture (§12) — is
//! dropped into it. Everything after moves along by the clip's length, so
//! nothing later falls out of step. One undo step (§79).

use bettercut_foundation::{ClipId, LinkId};
use bettercut_timeline::Clip;

use crate::command::{ClipPayload, Command};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Follow picture clip `clip` with itself played backwards. Returns the
    /// reversed copy.
    ///
    /// Refused for a photo or a held frame, which have no motion to play back.
    /// The linked sound is copied too when it lines up with the picture; a
    /// sound trimmed differently is left alone rather than put out of step.
    pub fn boomerang(&mut self, clip: ClipId) -> Result<ClipId, EditorError> {
        let sequence = self.active_sequence_id()?;
        let picture = self
            .video_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .clone();
        if !self.can_retime(clip) {
            return Err(EditorError::NoMotionToRetime);
        }
        let picture_track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let sound = self
            .linked_with(clip)
            .into_iter()
            .filter(|other| *other != clip)
            .find_map(|other| {
                let audio = self.audio_clip(other)?.clone();
                let track = self.track_of(other)?;
                (audio.timeline == picture.timeline).then_some((track, audio))
            });

        let at = picture.timeline.end;
        let length = picture.timeline.duration();
        let link = sound.as_ref().map(|_| LinkId::new());

        let mut back = picture.clone();
        back.id = ClipId::new();
        back.timeline = bettercut_timeline::TimelineRange::new(at, at + length)?;
        back.reversed = !picture.reversed;
        back.set_link(link);
        // The cut between the two halves is the turn: whatever led out of the
        // shot now leads out of its reversed copy, and nothing sits in between.
        back.transition_out = picture.transition_out;
        let back_id = back.id;

        self.staged("Boomerang", |editor, stage| {
            if picture.transition_out.is_some() {
                editor.stage(
                    stage,
                    Command::SetTransition {
                        sequence,
                        track: picture_track,
                        clip,
                        transition: None,
                    },
                )?;
            }
            editor.stage_insert_time(stage, sequence, at, length)?;
            editor.stage(
                stage,
                Command::AddClip {
                    sequence,
                    track: picture_track,
                    clip: ClipPayload::Video(Box::new(back)),
                },
            )?;
            if let Some((track, audio)) = sound {
                let mut reply = audio.clone();
                reply.id = ClipId::new();
                reply.timeline = bettercut_timeline::TimelineRange::new(at, at + length)?;
                reply.reversed = !audio.reversed;
                reply.set_link(link);
                editor.stage(
                    stage,
                    Command::AddClip {
                        sequence,
                        track,
                        clip: ClipPayload::Audio(Box::new(reply)),
                    },
                )?;
            }
            Ok(back_id)
        })
    }
}
