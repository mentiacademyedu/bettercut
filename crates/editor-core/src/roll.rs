//! Roll edit: move the cut between two clips that touch, lengthening one as
//! the other shortens, so nothing before or after the pair moves.
//!
//! Both edges are ordinary trims, run in the order that never makes the two
//! clips overlap — the one giving up time first — and staged as one undo step.
//! Linked sound or picture rolls with each clip by the same amount (§12).

use bettercut_foundation::{ClipId, TimelineTime, ticks_per_frame};
use bettercut_timeline::{SourceRange, TimelineRange};

use crate::command::{Command, TrimEdge};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// The clip that touches `clip` at `edge` on the same track — the other
    /// side of a cut that can be rolled. A gap between them is not a cut.
    pub fn roll_partner(&self, clip: ClipId, edge: TrimEdge) -> Option<ClipId> {
        let sequence = self.project().active()?;
        let span = sequence.clip_span(clip)?;
        sequence
            .clip_spans()
            .filter(|other| other.track == span.track && other.clip != clip)
            .find(|other| match edge {
                TrimEdge::End => other.timeline.start == span.timeline.end,
                TrimEdge::Start => other.timeline.end == span.timeline.start,
            })
            .map(|other| other.clip)
    }

    /// The earliest and latest the cut after `left` can be rolled to: each
    /// clip keeps at least a frame, and neither reaches past its file.
    ///
    /// `None` when nothing touches `left`'s end.
    pub fn roll_room(&self, left: ClipId) -> Option<(TimelineTime, TimelineTime)> {
        let right = self.roll_partner(left, TrimEdge::End)?;
        let sequence = self.project().active()?;
        let frame = ticks_per_frame(sequence.frame_rate).unwrap_or(1).max(1);
        let (left_timeline, left_later, _) = self.edge_room(left)?;
        let (right_timeline, _, right_earlier) = self.edge_room(right)?;
        let cut = left_timeline.end.ticks();

        let mut earliest = left_timeline.start.ticks() + frame;
        if let Some(room) = right_earlier {
            earliest = earliest.max(cut - room);
        }
        let mut latest = right_timeline.end.ticks() - frame;
        if let Some(room) = left_later {
            latest = latest.min(cut + room);
        }
        // A clip a frame long leaves no room either way.
        if earliest > cut {
            earliest = cut;
        }
        if latest < cut {
            latest = cut;
        }
        Some((
            TimelineTime::from_ticks(earliest),
            TimelineTime::from_ticks(latest),
        ))
    }

    /// Roll the cut between `left` and the clip that follows it to `to`, held
    /// to [`Self::roll_room`] and a frame boundary. Nothing happens, and no
    /// undo step is made, when that leaves the cut where it was.
    pub fn roll_edit(&mut self, left: ClipId, to: TimelineTime) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let right = self
            .roll_partner(left, TrimEdge::End)
            .ok_or(EditorError::NoNeighbour)?;
        let (earliest, latest) = self.roll_room(left).ok_or(EditorError::NoNeighbour)?;
        let cut = self
            .clip_end_of(left)
            .ok_or(EditorError::ClipNotFound(left))?;

        let frame_rate = self
            .project()
            .sequence(sequence)
            .ok_or(EditorError::SequenceNotFound(sequence))?
            .frame_rate;
        let mut to = to.clamp(earliest, latest);
        to = to.snap_to_frame(frame_rate).unwrap_or(to);
        // Snapping can step just past a limit that isn't on a frame; step back.
        if let Some(frame) = ticks_per_frame(frame_rate) {
            while to > latest && to.ticks() - frame >= cut.ticks().min(latest.ticks()) {
                to = TimelineTime::from_ticks(to.ticks() - frame);
            }
            while to < earliest && to.ticks() + frame <= cut.ticks().max(earliest.ticks()) {
                to = TimelineTime::from_ticks(to.ticks() + frame);
            }
        }
        if to == cut {
            return Ok(());
        }
        let delta = to.ticks() - cut.ticks();

        // The clip and its linked partners, each with the edge being trimmed
        // and where that edge ends up.
        let trims_for = |editor: &Self, clip: ClipId, edge: TrimEdge| -> Vec<Command> {
            let mut clips = vec![clip];
            clips.extend(editor.linked_with(clip).into_iter().filter(|c| *c != clip));
            clips
                .into_iter()
                .filter_map(|c| {
                    let track = editor.track_of(c)?;
                    let span = editor.project().active()?.clip_span(c)?.timeline;
                    let at = match edge {
                        TrimEdge::Start => span.start,
                        TrimEdge::End => span.end,
                    };
                    Some(Command::TrimClip {
                        sequence,
                        track,
                        clip: c,
                        edge,
                        to: TimelineTime::from_ticks(at.ticks() + delta),
                    })
                })
                .collect()
        };
        let shortens_left = trims_for(self, left, TrimEdge::End);
        let shortens_right = trims_for(self, right, TrimEdge::Start);
        // Whichever clip is giving up time goes first, so the other never
        // grows into it.
        let ordered: Vec<Command> = if delta > 0 {
            shortens_right.into_iter().chain(shortens_left).collect()
        } else {
            shortens_left.into_iter().chain(shortens_right).collect()
        };

        self.staged("Roll Edit", |editor, stage| {
            for command in ordered {
                editor.stage(stage, command)?;
            }
            Ok(())
        })
    }

    fn clip_end_of(&self, clip: ClipId) -> Option<TimelineTime> {
        Some(self.project().active()?.clip_span(clip)?.timeline.end)
    }

    /// A clip's span, and how much timeline it could grow by at its end and at
    /// its start before running out of file — `None` for no limit (a photo, a
    /// held frame, a title).
    fn edge_room(&self, clip: ClipId) -> Option<(TimelineRange, Option<i64>, Option<i64>)> {
        let sequence = self.project().active()?;
        let span = sequence.clip_span(clip)?;
        let footage = self
            .video_clip(clip)
            .map(|c| (c.media_id, c.source, c.reversed, c.frozen))
            .or_else(|| {
                self.audio_clip(clip)
                    .map(|c| (c.media_id, c.source, c.reversed, false))
            });
        // A held frame holds one instant however long it runs.
        let Some((media, source, reversed, false)) = footage else {
            return Some((span.timeline, None, None));
        };
        let limit = self
            .project()
            .media_asset(media)
            .and_then(|asset| asset.source_limit());
        let SourceRange { start, end } = source;
        // Source ticks left past each end of the clip.
        let after_out = limit.map(|limit| (limit.ticks() - end.ticks()).max(0));
        let before_in = Some(start.ticks().max(0));
        let (at_end, at_start) = if reversed {
            (before_in, after_out)
        } else {
            (after_out, before_in)
        };
        // Source ticks to timeline ticks, at the clip's speed.
        let on_timeline = span.timeline.duration().ticks().max(1);
        let source_length = source.duration().ticks().max(1);
        let to_timeline = |ticks: i64| {
            i64::try_from(i128::from(ticks) * i128::from(on_timeline) / i128::from(source_length))
                .unwrap_or(i64::MAX)
        };
        Some((
            span.timeline,
            at_end.map(to_timeline),
            at_start.map(to_timeline),
        ))
    }
}
