//! Closing gaps: pulling a track's later clips left over empty space.
//!
//! # One track, and whatever is tied to it
//!
//! A gap belongs to one track — the empty stretch between two clips, or before
//! the first one. Closing it moves that track's later clips left by its length,
//! the same per-track ripple a Shift+Delete makes (§10), and leaves the other
//! tracks where they are: music under a cut should not jump because a gap in
//! the pictures was closed.
//!
//! The exception is a clip's own sound (§12). A picture pulled left without the
//! sound recorded with it would be out of sync from there on, so linked partners
//! move by the same amount, on whatever track they are. When one of them has
//! no room to move — something unrelated sits where it would land — the gap is
//! left open rather than half-closed: [`EditorError::GapBlocked`].
//!
//! Markers stay put. They mark instants on the whole edit, and closing a gap
//! on one track does not move the others.

use bettercut_foundation::{ClipId, SequenceId, TimelineTime, TrackId};
use bettercut_timeline::{Sequence, TimelineRange};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

/// Every gap on `track`, earliest first.
///
/// The space before the first clip counts; the space after the last does not —
/// that is not a gap, just where the track ends. A stretch shorter than a frame
/// is not reported, since there is nothing a frame-snapped move could close.
pub fn gaps_on(sequence: &Sequence, track: TrackId) -> Vec<TimelineRange> {
    let mut spans: Vec<TimelineRange> = sequence
        .clip_spans()
        .filter(|span| span.track == track)
        .map(|span| span.timeline)
        .collect();
    spans.sort_by_key(|span| span.start);

    let mut gaps = Vec::new();
    let mut end = TimelineTime::ZERO;
    for span in spans {
        let start = closed_start(sequence, end);
        if span.start > start {
            gaps.push(TimelineRange {
                start: end,
                end: span.start,
            });
        }
        end = end.max(span.end);
    }
    gaps
}

/// Where the first clip after a gap ending at `end` lands once it is closed: on
/// the first frame boundary at or after `end`, since moves snap to frames
/// (§76) and landing on the frame before would overlap the clip in front.
fn closed_start(sequence: &Sequence, end: TimelineTime) -> TimelineTime {
    let snapped = sequence.snap_to_frame(end);
    if snapped < end {
        snapped + TimelineTime::from_ticks(sequence.ticks_per_frame())
    } else {
        snapped
    }
}

pub(crate) fn track_locked(sequence: &Sequence, track: TrackId) -> bool {
    sequence
        .video_tracks
        .iter()
        .find(|t| t.id == track)
        .map(|t| t.locked)
        .or_else(|| {
            sequence
                .audio_tracks
                .iter()
                .find(|t| t.id == track)
                .map(|t| t.locked)
        })
        .or_else(|| {
            sequence
                .text_tracks
                .iter()
                .find(|t| t.id == track)
                .map(|t| t.locked)
        })
        .or_else(|| {
            sequence
                .adjustment_tracks
                .iter()
                .find(|t| t.id == track)
                .map(|t| t.locked)
        })
        .unwrap_or(false)
}

impl Editor {
    /// The gap on `track` that `at` falls in, if it falls in one.
    pub fn gap_at(&self, track: TrackId, at: TimelineTime) -> Option<TimelineRange> {
        let sequence = self.project().active()?;
        gaps_on(sequence, track)
            .into_iter()
            .find(|gap| at >= gap.start && at < gap.end)
    }

    /// Every gap on `track`, earliest first. See [`gaps_on`].
    pub fn gaps_on(&self, track: TrackId) -> Vec<TimelineRange> {
        self.project()
            .active()
            .map_or_else(Vec::new, |sequence| gaps_on(sequence, track))
    }

