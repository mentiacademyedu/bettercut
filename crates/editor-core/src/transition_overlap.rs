//! A transition between two clips with no footage to spare, made the way
//! CapCut makes it: the clips overlap. Each gives up the half of the
//! transition it has no footage for — the outgoing clip's end, the incoming
//! clip's start — and what follows closes up, so the edit is that much
//! shorter. Without this, two clips dropped in whole (the usual way) could
//! never have a transition between them: neither has anything past its edge.

use bettercut_foundation::{ClipId, TimelineTime};
use bettercut_timeline::{Transition, TransitionKind};

use crate::command::{Command, TrimEdge};
use crate::editor::{Editor, Stage};
use crate::error::EditorError;
use crate::ops;

/// What overlapping would take: each side's trim, in timeline ticks.
struct OverlapPlan {
    track: bettercut_foundation::TrackId,
    cut: ops::CutHandles,
    need_after: i64,
    need_before: i64,
}

impl Editor {
    /// How much shorter the edit would get to put `kind` on the cut after
    /// `clip` at the project's transition length, when the clips have no
    /// footage to spare and must overlap. `None` when no overlap is needed
    /// (there is footage enough) or none would work (no cut, or clips too
    /// short) — ask [`Self::transition_room`] which.
    pub fn transition_overlap(&self, clip: ClipId, kind: TransitionKind) -> Option<TimelineTime> {
        if self
            .transition_room(clip, kind)
            .is_none_or(|room| room >= bettercut_timeline::MIN_TRANSITION)
        {
            return None;
        }
        let duration = self.transition_length_for(clip);
        self.overlap_plan(clip, kind, duration)
            .ok()
            .map(|plan| TimelineTime::from_ticks(plan.need_after + plan.need_before))
    }

    /// The length a transition on `clip` gets: the one already there, or the
    /// project's.
    pub(crate) fn transition_length_for(&self, clip: ClipId) -> TimelineTime {
        self.video_clip(clip)
            .and_then(|c| c.transition_out)
            .map(|t| t.duration)
            .unwrap_or(self.project().settings.transition_length)
    }

    /// Put `kind` on the cut after `clip` at `duration`, trimming each side
    /// by what it lacks and closing the gap, as one undo step. Returns how
    /// much shorter the edit got.
    ///
    /// Refused when there is no cut there, or when a clip is too short to
    /// give up its share and still outlast the transition.
    pub fn overlap_into_transition(
        &mut self,
        clip: ClipId,
        kind: TransitionKind,
        duration: TimelineTime,
    ) -> Result<TimelineTime, EditorError> {
        let sequence = self.active_sequence_id()?;
        let OverlapPlan {
            track,
            cut,
            need_after,
            need_before,
        } = self.overlap_plan(clip, kind, duration)?;

        let label = format!("{} Transition", kind.label());
        self.staged(label, |editor, stage| {
            if need_after > 0 {
                editor.stage_linked_trim(stage, cut.outgoing, TrimEdge::End, -need_after)?;
            }
            if need_before > 0 {
                editor.stage_linked_trim(stage, cut.incoming, TrimEdge::Start, need_before)?;
            }
            if need_after + need_before > 0 {
                let end = editor
                    .clip_end(cut.outgoing)
                    .ok_or(EditorError::ClipNotFound(cut.outgoing))?;
                let gap = editor.gap_at(track, end).ok_or(EditorError::NoGapThere)?;
                let moves = editor.gap_moves(track, gap)?;
                editor.stage_moves(stage, sequence, moves)?;
                editor.stage_markers_closed(stage, sequence, gap)?;
            }
            editor.stage(
                stage,
                Command::SetTransition {
                    sequence,
                    track,
                    clip,
                    transition: Some(Transition::new(kind, duration)),
                },
            )
        })?;
        Ok(TimelineTime::from_ticks(need_after + need_before))
    }

    fn overlap_plan(
        &self,
        clip: ClipId,
        kind: TransitionKind,
        duration: TimelineTime,
    ) -> Result<OverlapPlan, EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let cut = ops::cut_handles(self.project(), sequence, track, clip)
            .ok_or(EditorError::NoRoomForTransition)?;
        let frame = self
            .active_sequence()
            .and_then(|s| bettercut_foundation::ticks_per_frame(s.frame_rate))
            .unwrap_or(1)
            .max(1);
        // Whole frames, rounded up, so the cut lands on a frame and the
        // footage freed is never a sliver short of the half it is for.
        let whole_frames = |ticks: i64| (ticks.max(0) + frame - 1) / frame * frame;
        let half = duration.ticks() / 2;
        let need_after = if kind.needs_handles() {
            whole_frames(half - cut.after.ticks())
        } else {
            0
        };
        let need_before = if kind.needs_handles() {
            whole_frames(half - cut.before.ticks())
        } else {
            0
        };
        if cut.outgoing_length.ticks() - need_after < duration.ticks()
            || cut.incoming_length.ticks() - need_before < duration.ticks()
        {
            return Err(EditorError::NoRoomForTransition);
        }
        Ok(OverlapPlan {
            track,
            cut,
            need_after,
            need_before,
        })
    }

    /// Move one edge of `clip` by `delta`, and the same edge of what it is
    /// linked to, inside a staged step — `trim_clip`, staged.
    fn stage_linked_trim(
        &mut self,
        stage: &mut Stage,
        clip: ClipId,
        edge: TrimEdge,
        delta: i64,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        for member in self.linked_with(clip) {
            let track = self
                .track_of(member)
                .ok_or(EditorError::ClipNotFound(member))?;
            let at = match edge {
                TrimEdge::Start => self.clip_start(member),
                TrimEdge::End => self.clip_end(member),
            }
            .ok_or(EditorError::ClipNotFound(member))?;
            self.stage(
                stage,
                Command::TrimClip {
                    sequence,
                    track,
                    clip: member,
                    edge,
                    to: TimelineTime::from_ticks(at.ticks() + delta),
                },
            )?;
        }
        Ok(())
    }
}
