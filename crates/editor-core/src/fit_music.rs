//! Fit music to the edit: a song that runs past the last picture trimmed to
//! end where the pictures do, with a fade-out so it ends rather than stops.
//!
//! The pictures decide where the edit ends: the last picture or title on any
//! lane. A sound clip that ends after that is trimmed back to it and given a
//! fade-out of a couple of seconds (less on a short clip). A song that already
//! ends in time only gets the fade. One undo step.

use bettercut_foundation::{ClipId, TimelineTime};

use crate::command::{Command, TrimEdge};
use crate::editor::Editor;
use crate::error::EditorError;

/// The fade-out a fitted song ends with, at most.
pub const FIT_FADE: TimelineTime = TimelineTime::from_seconds(2);

impl Editor {
    /// Where the pictures end: the latest end of any picture, title or
    /// adjustment clip. `None` with no pictures at all.
    pub fn picture_end(&self) -> Option<TimelineTime> {
        let sequence = self.active_sequence()?;
        sequence
            .clip_spans()
            .filter(|span| span.kind != bettercut_timeline::TrackKind::Audio)
            .map(|span| span.timeline.end)
            .max()
    }

    /// Trim sound clip `clip` to end where the pictures end, and fade it out
    /// over its last couple of seconds. Returns where it now ends.
    ///
    /// Refused for a clip with a picture of its own — a shot's sound belongs
    /// to the shot — and when the song starts after the pictures end.
    pub fn fit_music(&mut self, clip: ClipId) -> Result<TimelineTime, EditorError> {
        let sequence = self.active_sequence_id()?;
        let sound = self
            .audio_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .clone();
        if self.linked_with(clip).len() > 1 {
            return Err(EditorError::MusicHasPicture);
        }
        let end = self.picture_end().ok_or(EditorError::NothingToFitTo)?;
        if end <= sound.timeline.start {
            return Err(EditorError::NothingToFitTo);
        }
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let new_end = sound.timeline.end.min(end);
        let length = new_end.ticks() - sound.timeline.start.ticks();
        let fade_out = TimelineTime::from_ticks(FIT_FADE.ticks().min(length / 3));
        let fade_in =
            TimelineTime::from_ticks(sound.fade_in.ticks().min(length - fade_out.ticks()).max(0));

        let mut commands = Vec::new();
        if new_end < sound.timeline.end {
            commands.push(Command::TrimClip {
                sequence,
                track,
                clip,
                edge: TrimEdge::End,
                to: new_end,
            });
        }
        commands.push(Command::SetClipFades {
            sequence,
            track,
            clip,
            fade_in,
            fade_out,
        });
        self.dispatch_group("Fit Music".to_owned(), commands)?;
        Ok(new_end)
    }
}