    /// Close the gap on `track` under `at`, as one undo step. Returns how many
    /// clips moved, counting linked partners on other tracks.
    ///
    /// Refused, changing nothing, when there is no gap there, or when closing
    /// it would push a linked partner into another clip.
    pub fn close_gap(&mut self, track: TrackId, at: TimelineTime) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let gap = self.gap_at(track, at).ok_or(EditorError::NoGapThere)?;
        let moves = self.gap_moves(track, gap)?;
        self.staged("Close Gap", |editor, stage| {
            let moved = editor.stage_moves(stage, sequence, moves)?;
            editor.stage_markers_closed(stage, sequence, gap)?;
            Ok(moved)
        })
    }

    /// Close every gap on `track`, as one undo step. Returns how many closed.
    ///
    /// A gap whose linked partners have no room is skipped rather than stopping
    /// the rest, so one awkward stretch does not leave the whole track open.
    pub fn close_all_gaps(&mut self, track: TrackId) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        if self.gaps_on(track).is_empty() {
            return Err(EditorError::NoGapThere);
        }
        let closed = self.staged("Close All Gaps", |editor, stage| {
            let mut closed = 0;
            // Latest first: closing a gap moves only what comes after it, so
            // the earlier gaps are still where they were found.
            let mut gaps = editor.gaps_on(track);
            gaps.reverse();
            for gap in gaps {
                if let Ok(moves) = editor.gap_moves(track, gap) {
                    editor.stage_moves(stage, sequence, moves)?;
                    editor.stage_markers_closed(stage, sequence, gap)?;
                    closed += 1;
                }
            }
            Ok(closed)
        })?;
        if closed == 0 {
            return Err(EditorError::GapBlocked);
        }
        Ok(closed)
    }

    /// What closing `gap` on `track` would move, and where to — checked
    /// against the timeline as it is now.
    fn gap_moves(
        &self,
        track: TrackId,
        gap: TimelineRange,
    ) -> Result<Vec<(TrackId, ClipId, TimelineTime)>, EditorError> {
        let sequence = self
            .project()
            .active()
            .ok_or_else(|| EditorError::SequenceNotFound(SequenceId::new()))?;
        let shift = gap.end.ticks() - closed_start(sequence, gap.start).ticks();
        if shift <= 0 {
            return Err(EditorError::NoGapThere);
        }

        let spans: Vec<_> = sequence.clip_spans().collect();
        // The track's own later clips, and the later clips of every lane
        // riding along with it (§10's sync lock).
        let riders = self.sync_riders(&[track]);
        let mut moving: Vec<ClipId> = Vec::new();
        for span in spans.iter().filter(|span| {
            span.timeline.start >= gap.end && (span.track == track || riders.contains(&span.track))
        }) {
            for clip in self.linked_with(span.clip) {
                if !moving.contains(&clip) {
                    moving.push(clip);
                }
            }
        }

        let mut moves = Vec::with_capacity(moving.len());
        for span in spans.iter().filter(|span| moving.contains(&span.clip)) {
            let landed = TimelineRange {
                start: TimelineTime::from_ticks(span.timeline.start.ticks() - shift),
                end: TimelineTime::from_ticks(span.timeline.end.ticks() - shift),
            };
            let collides = spans.iter().any(|other| {
                other.track == span.track
                    && !moving.contains(&other.clip)
                    && other.timeline.overlaps(landed)
            });
            if landed.start.is_negative() || collides || track_locked(sequence, span.track) {
                return Err(EditorError::GapBlocked);
            }
            moves.push((span.track, span.clip, landed.start));
        }
        Ok(moves)
    }

    /// Stage the moves, earliest first on each track so nothing is moved into
    /// space its neighbour has not vacated yet.
    fn stage_moves(
        &mut self,
        stage: &mut crate::editor::Stage,
        sequence: SequenceId,
        mut moves: Vec<(TrackId, ClipId, TimelineTime)>,
    ) -> Result<usize, EditorError> {
        moves.sort_by_key(|(_, _, start)| *start);
        let count = moves.len();
        for (track, clip, new_start) in moves {
            self.stage(
                stage,
                Command::MoveClip {
                    sequence,
                    from_track: track,
                    to_track: track,
                    clip,
                    new_start,
                },
            )?;
        }
        Ok(count)
    }
}

