//! Rate stretch: make a shot fill a length by changing how fast it plays,
//! rather than how much of it plays.
//!
//! An ordinary trim answers "show less of this"; a rate stretch answers "make
//! this fit here". The gap before the music comes in is four and a half
//! seconds and the shot is five: trimming it loses half a second of the
//! action, and re-timing it by hand means working out 5 ÷ 4.5 and typing
//! 111%. Dragging the end to where it has to be is the same edit, arrived at
//! by looking.
//!
//! # Anchored at the start
//!
//! A speed change keeps the clip's start and re-derives its end
//! (`ops::SetClipSpeed`), rippling the lane behind it — so a stretch is asked
//! for as *where the clip should end*, and the start is where it was. There is
//! no stretch of the head for the same reason: it would mean re-timing a clip
//! and then moving it back under itself, which is two edits pretending to be
//! one.
//!
//! The picture and its sound are re-timed together (§12), as any speed change
//! is, and the speed is held to the same limits the slider has: a drag past
//! them stops there rather than being refused.

use bettercut_foundation::{ClipId, Rational, TimelineTime};

use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// The speed that would make `clip` end at `to`, held to what the editor
    /// allows.
    ///
    /// `None` when there is nothing to re-time — a photo or a held frame has
    /// no motion, so stretching it is an ordinary trim.
    pub fn rate_stretch_speed(&self, clip: ClipId, to: TimelineTime) -> Option<Rational> {
        if !self.can_retime(clip) {
            return None;
        }
        let span = self.active_sequence()?.clip_span(clip)?.timeline;
        let wanted = to.ticks() - span.start.ticks();
        if wanted <= 0 {
            return None;
        }
        let material = self
            .video_clip(clip)
            .map(|video| video.source.duration())
            .or_else(|| self.audio_clip(clip).map(|audio| audio.source.duration()))?;
        // Speed is material over time: the same footage across a shorter
        // stretch plays faster.
        let speed = Rational::new(material.ticks(), wanted)?;
        Some(bettercut_timeline::clamped_speed(speed))
    }

    /// Re-time `clip` so that it ends at `to`, keeping all of the footage it
    /// plays. Returns the speed it ended up at.
    ///
    /// `continuing` collapses a drag into one undo step (§11), as the speed
    /// slider does — and for the same reason, since this is that slider being
    /// moved by the clip's edge.
    pub fn rate_stretch(
        &mut self,
        clip: ClipId,
        to: TimelineTime,
        continuing: bool,
    ) -> Result<Rational, EditorError> {
        let sequence = self.active_sequence_id()?;
        let to = self.snap_to_frame(sequence, to);
        // A drag that would leave the clip shorter than a frame is a drag that
        // has run out of room, not a mistake: it asks for the fastest the clip
        // can play, and the speed limit answers.
        let to = match self
            .active_sequence()
            .and_then(|active| active.clip_span(clip))
        {
            Some(span) => {
                let frame = self
                    .active_sequence()
                    .and_then(|active| bettercut_foundation::ticks_per_frame(active.frame_rate))
                    .unwrap_or(1)
                    .max(1);
                to.max(TimelineTime::from_ticks(
                    span.timeline.start.ticks() + frame,
                ))
            }
            None => to,
        };
        let speed = self
            .rate_stretch_speed(clip, to)
            .ok_or(EditorError::NoMotionToRetime)?;
        self.set_clip_speed(clip, speed, continuing)?;
        Ok(speed)
    }
}
