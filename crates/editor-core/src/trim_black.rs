//! Taking the ends off a clip in one step: the black before the camera woke
//! up and after the lens cap went back on.

use bettercut_foundation::{ClipId, TimelineTime};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;
use crate::ops::TrimEdge;

impl Editor {
    /// How long a clip is on the timeline.
    pub fn clip_duration(&self, clip: ClipId) -> Option<TimelineTime> {
        Some(self.clip_end(clip)? - self.clip_start(clip)?)
    }

    /// Trim `clip` so it starts at `from` and ends at `to` on the timeline
    /// (`None` leaves that edge), with whatever is linked to it trimmed by
    /// the same amounts. One undo step, labelled `label`. Returns whether
    /// anything changed.
    pub fn trim_ends(
        &mut self,
        clip: ClipId,
        from: Option<TimelineTime>,
        to: Option<TimelineTime>,
        label: &str,
    ) -> Result<bool, EditorError> {
        let sequence = self.active_sequence_id()?;
        let (Some(start), Some(end)) = (self.clip_start(clip), self.clip_end(clip)) else {
            return Err(EditorError::ClipNotFound(clip));
        };
        let from = from.filter(|at| *at > start && *at < end);
        let to = to.filter(|at| *at > start && *at < end);
        if let (Some(from), Some(to)) = (from, to)
            && from >= to
        {
            return Ok(false); // nothing would be left
        }
        let mut members = vec![clip];
        members.extend(self.linked_with(clip).into_iter().filter(|c| *c != clip));
        let mut commands = Vec::new();
        for member in members {
            let (Some(track), Some(own_start), Some(own_end)) = (
                self.track_of(member),
                self.clip_start(member),
                self.clip_end(member),
            ) else {
                continue;
            };
            if let Some(from) = from {
                commands.push(Command::TrimClip {
                    sequence,
                    track,
                    clip: member,
                    edge: TrimEdge::Start,
                    to: own_start + (from - start),
                });
            }
            if let Some(to) = to {
                commands.push(Command::TrimClip {
                    sequence,
                    track,
                    clip: member,
                    edge: TrimEdge::End,
                    to: TimelineTime::from_ticks(own_end.ticks() - (end - to).ticks()),
                });
            }
        }
        if commands.is_empty() {
            return Ok(false);
        }
        self.dispatch_group(label, commands)?;
        Ok(true)
    }
}