impl Editor {
    /// Take out every picture and sound clip shorter than `frames` frames,
    /// and close every gap on those lanes shorter than that — the flash
    /// frames and blinks of black a fast edit leaves behind. Locked lanes
    /// are left alone. One undo step. Returns (clips removed, gaps closed).
    pub fn clean_up_slivers(&mut self, frames: u32) -> Result<(usize, usize), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let rate = sequence.frame_rate.as_rational();
        let limit = bettercut_foundation::TICKS_PER_SECOND * rate.den() * i64::from(frames.max(1))
            / rate.num().max(1);
        let lanes: Vec<TrackId> = sequence
            .video_tracks
            .iter()
            .map(|t| t.id)
            .chain(sequence.audio_tracks.iter().map(|t| t.id))
            .filter(|t| !self.track_flag(*t, crate::command::TrackFlag::Locked))
            .collect();
        let slivers: Vec<(TrackId, ClipId)> = sequence
            .clip_spans()
            .filter(|s| lanes.contains(&s.track))
            .filter(|s| (s.timeline.end - s.timeline.start).ticks() < limit)
            .map(|s| (s.track, s.clip))
            .collect();
        let short = |editor: &Self, track: TrackId| -> Vec<TimelineRange> {
            editor
                .gaps_on(track)
                .into_iter()
                .filter(|g| (g.end - g.start).ticks() < limit)
                .collect()
        };
        if slivers.is_empty() && lanes.iter().all(|t| short(self, *t).is_empty()) {
            return Ok((0, 0));
        }
        self.staged("Clean Up Slivers", |editor, stage| {
            for (track, clip) in &slivers {
                editor.stage(
                    stage,
                    Command::RemoveClip {
                        sequence: sequence_id,
                        track: *track,
                        clip: *clip,
                    },
                )?;
            }
            let mut closed = 0;
            for track in &lanes {
                // Latest first, as in `close_all_gaps`; and read again per
                // lane, since closing one moves linked clips on others.
                let mut gaps = short(editor, *track);
                gaps.reverse();
                for gap in gaps {
                    if let Ok(moves) = editor.gap_moves(*track, gap) {
                        editor.stage_moves(stage, sequence_id, moves)?;
                        editor.stage_markers_closed(stage, sequence_id, gap)?;
                        closed += 1;
                    }
                }
            }
            Ok((slivers.len(), closed))
        })
    }
}

impl Editor {
    /// Close every gap on every unlocked picture and sound lane, pictures
    /// first so their linked sound comes along. Gaps whose linked partners
    /// have no room stay open. One undo step. Returns how many closed.
    pub fn close_gaps_everywhere(&mut self) -> Result<usize, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let lanes: Vec<TrackId> = sequence
            .video_tracks
            .iter()
            .map(|t| t.id)
            .chain(sequence.audio_tracks.iter().map(|t| t.id))
            .filter(|t| !self.track_flag(*t, crate::command::TrackFlag::Locked))
            .collect();
        if lanes.iter().all(|t| self.gaps_on(*t).is_empty()) {
            return Err(EditorError::NoGapThere);
        }
        let closed = self.staged("Close Gaps Everywhere", |editor, stage| {
            let mut closed = 0;
            for track in &lanes {
                // Read per lane: closing a picture gap moves its sound.
                let mut gaps = editor.gaps_on(*track);
                gaps.reverse();
                for gap in gaps {
                    if let Ok(moves) = editor.gap_moves(*track, gap) {
                        editor.stage_moves(stage, sequence_id, moves)?;
                        editor.stage_markers_closed(stage, sequence_id, gap)?;
                        closed += 1;
                    }
                }
            }
            Ok(closed)
        })?;
        if closed == 0 {
            return Err(EditorError::GapBlocked);
        }
        Ok(closed)
    }
}
