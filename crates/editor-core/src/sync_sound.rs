//! Lining a clip up with another by their sound.
//!
//! The listening is done elsewhere — [`bettercut_cache::align`] slides the two
//! loudness shapes over each other — and what is left is the arithmetic: the
//! shift it found is between two *files*, and what has to move is a clip on the
//! timeline that plays part of one of them.

use bettercut_foundation::{ClipId, TICKS_PER_SECOND, TimelineTime};

use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Where `clip` would have to start for the sound it plays to line up with
    /// `reference`, given that `clip`'s file matches `reference`'s file
    /// `shift_seconds` later in it.
    ///
    /// `None` when either clip is not on the timeline. It can be negative,
    /// which means the two cannot be lined up without moving the reference —
    /// [`Self::sync_to_sound`] says so rather than clamping.
    pub fn sync_start(
        &self,
        clip: ClipId,
        reference: ClipId,
        shift_seconds: f64,
    ) -> Option<TimelineTime> {
        let (_, clip_source) = self.timeline_and_source_start(clip)?;
        let (reference_start, reference_source) = self.timeline_and_source_start(reference)?;
        let shift = (shift_seconds * TICKS_PER_SECOND as f64).round() as i64;
        // The instant in the reference's file that `clip` opens on, and where
        // that instant sits on the timeline.
        let into_reference = clip_source + shift - reference_source;
        Some(TimelineTime::from_ticks(reference_start + into_reference))
    }

    /// Move `clip` so its sound lines up with `reference`'s, by a shift found
    /// from their recordings. Its linked partner moves with it (§12).
    ///
    /// Refused when the answer would put the clip before the start of the
    /// timeline: the two recordings do line up, but only somewhere there is no
    /// room for — the reference is the thing to move in that case.
    pub fn sync_to_sound(
        &mut self,
        clip: ClipId,
        reference: ClipId,
        shift_seconds: f64,
    ) -> Result<TimelineTime, EditorError> {
        let start = self
            .sync_start(clip, reference, shift_seconds)
            .ok_or(EditorError::ClipNotFound(clip))?;
        if start.is_negative() {
            return Err(EditorError::NoRoomToSync);
        }
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        self.move_clip(track, track, clip, start)?;
        Ok(start)
    }

    /// Where a clip starts on the timeline and where it starts in its file,
    /// both in ticks — the two numbers the arithmetic above needs.
    fn timeline_and_source_start(&self, clip: ClipId) -> Option<(i64, i64)> {
        if let Some(video) = self.video_clip(clip) {
            return Some((video.timeline.start.ticks(), video.source.start.ticks()));
        }
        let audio = self.audio_clip(clip)?;
        Some((audio.timeline.start.ticks(), audio.source.start.ticks()))
    }
}
