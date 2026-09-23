//! A new sequence made of the selected clips: the short cut from the long
//! edit, the vertical version of one scene, the reel of the best bits —
//! without touching the edit it came from.

use std::collections::HashSet;

use bettercut_foundation::{ClipId, SequenceId, TimelineTime};
use bettercut_timeline::Clip;

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Copy `clips` (and whatever is linked to them) into a new sequence
    /// named `name`, keeping their lanes and the spaces between them, moved
    /// so the earliest starts at zero. Markers inside the stretch come
    /// along. One undo step; the new sequence is not switched to. Returns
    /// its id.
    pub fn sequence_from_clips(
        &mut self,
        clips: &[ClipId],
        name: &str,
    ) -> Result<SequenceId, EditorError> {
        let source = self.active_sequence_id()?;
        let mut wanted: HashSet<ClipId> = HashSet::new();
        for clip in clips {
            for member in self.linked_with(*clip) {
                wanted.insert(member);
            }
            wanted.insert(*clip);
        }
        let spans: Vec<_> = {
            let sequence = self
                .active_sequence()
                .ok_or(EditorError::SequenceNotFound(source))?;
            wanted
                .iter()
                .filter_map(|clip| sequence.clip_span(*clip).map(|span| span.timeline))
                .collect()
        };
        let (Some(first), Some(last)) = (
            spans.iter().map(|s| s.start).min(),
            spans.iter().map(|s| s.end).max(),
        ) else {
            return Err(EditorError::NothingToCompound);
        };

        let (mut copy, index, map) = self.sequence_copy_mapped(source)?;
        let keep: HashSet<ClipId> = wanted
            .iter()
            .filter_map(|old| map.get(old).copied())
            .collect();
        let name = name.trim();
        copy.name = if name.is_empty() {
            format!(
                "{} (part)",
                self.active_sequence()
                    .map_or("Sequence", |s| s.name.as_str())
            )
        } else {
            name.to_owned()
        };
        let shift = first.ticks();

        fn prune<C: Clip + Clone>(
            track: &mut bettercut_timeline::Track<C>,
            keep: &HashSet<ClipId>,
            shift: i64,
        ) {
            let gone: Vec<ClipId> = track
                .clips()
                .iter()
                .map(Clip::id)
                .filter(|id| !keep.contains(id))
                .collect();
            for id in gone {
                let _ = track.remove(id);
            }
            // Earliest first: each moves left into room already cleared.
            let mut kept: Vec<(ClipId, TimelineTime)> = track
                .clips()
                .iter()
                .map(|c| (c.id(), c.timeline().start))
                .collect();
            kept.sort_by_key(|(_, start)| start.ticks());
            for (id, start) in kept {
                let _ = track.move_clip(id, TimelineTime::from_ticks(start.ticks() - shift));
            }
        }
        for track in &mut copy.video_tracks {
            prune(track, &keep, shift);
        }
        for track in &mut copy.audio_tracks {
            prune(track, &keep, shift);
        }
        for track in &mut copy.text_tracks {
            prune(track, &keep, shift);
        }
        for track in &mut copy.adjustment_tracks {
            prune(track, &keep, shift);
        }

        // What refers to clips that did not come along goes; marks inside
        // the stretch move with it.
        let exists =
            |clip: &ClipId, copy: &bettercut_timeline::Sequence| copy.clip_span(*clip).is_some();
        let snapshot = copy.clone();
        copy.notes.retain(|note| exists(&note.clip, &snapshot));
        copy.clip_marks.retain(|mark| exists(&mark.clip, &snapshot));
        copy.soloed_clips.retain(|clip| exists(clip, &snapshot));
        for group in &mut copy.groups {
            group.retain(|clip| exists(clip, &snapshot));
        }
        copy.groups.retain(|group| group.len() > 1);
        copy.markers = copy
            .markers
            .iter()
            .filter(|m| m.time >= first && m.time < last)
            .map(|m| {
                let mut m = m.clone();
                m.time = TimelineTime::from_ticks(m.time.ticks() - shift);
                m
            })
            .collect();
        copy.mark_in = None;
        copy.mark_out = None;

        let id = copy.id;
        self.dispatch(Command::AddSequence {
            sequence: Box::new(copy),
            index: index + 1,
        })?;
        Ok(id)
    }
}
