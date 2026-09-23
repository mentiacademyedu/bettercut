//! Filling the gap after a clip with more of the clip: its end pulled out to
//! meet the next clip on its lane, as far as the footage goes.

use bettercut_foundation::{ClipId, TimelineTime};

use crate::editor::Editor;
use crate::error::EditorError;
use crate::ops::TrimEdge;

impl Editor {
    /// Pull `clip`'s end (and its linked sound's) out towards the start of
    /// the next clip on its lane — all the way, or as far as its footage
    /// runs. One undo step. Returns the new end.
    pub fn extend_to_next(&mut self, clip: ClipId) -> Result<TimelineTime, EditorError> {
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let end = self.clip_end(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let next = self
            .active_sequence()
            .and_then(|sequence| {
                sequence
                    .clip_spans()
                    .filter(|s| s.track == track && s.clip != clip && s.timeline.start >= end)
                    .map(|s| s.timeline.start)
                    .min()
            })
            .filter(|next| *next > end)
            .ok_or(EditorError::NothingToExtendTo)?;
        let mut room = (next - end).ticks();
        let mut members = vec![clip];
        members.extend(self.linked_with(clip).into_iter().filter(|c| *c != clip));
        for member in members {
            if let Some(growth) = self.growth_room(member, TrimEdge::End) {
                room = room.min(growth);
            }
        }
        if room <= 0 {
            return Err(EditorError::NoFootageToExtend);
        }
        let to = TimelineTime::from_ticks(end.ticks() + room);
        self.trim_clip(track, clip, TrimEdge::End, to)?;
        Ok(to)
    }
}
