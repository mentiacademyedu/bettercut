//! Insert drag: drop a clip between others and everything from there on moves
//! along to make room, instead of the drop being refused for landing on
//! something.
//!
//! The room is opened across every track, as the insert-gap edit does, so the
//! sound and titles after the drop keep their place against the picture.

use bettercut_foundation::{ClipId, TimelineTime, TrackId};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Move `clip` — with its sound and anything grouped with it — to start at
    /// `at` on `to_track`, pushing whatever is at `at` or later along by the
    /// room it needs. A clip `at` falls inside is split there first.
    ///
    /// The place it left is not closed up (a magnetic main track does that on
    /// its own). One undo step.
    pub fn insert_clip_at(
        &mut self,
        clip: ClipId,
        to_track: TrackId,
        at: TimelineTime,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let active = self
            .project()
            .sequence(sequence)
            .ok_or(EditorError::SequenceNotFound(sequence))?;
        let at = active.snap_to_frame(at.max(TimelineTime::ZERO));
        let from_track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        if active.track_kind(from_track) != active.track_kind(to_track) {
            return Err(EditorError::ClipKindMismatch);
        }

        // Everything that travels with the clip, where it is now.
        let spans: Vec<(TrackId, ClipId, bettercut_timeline::TimelineRange)> = self
            .moves_with(clip)
            .into_iter()
            .filter_map(|member| {
                let span = active.clip_span(member)?;
                Some((span.track, member, span.timeline))
            })
            .collect();
        let own = spans
            .iter()
            .find(|(_, member, _)| *member == clip)
            .map(|(_, _, range)| *range)
            .ok_or(EditorError::ClipNotFound(clip))?;
        let first = spans
            .iter()
            .map(|(_, _, r)| r.start)
            .min()
            .unwrap_or(own.start);
        let last = spans.iter().map(|(_, _, r)| r.end).max().unwrap_or(own.end);
        let delta = at.ticks() - own.start.ticks();
        // The room opens where the earliest of them will land, as wide as they
        // span together.
        let gap_at = TimelineTime::from_ticks((first.ticks() + delta).max(0));
        let room = last - first;
        // Somewhere past everything, clear of the whole edit, to wait in while
        // the room is made — otherwise the clip would be pushed along with the
        // rest, or split by its own insert.
        let parked = active.duration().ticks() + room.ticks() - first.ticks();

        self.staged("Insert Clip", |editor, stage| {
            for (track, member, range) in &spans {
                editor.stage(
                    stage,
                    Command::MoveClip {
                        sequence,
                        from_track: *track,
                        to_track: *track,
                        clip: *member,
                        new_start: TimelineTime::from_ticks(range.start.ticks() + parked),
                    },
                )?;
            }
            editor.stage_insert_time(stage, sequence, gap_at, room)?;
            // Set each down in the room. A move names only where a clip is
            // going, so it does not matter that parking put them somewhere else.
            for (track, member, range) in &spans {
                let to = if *member == clip { to_track } else { *track };
                editor.stage(
                    stage,
                    Command::MoveClip {
                        sequence,
                        from_track: *track,
                        to_track: to,
                        clip: *member,
                        new_start: TimelineTime::from_ticks(range.start.ticks() + delta),
                    },
                )?;
            }
            Ok(())
        })
    }
}
