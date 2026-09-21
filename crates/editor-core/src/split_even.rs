//! Split a clip into equal parts, or into pieces of a set length: a long take
//! cut into ten-second chunks for a slideshow of moments, or into three for
//! a story told in parts. One undo step, sound cut with its picture (§12).

use bettercut_foundation::{ClipId, TimelineTime};

use crate::editor::Editor;
use crate::error::EditorError;

/// The most pieces one split makes. Past this the timeline is slivers, and a
/// mistyped length should not put a thousand clips in the edit.
pub const MAX_SPLIT_PIECES: u32 = 100;

impl Editor {
    /// Cut `clip` into `parts` pieces of equal length, as one undo step.
    /// Returns how many cuts were made.
    ///
    /// Refused for fewer than two parts, more than [`MAX_SPLIT_PIECES`], or a
    /// clip too short to give each part a frame.
    pub fn split_into_parts(&mut self, clip: ClipId, parts: u32) -> Result<usize, EditorError> {
        if !(2..=MAX_SPLIT_PIECES).contains(&parts) {
            return Err(EditorError::SplitTooFine);
        }
        let range = self.span_of(clip)?;
        let length = range.1 - range.0;
        let times: Vec<TimelineTime> = (1..parts)
            .map(|i| {
                TimelineTime::from_ticks(
                    range.0.ticks() + length.ticks() * i64::from(i) / i64::from(parts),
                )
            })
            .collect();
        self.split_evenly(clip, &times, range)
    }

    /// Cut `clip` every `every` from its start, as one undo step; the last
    /// piece is whatever is left over. Returns how many cuts were made.
    ///
    /// Refused when that would make more than [`MAX_SPLIT_PIECES`] pieces, or
    /// when the clip is no longer than one piece.
    pub fn split_every(&mut self, clip: ClipId, every: TimelineTime) -> Result<usize, EditorError> {
        let range = self.span_of(clip)?;
        let length = range.1 - range.0;
        if every <= TimelineTime::ZERO {
            return Err(EditorError::SplitTooFine);
        }
        let cuts = (length.ticks() - 1) / every.ticks();
        if cuts < 1 {
            return Err(EditorError::SplitTooFine);
        }
        if cuts + 1 > i64::from(MAX_SPLIT_PIECES) {
            return Err(EditorError::SplitTooFine);
        }
        let times: Vec<TimelineTime> = (1..=cuts)
            .map(|i| range.0 + TimelineTime::from_ticks(every.ticks() * i))
            .collect();
        self.split_evenly(clip, &times, range)
    }

    fn span_of(&self, clip: ClipId) -> Result<(TimelineTime, TimelineTime), EditorError> {
        let span = self
            .active_sequence()
            .and_then(|s| s.clip_span(clip))
            .ok_or(EditorError::ClipNotFound(clip))?;
        Ok((span.timeline.start, span.timeline.end))
    }

    fn split_evenly(
        &mut self,
        clip: ClipId,
        times: &[TimelineTime],
        range: (TimelineTime, TimelineTime),
    ) -> Result<usize, EditorError> {
        // Every piece at least a frame: cuts that snap onto each other or onto
        // the clip's edges would quietly make fewer pieces than asked for.
        let frame = self
            .active_sequence()
            .and_then(|s| bettercut_foundation::ticks_per_frame(s.frame_rate))
            .unwrap_or(1);
        let mut previous = range.0;
        for at in times.iter().copied().chain([range.1]) {
            if (at - previous).ticks() < frame {
                return Err(EditorError::SplitTooFine);
            }
            previous = at;
        }
        self.split_clip_at(clip, times)
    }
}
