//! Loop a clip: the same shot, or the same stretch of music, played several
//! times back to back. Room is made after it, as a boomerang makes room, so
//! nothing later falls out of step. One undo step (§79).

use bettercut_foundation::{ClipId, LinkId, TimelineTime};
use bettercut_timeline::{Clip, TimelineRange};

use crate::command::{ClipPayload, Command};
use crate::editor::Editor;
use crate::error::EditorError;

/// The most times one loop plays a clip.
pub const MAX_LOOPS: u32 = 20;

impl Editor {
    /// Play `clip` `times` times in a row — itself, then `times − 1` copies
    /// straight after it — moving everything later along. Returns the copies.
    ///
    /// A picture brings its sound when the sound lines up with it; picking the
    /// sound of a picture loops the picture. Music on its own loops on its own.
    /// A transition out of the clip moves to the end of the last copy, so the
    /// loops join with clean cuts. Refused for fewer than two plays or more
    /// than [`MAX_LOOPS`].
    pub fn loop_clip(&mut self, clip: ClipId, times: u32) -> Result<Vec<ClipId>, EditorError> {
        if !(2..=MAX_LOOPS).contains(&times) {
            return Err(EditorError::LoopCountOutOfRange);
        }
        let sequence = self.active_sequence_id()?;

        // The sound of a picture loops the picture.
        let clip = if self.audio_clip(clip).is_some() {
            self.linked_with(clip)
                .into_iter()
                .find(|other| self.video_clip(*other).is_some())
                .unwrap_or(clip)
        } else {
            clip
        };

        let picture = self.video_clip(clip).cloned();
        let lead_track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let (range, sound) = match &picture {
            Some(picture) => {
                let sound = self
                    .linked_with(clip)
                    .into_iter()
                    .filter(|other| *other != clip)
                    .find_map(|other| {
                        let audio = self.audio_clip(other)?.clone();
                        let track = self.track_of(other)?;
                        (audio.timeline == picture.timeline).then_some((track, audio))
                    });
                (picture.timeline, sound)
            }
            None => {
                let audio = self
                    .audio_clip(clip)
                    .ok_or(EditorError::ClipKindMismatch)?
                    .clone();
                (audio.timeline, None)
            }
        };
        let music = if picture.is_none() {
            self.audio_clip(clip).cloned()
        } else {
            None
        };

        let length = range.duration();
        let at = range.end;
        let added = TimelineTime::from_ticks(length.ticks() * i64::from(times - 1));
        let transition = picture.as_ref().and_then(|p| p.transition_out);

        self.staged(format!("Loop {times} Times"), |editor, stage| {
            if transition.is_some() {
                editor.stage(
                    stage,
                    Command::SetTransition {
                        sequence,
                        track: lead_track,
                        clip,
                        transition: None,
                    },
                )?;
            }
            editor.stage_insert_time(stage, sequence, at, added)?;

            let mut copies = Vec::new();
            for index in 1..times {
                let start = TimelineTime::from_ticks(
                    range.start.ticks() + length.ticks() * i64::from(index),
                );
                let span = TimelineRange::new(start, start + length)?;
                let last = index == times - 1;
                let link = sound.as_ref().map(|_| LinkId::new());

                if let Some(picture) = &picture {
                    let mut copy = picture.clone();
                    copy.id = ClipId::new();
                    copy.timeline = span;
                    copy.set_link(link);
                    copy.transition_out = if last { transition } else { None };
                    copies.push(copy.id);
                    editor.stage(
                        stage,
                        Command::AddClip {
                            sequence,
                            track: lead_track,
                            clip: ClipPayload::Video(Box::new(copy)),
                        },
                    )?;
                }
                let audio = match (&sound, &music) {
                    (Some((track, audio)), _) => Some((*track, audio)),
                    (None, Some(audio)) => Some((lead_track, audio)),
                    (None, None) => None,
                };
                if let Some((track, audio)) = audio {
                    let mut copy = audio.clone();
                    copy.id = ClipId::new();
                    copy.timeline = span;
                    copy.set_link(link);
                    if picture.is_none() {
                        copies.push(copy.id);
                    }
                    editor.stage(
                        stage,
                        Command::AddClip {
                            sequence,
                            track,
                            clip: ClipPayload::Audio(Box::new(copy)),
                        },
                    )?;
                }
            }
            Ok(copies)
        })
    }
}
