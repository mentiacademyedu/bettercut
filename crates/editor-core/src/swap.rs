//! Swapping a clip with its neighbour: two shots trade places.
//!
//! The commonest reordering in a rough cut — "this one should come first" —
//! is otherwise a drag out of the way, a drag into the hole, and a drag back.
//! Here it is one edit. The pair keeps the stretch of timeline it covered: the
//! later clip moves to where the earlier one started, the earlier one follows
//! it, and any gap between them stays between them.
//!
//! Linked sound travels with its picture (§12), by the same amount, on its own
//! track. When that would land it on something that is not part of the swap,
//! the swap is refused and nothing moves.

use bettercut_foundation::{ClipId, SequenceId, TimelineTime, TrackId};
use bettercut_timeline::TimelineRange;

use crate::command::Command;
use crate::editor::{Editor, Stage};
use crate::error::EditorError;

/// Which neighbour to swap with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Neighbour {
    Previous,
    Next,
}

impl Editor {
    /// The clip beside `clip` on its track, in `direction`, if there is one.
    pub fn neighbour_of(&self, clip: ClipId, direction: Neighbour) -> Option<ClipId> {
        let sequence = self.project().active()?;
        let span = sequence.clip_span(clip)?;
        let mut on_track: Vec<(TimelineRange, ClipId)> = sequence
            .clip_spans()
            .filter(|other| other.track == span.track && other.clip != clip)
            .map(|other| (other.timeline, other.clip))
            .collect();
        on_track.sort_by_key(|(range, _)| range.start);
        match direction {
            Neighbour::Next => on_track
                .iter()
                .find(|(range, _)| range.start >= span.timeline.end)
                .map(|(_, id)| *id),
            Neighbour::Previous => on_track
                .iter()
                .rev()
                .find(|(range, _)| range.end <= span.timeline.start)
                .map(|(_, id)| *id),
        }
    }

    /// Swap `clip` with its neighbour in `direction`, as one undo step.
    /// Returns the neighbour it swapped with.
    pub fn swap_with_neighbour(
        &mut self,
        clip: ClipId,
        direction: Neighbour,
    ) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let other = self
            .neighbour_of(clip, direction)
            .ok_or(EditorError::NoNeighbour)?;
        let (first, second) = match direction {
            Neighbour::Next => (clip, other),
            Neighbour::Previous => (other, clip),
        };
        let sequence = self
            .project()
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let span_of = |id: ClipId| sequence.clip_span(id).map(|span| span.timeline);
        let (a, b) = (
            span_of(first).ok_or(EditorError::ClipNotFound(first))?,
            span_of(second).ok_or(EditorError::ClipNotFound(second))?,
        );
        // The later clip to where the earlier began; the earlier to end where
        // the later ended. The gap between them keeps its size.
        let later_moves = a.start.ticks() - b.start.ticks();
        let earlier_moves = b.end.ticks() - a.end.ticks();
        // Somewhere past everything, clear of the whole edit, to wait in while
        // the other clip takes its place — a move onto a clip that has not left
        // yet would be refused.
        let parked = sequence.duration().ticks() + a.duration().ticks() - a.start.ticks();

        let group = |editor: &Self, id: ClipId| -> Vec<(TrackId, ClipId, TimelineTime)> {
            editor
                .linked_with(id)
                .into_iter()
                .filter_map(|member| {
                    let span = editor.project().active()?.clip_span(member)?;
                    Some((span.track, member, span.timeline.start))
                })
                .collect()
        };
        let earlier = group(self, first);
        let later = group(self, second);

        self.staged("Swap Clips", |editor, stage| {
            let parked_at = |start: TimelineTime| TimelineTime::from_ticks(start.ticks() + parked);
            // Park the earlier clip and its partners, bring the later one in,
            // then set the earlier one down behind it.
            editor.stage_moves_to(
                stage,
                sequence_id,
                earlier.iter().map(|(t, c, s)| (*t, *c, *s, parked_at(*s))),
            )?;
            editor.stage_moves_to(
                stage,
                sequence_id,
                later.iter().map(|(t, c, s)| {
                    (
                        *t,
                        *c,
                        *s,
                        TimelineTime::from_ticks(s.ticks() + later_moves),
                    )
                }),
            )?;
            editor.stage_moves_to(
                stage,
                sequence_id,
                earlier.iter().map(|(t, c, s)| {
                    (
                        *t,
                        *c,
                        parked_at(*s),
                        TimelineTime::from_ticks(s.ticks() + earlier_moves),
                    )
                }),
            )
        })
        .map_err(|err| match err {
            EditorError::Timeline(bettercut_timeline::TimelineError::ClipOverlap { .. })
            | EditorError::Timeline(bettercut_timeline::TimelineError::NegativePosition {
                ..
            }) => EditorError::NoRoomToSwap,
            other => other,
        })?;
        Ok(other)
    }

    /// Stage one move per clip, from where it is to `to`.
    fn stage_moves_to(
        &mut self,
        stage: &mut Stage,
        sequence: SequenceId,
        moves: impl Iterator<Item = (TrackId, ClipId, TimelineTime, TimelineTime)>,
    ) -> Result<(), EditorError> {
        for (track, clip, _from, to) in moves {
            self.stage(
                stage,
                Command::MoveClip {
                    sequence,
                    from_track: track,
                    to_track: track,
                    clip,
                    new_start: to,
                },
            )?;
        }
        Ok(())
    }
}
