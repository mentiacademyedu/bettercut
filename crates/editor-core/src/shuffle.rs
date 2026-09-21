//! Shuffle: selected clips on a lane put into a random order.
//!
//! For a montage — a pile of holiday shots dropped in in file order, which is
//! rarely the order they look best in. The clips keep the stretch of timeline
//! they covered together, and the gaps between them stay where they were;
//! only which shot sits in which place changes. Linked sound travels with its
//! picture (§12). One undo step, and when a partner would land on something
//! outside the shuffle nothing moves at all.

use std::collections::HashMap;

use bettercut_foundation::{ClipId, TimelineTime, TrackId};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Put the selected clips into a random order, as one undo step. Returns
    /// the clips in their new order.
    ///
    /// The shuffle is on one lane: the one holding most of the selection.
    /// Anything else selected must be tied to those clips (their sound).
    /// Refused unless at least two clips sit next to each other there, with
    /// nothing unselected between them. `seed` decides the order; the same
    /// seed on the same clips gives the same order, and the order is never the
    /// one they were already in.
    pub fn shuffle_clips(
        &mut self,
        selected: &[ClipId],
        seed: u64,
    ) -> Result<Vec<ClipId>, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .project()
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;

        // The lane with most of the selection.
        let mut per_track: HashMap<TrackId, Vec<ClipId>> = HashMap::new();
        for clip in selected {
            if let Some(span) = sequence.clip_span(*clip) {
                per_track.entry(span.track).or_default().push(*clip);
            }
        }
        let Some((track, clips)) = per_track
            .iter()
            .max_by_key(|(track, clips)| (clips.len(), sequence.video_track(**track).is_some()))
            .map(|(track, clips)| (*track, clips.clone()))
        else {
            return Err(EditorError::NothingToShuffle);
        };
        if clips.len() < 2 {
            return Err(EditorError::NothingToShuffle);
        }
        // Anything selected elsewhere has to be riding along with these.
        let riding: Vec<ClipId> = clips.iter().flat_map(|c| self.linked_with(*c)).collect();
        if selected.iter().any(|c| {
            sequence.clip_span(*c).is_some_and(|s| s.track != track) && !riding.contains(c)
        }) {
            return Err(EditorError::NothingToShuffle);
        }

        // In timeline order, and next to each other.
        let mut placed: Vec<(ClipId, TimelineTime, TimelineTime)> = clips
            .iter()
            .filter_map(|c| {
                let span = sequence.clip_span(*c)?;
                Some((*c, span.timeline.start, span.timeline.end))
            })
            .collect();
        placed.sort_by_key(|(_, start, _)| *start);
        let (first, last) = (placed[0].1, placed[placed.len() - 1].2);
        let between = sequence
            .clip_spans()
            .filter(|s| s.track == track && s.timeline.start < last && s.timeline.end > first)
            .count();
        if between != placed.len() {
            return Err(EditorError::NothingToShuffle);
        }

        let order = shuffled(placed.len(), seed);
        self.lay_out_in_order(&placed, &order, "Shuffle Clips")?;
        Ok(order.into_iter().map(|index| placed[index].0).collect())
    }

    /// Lay `placed` out in `order`, keeping the stretch of timeline they cover
    /// together and the gaps between them — only which clip sits in which
    /// place changes.
    ///
    /// `placed` is in timeline order, each entry a clip with its start and
    /// end; `order` says which of them goes in which slot. Linked sound
    /// travels with its picture (§12), and it is one undo step.
    ///
    /// Shared by the shuffle and the storyboard, because "put these clips in
    /// this order" is one operation however the order was arrived at — and the
    /// part that is easy to get wrong (setting a clip down where another has
    /// not left yet) should exist once.
    pub(crate) fn lay_out_in_order(
        &mut self,
        placed: &[(ClipId, TimelineTime, TimelineTime)],
        order: &[usize],
        label: &str,
    ) -> Result<(), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let gaps: Vec<i64> = placed
            .windows(2)
            .map(|pair| pair[1].1.ticks() - pair[0].2.ticks())
            .collect();
        let mut at = placed.first().map_or(0, |(_, start, _)| start.ticks());
        let mut shifts: Vec<(ClipId, i64)> = Vec::with_capacity(placed.len());
        for (slot, &index) in order.iter().enumerate() {
            let (clip, start, end) = placed[index];
            shifts.push((clip, at - start.ticks()));
            at += end.ticks() - start.ticks() + gaps.get(slot).copied().unwrap_or(0);
        }

        // Every clip and its partners, from where they are now.
        let mut moves: Vec<(TrackId, ClipId, i64, i64)> = Vec::new();
        for (clip, shift) in &shifts {
            for member in self.linked_with(*clip) {
                if let Some(span) = self.project().active().and_then(|s| s.clip_span(member)) {
                    moves.push((span.track, member, span.timeline.start.ticks(), *shift));
                }
            }
        }
        // Somewhere past the whole edit to wait in, so no clip is set down
        // onto one that has not left yet.
        let parked = self
            .active_sequence()
            .map_or(1, |sequence| sequence.duration().ticks().max(1));

        self.staged(label.to_owned(), |editor, stage| {
            for stage_to in [Some(parked), None] {
                for (track, clip, start, shift) in &moves {
                    let new_start = match stage_to {
                        Some(offset) => start + offset,
                        None => start + shift,
                    };
                    editor.stage(
                        stage,
                        Command::MoveClip {
                            sequence: sequence_id,
                            from_track: *track,
                            to_track: *track,
                            clip: *clip,
                            new_start: TimelineTime::from_ticks(new_start),
                        },
                    )?;
                }
            }
            Ok(())
        })
        .map_err(|err| match err {
            EditorError::Timeline(bettercut_timeline::TimelineError::ClipOverlap { .. })
            | EditorError::Timeline(bettercut_timeline::TimelineError::NegativePosition {
                ..
            }) => EditorError::NoRoomToShuffle,
            other => other,
        })
    }
}

/// A random order of `0..count` from `seed`, never the order it started in.
fn shuffled(count: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    // xorshift64*: small, and the same on every machine, so a seed means the
    // same order in a test as in the app.
    let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
    if state == 0 {
        state = 1;
    }
    let mut next = || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for i in (1..count).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    // A shuffle that changed nothing looks like a button that did nothing.
    if order.iter().enumerate().all(|(i, &v)| i == v) {
        order.rotate_left(1);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::shuffled;

    #[test]
    fn every_clip_appears_once_and_the_order_changes() {
        for count in 2..12 {
            for seed in 0..50 {
                let order = shuffled(count, seed);
                let mut sorted = order.clone();
                sorted.sort_unstable();
                assert_eq!(sorted, (0..count).collect::<Vec<_>>());
                assert!(order.iter().enumerate().any(|(i, &v)| i != v));
            }
        }
        assert_eq!(shuffled(5, 7), shuffled(5, 7));
    }
}
