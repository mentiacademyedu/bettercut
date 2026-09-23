//! Slide edit: a clip moves along the timeline and its neighbours give way.
//!
//! The last of the four (§10): a trim moves one edge, a roll moves a cut, a
//! slip changes what a clip plays, and a slide moves *where* it plays while
//! everything around it stays put. The clip keeps its length and its content;
//! the clip before it grows or shrinks by as much as the clip after it shrinks
//! or grows, so nothing later on the track moves at all.
//!
//! What it is for: the cutaway that lands two frames late. Dragging it would
//! leave a hole behind and overlap what follows; sliding it just moves it.

use bettercut_foundation::{ClipId, TimelineTime, TrackId, ticks_per_frame};

use crate::command::{Command, TrimEdge};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// How far `clip` can slide, in ticks: the most it can move earlier (zero
    /// or negative) and later (zero or positive).
    ///
    /// Bounded by what its neighbours can give: a clip before it that has run
    /// out of file cannot grow, and one that is a frame long cannot shrink.
    /// `None` when the clip is not on the timeline.
    pub fn slide_room(&self, clip: ClipId) -> Option<(i64, i64)> {
        let sequence = self.project().active()?;
        let span = sequence.clip_span(clip)?;
        let frame = ticks_per_frame(sequence.frame_rate).unwrap_or(1).max(1);
        let before = self.roll_partner(clip, TrimEdge::Start);
        let after = self.roll_partner(clip, TrimEdge::End);

        let mut earliest = i64::MIN / 4;
        let mut latest = i64::MAX / 4;
        match before {
            // The clip before gives up time when this one slides earlier, and
            // takes it back when it slides later.
            Some(previous) => {
                let previous_span = sequence.clip_span(previous)?.timeline;
                // It must keep a frame of itself.
                earliest =
                    earliest.max(previous_span.start.ticks() + frame - span.timeline.start.ticks());
                if let Some(room) = self.growth_room(previous, TrimEdge::End) {
                    latest = latest.min(room);
                }
            }
            // Nothing there: the clip may slide back only as far as the start
            // of the timeline, and leaves a gap behind it either way.
            None => earliest = earliest.max(-span.timeline.start.ticks()),
        }
        // Nothing after it: it can slide later as far as it likes, and leaves
        // a gap behind rather than pushing anything.
        if let Some(next) = after {
            let next_span = sequence.clip_span(next)?.timeline;
            latest = latest.min(next_span.end.ticks() - frame - span.timeline.end.ticks());
            if let Some(room) = self.growth_room(next, TrimEdge::Start) {
                earliest = earliest.max(-room);
            }
        }
        if before.is_none() && after.is_none() {
            return Some((0, 0));
        }
        Some((earliest.min(0), latest.max(0)))
    }

    /// Slide `clip` by `by` ticks (later when positive), held to
    /// [`Self::slide_room`] and the frame grid. One undo step.
    ///
    /// Nothing happens, and no undo step is made, when that leaves it where it
    /// was.
    pub fn slide_clip(&mut self, clip: ClipId, by: i64) -> Result<i64, EditorError> {
        let sequence = self.active_sequence_id()?;
        let (earliest, latest) = self
            .slide_room(clip)
            .ok_or(EditorError::ClipNotFound(clip))?;
        let frame = self
            .project()
            .sequence(sequence)
            .and_then(|s| ticks_per_frame(s.frame_rate))
            .unwrap_or(1)
            .max(1);
        // On the frame grid, and inward — a slide that rounded outward would
        // be refused by the trim it asks for.
        let by = (by / frame) * frame;
        let by = by.clamp(earliest, latest);
        if by == 0 {
            return Ok(0);
        }
        let before = self.roll_partner(clip, TrimEdge::Start);
        let after = self.roll_partner(clip, TrimEdge::End);
        if before.is_none() && after.is_none() {
            return Err(EditorError::NoNeighbour);
        }

        let span = self
            .project()
            .sequence(sequence)
            .and_then(|s| s.clip_span(clip))
            .ok_or(EditorError::ClipNotFound(clip))?
            .timeline;
        let moves = self.slide_moves(sequence, clip, by);
        let shrink_before = before.map(|previous| {
            self.edge_commands(sequence, previous, TrimEdge::End, span.start.ticks() + by)
        });
        let shrink_after = after
            .map(|next| self.edge_commands(sequence, next, TrimEdge::Start, span.end.ticks() + by));

        // Whichever neighbour is giving up time goes first, so the clip never
        // has to pass through a place something else is still in.
        let (first, last) = if by > 0 {
            (shrink_after, shrink_before)
        } else {
            (shrink_before, shrink_after)
        };

        self.staged("Slide Clip", |editor, stage| {
            for command in first.into_iter().flatten() {
                editor.stage(stage, command)?;
            }
            for command in moves {
                editor.stage(stage, command)?;
            }
            for command in last.into_iter().flatten() {
                editor.stage(stage, command)?;
            }
            Ok(())
        })?;
        Ok(by)
    }

    /// How much timeline a clip could grow by at one edge before it runs out
    /// of file. `None` for no limit — a photo, a held frame, a title.
    pub(crate) fn growth_room(&self, clip: ClipId, edge: TrimEdge) -> Option<i64> {
        let (earlier, later) = self.slip_room(clip)?;
        // Slipping later means there is material after the out-point, which is
        // exactly what growing at the end needs; and the mirror for the start.
        let source_room = match edge {
            TrimEdge::End => later,
            TrimEdge::Start => -earlier,
        };
        let (timeline, source) = self
            .video_clip(clip)
            .map(|c| (c.timeline, c.source))
            .or_else(|| self.audio_clip(clip).map(|c| (c.timeline, c.source)))?;
        let on_timeline = timeline.duration().ticks().max(1);
        let source_length = source.duration().ticks().max(1);
        Some(
            i64::try_from(
                i128::from(source_room) * i128::from(on_timeline) / i128::from(source_length),
            )
            .unwrap_or(i64::MAX / 4),
        )
    }

    /// The clip and everything linked to it, each moved by `by`.
    fn slide_moves(
        &self,
        sequence: bettercut_foundation::SequenceId,
        clip: ClipId,
        by: i64,
    ) -> Vec<Command> {
        let mut moving = vec![clip];
        moving.extend(self.linked_with(clip).into_iter().filter(|c| *c != clip));
        moving
            .into_iter()
            .filter_map(|member| {
                let track: TrackId = self.track_of(member)?;
                let start = self.project().active()?.clip_span(member)?.timeline.start;
                Some(Command::MoveClip {
                    sequence,
                    from_track: track,
                    to_track: track,
                    clip: member,
                    new_start: TimelineTime::from_ticks(start.ticks() + by),
                })
            })
            .collect()
    }

    /// Trim one edge of a clip and of everything linked to it, each by the
    /// same amount — `to` is where the named clip's edge lands.
    fn edge_commands(
        &self,
        sequence: bettercut_foundation::SequenceId,
        clip: ClipId,
        edge: TrimEdge,
        to: i64,
    ) -> Vec<Command> {
        let Some(span) = self
            .project()
            .active()
            .and_then(|active| active.clip_span(clip))
        else {
            return Vec::new();
        };
        let at = match edge {
            TrimEdge::Start => span.timeline.start.ticks(),
            TrimEdge::End => span.timeline.end.ticks(),
        };
        let delta = to - at;
        let mut clips = vec![clip];
        clips.extend(self.linked_with(clip).into_iter().filter(|c| *c != clip));
        clips
            .into_iter()
            .filter_map(|member| {
                let track = self.track_of(member)?;
                let span = self.project().active()?.clip_span(member)?.timeline;
                let edge_at = match edge {
                    TrimEdge::Start => span.start,
                    TrimEdge::End => span.end,
                };
                Some(Command::TrimClip {
                    sequence,
                    track,
                    clip: member,
                    edge,
                    to: TimelineTime::from_ticks(edge_at.ticks() + delta),
                })
            })
            .collect()
    }
}
