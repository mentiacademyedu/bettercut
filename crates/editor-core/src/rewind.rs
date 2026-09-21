//! The rewind: a shot plays, then winds back fast to where it began, with the
//! smeared colour and flicker of a tape being rewound. A reversed, sped-up copy
//! after the clip — a boomerang with the brakes off — and room made for it.
//! One undo step.

use bettercut_foundation::{ClipId, LinkId, Rational, TimelineTime};
use bettercut_timeline::{Clip, TimelineRange, timeline_ticks_for};

use crate::command::{ClipPayload, Command};
use crate::editor::Editor;
use crate::error::EditorError;

/// How much faster than the shot the rewind runs.
pub const REWIND_SPEED: i64 = 4;

impl Editor {
    /// Follow picture clip `clip` with itself winding back at [`REWIND_SPEED`]×
    /// with a worn-tape look, making room after it. Its sound, when it lines
    /// up, rewinds with it. Returns the rewind.
    pub fn rewind(&mut self, clip: ClipId) -> Result<ClipId, EditorError> {
        let sequence = self.active_sequence_id()?;
        let picture = self
            .video_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .clone();
        if !self.can_retime(clip) {
            return Err(EditorError::NoMotionToRetime);
        }
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let sound = self
            .linked_with(clip)
            .into_iter()
            .filter(|other| *other != clip)
            .find_map(|other| {
                let audio = self.audio_clip(other)?.clone();
                let track = self.track_of(other)?;
                (audio.timeline == picture.timeline).then_some((track, audio))
            });

        // Never past the fastest a clip may play, however fast it was already.
        let speed = bettercut_timeline::clamped_speed(
            Rational::new(picture.speed.num() * REWIND_SPEED, picture.speed.den())
                .ok_or(EditorError::NoMotionToRetime)?,
        );
        let at = picture.timeline.end;
        let length =
            TimelineTime::from_ticks(timeline_ticks_for(picture.source.duration(), speed).max(1));
        let span = TimelineRange::new(at, at + length)?;
        let link = sound.as_ref().map(|_| LinkId::new());

        let mut back = picture.clone();
        back.id = ClipId::new();
        back.timeline = span;
        back.speed = speed;
        back.reversed = !picture.reversed;
        back.set_link(link);
        // The tape look: colour fringes, a little breakup and a worn print.
        back.rgb_split = back.rgb_split.max(35.0);
        back.glitch = back.glitch.max(15.0);
        back.old_film = back.old_film.max(45.0);
        // Keyframes are in source time and would race through at this speed;
        // the rewind is a plain copy of the picture winding back.
        back.keyframes = Default::default();
        back.transition_out = picture.transition_out;
        let back_id = back.id;

        self.staged("Rewind", |editor, stage| {
            if picture.transition_out.is_some() {
                editor.stage(
                    stage,
                    Command::SetTransition {
                        sequence,
                        track,
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
                    track,
                    clip: ClipPayload::Video(Box::new(back)),
                },
            )?;
            if let Some((sound_track, audio)) = sound {
                let mut reply = audio.clone();
                reply.id = ClipId::new();
                reply.timeline = span;
                // The same speed as the picture, so the two stay together.
                reply.speed = speed;
                reply.reversed = !audio.reversed;
                reply.set_link(link);
                editor.stage(
                    stage,
                    Command::AddClip {
                        sequence,
                        track: sound_track,
                        clip: ClipPayload::Audio(Box::new(reply)),
                    },
                )?;
            }
            Ok(back_id)
        })
    }
}
