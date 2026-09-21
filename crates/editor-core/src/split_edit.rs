//! Split edits: the J and the L cut.
//!
//! A straight cut changes picture and sound at the same instant, which is the
//! one thing real scenes never do. The sound of the next shot arriving a
//! moment early carries the cut (a J); the sound of this one hanging over the
//! next carries the thought (an L). Every drama and every interview is built
//! out of them, and they are the difference between a cut that reads as a cut
//! and one nobody notices.
//!
//! # The exception to §12
//!
//! Linked picture and sound trim together — that is the rule, and it is what
//! keeps a shot in sync while it is being cut. A split edit is the deliberate
//! exception: one edge of one half of the pair moves, and the other half stays
//! exactly where it is. Nothing else about the link changes, so the pair still
//! moves together, still splits together, and still ripples together
//! afterwards; only this one edge is out of step, which is the whole point.
//!
//! Held to the same limits as any other trim: a sound edge cannot be pulled
//! past the end of its file, and cannot be pushed into the clip beside it.
//! Both refusals come from the trim itself, so a split edit cannot make a
//! timeline an ordinary trim could not.

use bettercut_foundation::{ClipId, TimelineTime};

use crate::command::{Command, TrimEdge};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Trim one edge of `clip` alone, leaving its linked partner where it is.
    ///
    /// One undo step, named for the cut it makes: leading a sound in early is
    /// a J cut, letting it hang over is an L cut, and pulling it back the
    /// other way is neither — it is straightening the cut it was.
    pub fn split_edit(
        &mut self,
        clip: ClipId,
        edge: TrimEdge,
        to: TimelineTime,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let span = self
            .active_sequence()
            .and_then(|active| active.clip_span(clip))
            .ok_or(EditorError::ClipNotFound(clip))?
            .timeline;
        let to = self.snap_to_frame(sequence, to);
        let was = match edge {
            TrimEdge::Start => span.start,
            TrimEdge::End => span.end,
        };
        if to == was {
            return Ok(());
        }

        // Which way it went, in the words the undo menu should use.
        let label = match (edge, to < was) {
            (TrimEdge::Start, true) => "J Cut",
            (TrimEdge::End, false) => "L Cut",
            _ => "Straighten Cut",
        };
        self.dispatch_group(
            label.to_owned(),
            vec![Command::TrimClip {
                sequence,
                track,
                clip,
                edge,
                to,
            }],
        )
    }

    /// Lead this pair's sound in by `by` before its picture (`TrimEdge::Start`)
    /// or let it hang over by `by` after (`TrimEdge::End`).
    ///
    /// Asked of either half of the pair — a user right-clicks the picture as
    /// often as the sound — and always moves the sound, because moving the
    /// picture instead would be a different edit with the same name.
    /// A negative `by` pulls the sound back towards the picture.
    ///
    /// Refused, changing nothing, when the clip has no sound linked to it: a
    /// shot with no sound has no split edit to make.
    pub fn lead_sound(
        &mut self,
        clip: ClipId,
        edge: TrimEdge,
        by: TimelineTime,
    ) -> Result<(), EditorError> {
        let sound = self.sound_of(clip).ok_or(EditorError::NothingLinked)?;
        let span = self
            .active_sequence()
            .and_then(|active| active.clip_span(sound))
            .ok_or(EditorError::ClipNotFound(sound))?
            .timeline;
        // Early at the head, late at the tail: both are the sound reaching
        // further than the picture.
        let to = match edge {
            TrimEdge::Start => TimelineTime::from_ticks(span.start.ticks() - by.ticks()),
            TrimEdge::End => TimelineTime::from_ticks(span.end.ticks() + by.ticks()),
        };
        self.split_edit(sound, edge, to)
    }

    /// Roll the cut between this pair's sound and the next pair's, leaving
    /// both pictures where they are.
    ///
    /// The gesture that actually makes a split edit between two shots that
    /// touch: there is no room to *extend* a sound into, because the next
    /// shot's sound is already there, so one gives up exactly what the other
    /// takes. Later makes this shot's sound hang over the next picture (an L
    /// for this cut, a J for the next); earlier brings the next shot's sound
    /// in before its picture.
    ///
    /// One undo step. Refused, changing nothing, when either sound runs out of
    /// footage — the same limit an ordinary roll has.
    pub fn roll_sound_cut(&mut self, left: ClipId, by: TimelineTime) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let sound = self.sound_of(left).ok_or(EditorError::NothingLinked)?;
        let track = self
            .track_of(sound)
            .ok_or(EditorError::ClipNotFound(sound))?;
        let right = self
            .roll_partner(sound, TrimEdge::End)
            .ok_or(EditorError::NoNeighbour)?;
        let cut = self
            .active_sequence()
            .and_then(|active| active.clip_span(sound))
            .ok_or(EditorError::ClipNotFound(sound))?
            .timeline
            .end;
        let to = self.snap_to_frame(sequence, TimelineTime::from_ticks(cut.ticks() + by.ticks()));
        if to == cut {
            return Ok(());
        }

        let label = if to > cut { "L Cut" } else { "J Cut" };
        let trim = |clip: ClipId, edge: TrimEdge| Command::TrimClip {
            sequence,
            track,
            clip,
            edge,
            to,
        };
        // Whichever sound is giving up time goes first, so the other never
        // grows into it — the rule an ordinary roll follows.
        let ordered = if to > cut {
            [trim(right, TrimEdge::Start), trim(sound, TrimEdge::End)]
        } else {
            [trim(sound, TrimEdge::End), trim(right, TrimEdge::Start)]
        };
        self.staged(label, |editor, stage| {
            for command in ordered {
                editor.stage(stage, command)?;
            }
            Ok(())
        })
    }

    /// How far this pair's sound reaches past its picture at `edge`: positive
    /// where the sound is early (a J) or late (an L), negative where it falls
    /// short of the picture, zero for a straight cut.
    ///
    /// `None` when there is no linked pair to compare.
    pub fn split_edit_offset(&self, clip: ClipId, edge: TrimEdge) -> Option<TimelineTime> {
        let sequence = self.active_sequence()?;
        let sound = self.sound_of(clip)?;
        let picture = self
            .linked_with(clip)
            .into_iter()
            .find(|member| self.video_clip(*member).is_some())?;
        let (sound, picture) = (
            sequence.clip_span(sound)?.timeline,
            sequence.clip_span(picture)?.timeline,
        );
        Some(match edge {
            TrimEdge::Start => {
                TimelineTime::from_ticks(picture.start.ticks() - sound.start.ticks())
            }
            TrimEdge::End => TimelineTime::from_ticks(sound.end.ticks() - picture.end.ticks()),
        })
    }

    /// Put a split edit back: the sound's edge returns to the picture's.
    pub fn straighten_cut(&mut self, clip: ClipId, edge: TrimEdge) -> Result<(), EditorError> {
        let offset = self
            .split_edit_offset(clip, edge)
            .ok_or(EditorError::NothingLinked)?;
        if offset == TimelineTime::ZERO {
            return Ok(());
        }
        self.lead_sound(clip, edge, TimelineTime::from_ticks(-offset.ticks()))
    }

    /// The sound half of `clip`'s linked pair — `clip` itself when it is the
    /// sound.
    pub fn sound_of(&self, clip: ClipId) -> Option<ClipId> {
        self.linked_with(clip)
            .into_iter()
            .find(|member| self.audio_clip(*member).is_some())
    }
}
