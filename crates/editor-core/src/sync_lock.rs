//! Two questions about lanes, answered per track: where does a new clip land,
//! and which lanes move when a ripple edit moves one of them (§10).
//!
//! # Targeting
//!
//! An import, a paste and a voiceover all have to land somewhere, and until
//! now that somewhere was the first track of the right kind. With four picture
//! lanes and an overlay being built on the top one, the first is exactly where
//! the user does not want it. A track flagged as the target takes them
//! instead; with none flagged the first is still the answer, which is what
//! every project made before this did.
//!
//! One target per lane kind, kept exclusive when it is set, so "where does
//! this land" has one answer rather than four.
//!
//! # Sync lock
//!
//! A ripple edit — a ripple delete, a ripple trim, a gap closed — moves one
//! track's later clips left and leaves the other lanes where they are. That is
//! right for the music bed under the whole edit and wrong for the sound
//! recorded with the shot, and no rule about *kinds* of lane gets it right
//! either way. Sync lock is how the user says which is which: every other
//! track with the flag on has its later clips moved by the same amount at the
//! same instant, so what sat under the cut still sits under it.
//!
//! ## Whole clips only
//!
//! A clip the edit runs through, on a riding lane, stops the edit
//! ([`EditorError::SyncBlocked`]) instead of being cut through. Premiere cuts
//! it — which means an edit on the pictures quietly takes a piece out of a
//! lane the user was not looking at, and the way back is an undo they have to
//! notice they need. Refusing says what is in the way, and both ways out are
//! one click: take the sync lock off that lane, or cut it at the edit point
//! first.
//!
//! A locked lane never rides. Lock says "leave this alone", and riding along
//! with someone else's edit is not leaving it alone.

use bettercut_foundation::{ClipId, SequenceId, TimelineTime, TrackId};
use bettercut_timeline::{TimelineRange, TrackKind};

use crate::command::{Command, TrackFlag};
use crate::editor::{Editor, Stage};
use crate::error::EditorError;

impl Editor {
    // ---- targeting ----

    /// The track a new clip of `kind` lands on: the targeted one, or the first
    /// of its kind when none is targeted.
    pub fn target_track(&self, kind: TrackKind) -> Option<TrackId> {
        self.active_sequence()
            .and_then(|sequence| sequence.target_track(kind))
    }

    /// Make `track` the one new clips of its kind land on, or stop it being.
    ///
    /// Setting one takes the flag off the others in its lane, in the same undo
    /// step: two targeted picture lanes would be two answers to a question
    /// with one.
    pub fn set_target_track(&mut self, track: TrackId, targeted: bool) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let active = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence))?;
        let kind = active
            .track_kind(track)
            .ok_or(EditorError::TrackNotFound(track))?;

        let mut commands: Vec<Command> = Vec::new();
        if targeted {
            for other in active.tracks_of(kind) {
                if other != track && active.track_targeted(other) {
                    commands.push(Command::SetTrackFlag {
                        sequence,
                        track: other,
                        flag: TrackFlag::Targeted,
                        value: false,
                    });
                }
            }
        }
        commands.push(Command::SetTrackFlag {
            sequence,
            track,
            flag: TrackFlag::Targeted,
            value: targeted,
        });

        if commands.len() == 1 {
            return self.dispatch(commands.remove(0));
        }
        self.dispatch_group("Target Track".to_owned(), commands)
    }

    // ---- sync lock ----

    /// The lanes that ride along with a ripple on `edited`: every other track
    /// with sync lock on, unlocked.
    pub fn sync_riders(&self, edited: &[TrackId]) -> Vec<TrackId> {
        self.active_sequence().map_or_else(Vec::new, |sequence| {
            sequence
                .sync_locked_tracks()
                .into_iter()
                .filter(|track| {
                    !edited.contains(track) && !crate::gaps::track_locked(sequence, *track)
                })
                .collect()
        })
    }

    /// Close `range` on every riding lane, inside someone else's staged edit:
    /// everything from its end onwards moves left by its length, so it keeps
    /// the place it had against the track the edit was made on.
    ///
    /// Refused, changing nothing, when a riding lane has a clip across the
    /// range or nowhere for one to land. Returns how many clips moved.
    pub(crate) fn stage_sync_remove(
        &mut self,
        stage: &mut Stage,
        sequence: SequenceId,
        edited: &[TrackId],
        range: TimelineRange,
    ) -> Result<usize, EditorError> {
        let shift = range.duration().ticks();
        let riders = self.sync_riders(edited);
        if riders.is_empty() || shift <= 0 {
            return Ok(0);
        }
        let moves = self.sync_moves(&riders, range, shift)?;

        // Earliest first: nothing is moved into space its neighbour has not
        // left yet.
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

    /// What the riding lanes would move, and where to — checked against the
    /// timeline as it is now, earliest first.
    fn sync_moves(
        &self,
        riders: &[TrackId],
        range: TimelineRange,
        shift: i64,
    ) -> Result<Vec<(TrackId, ClipId, TimelineTime)>, EditorError> {
        let Some(sequence) = self.project().active() else {
            return Ok(Vec::new());
        };
        let spans: Vec<bettercut_timeline::ClipSpan> = sequence.clip_spans().collect();

        let mut moves: Vec<(TrackId, ClipId, TimelineTime)> = Vec::new();
        for span in spans.iter().filter(|span| riders.contains(&span.track)) {
            // Anything the edit runs through stops it: see the module note.
            if span.timeline.start < range.end && span.timeline.end > range.start {
                return Err(EditorError::SyncBlocked);
            }
            if span.timeline.start < range.end {
                continue;
            }
            let landed = TimelineTime::from_ticks(span.timeline.start.ticks() - shift);
            if landed.is_negative() {
                return Err(EditorError::SyncBlocked);
            }
            moves.push((span.track, span.clip, landed));
        }

        // And nothing may land on a clip that is staying put.
        let moving: Vec<ClipId> = moves.iter().map(|(_, clip, _)| *clip).collect();
        for (track, clip, start) in &moves {
            let Some(span) = spans.iter().find(|span| span.clip == *clip) else {
                continue;
            };
            let landed = TimelineRange {
                start: *start,
                end: TimelineTime::from_ticks(start.ticks() + span.timeline.duration().ticks()),
            };
            let blocked = spans.iter().any(|other| {
                other.track == *track
                    && !moving.contains(&other.clip)
                    && other.timeline.overlaps(landed)
            });
            if blocked {
                return Err(EditorError::SyncBlocked);
            }
        }

        moves.sort_by_key(|(_, _, start)| start.ticks());
        Ok(moves)
    }
}
