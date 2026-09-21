//! Fit to fill: re-timing a clip so it ends exactly where it should.
//!
//! A shot is a little short for the space left for it — the gap before the
//! next clip, or the stretch up to the playhead. Trimming cannot lengthen it
//! past its material, but playing it slower can. Fit to fill picks the one
//! speed at which the clip's material lasts exactly that long, and applies it
//! the way the speed control does (§12: with its linked sound; §79: one undo
//! step), so the same limits hold: a clip already faster or slower than
//! [`MIN_SPEED`]..[`MAX_SPEED`] allows is refused rather than half-fitted.
//!
//! The speed is exact (§74): material ticks over timeline ticks, a
//! `Rational`, so the clip ends on the very tick asked for.

use bettercut_foundation::{ClipId, Rational, TimelineTime};
use bettercut_timeline::{MAX_SPEED, MIN_SPEED};

use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Where fit to fill would end `clip`: the start of the next clip on its
    /// track, when there is space between the two. `None` when the next clip
    /// touches it or nothing follows.
    pub fn fill_end(&self, clip: ClipId) -> Option<TimelineTime> {
        let sequence = self.project().active()?;
        let span = sequence.clip_span(clip)?;
        sequence
            .clip_spans()
            .filter(|other| other.track == span.track && other.clip != clip)
            .map(|other| other.timeline.start)
            .filter(|start| *start >= span.timeline.end)
            .min()
            .filter(|start| *start > span.timeline.end)
    }

    /// The speed at which `clip`'s material lasts from its start to `end`.
    /// `None` when `end` is not after the start, or the clip cannot be
    /// re-timed.
    pub fn speed_to_end_at(&self, clip: ClipId, end: TimelineTime) -> Option<Rational> {
        let sequence = self.project().active()?;
        let span = sequence.clip_span(clip)?;
        let length = end.ticks() - span.timeline.start.ticks();
        if length <= 0 || !self.can_retime(clip) {
            return None;
        }
        let material = self
            .video_clip(clip)
            .map(|c| c.source.duration())
            .or_else(|| self.audio_clip(clip).map(|c| c.source.duration()))?;
        Rational::new(material.ticks(), length)
    }

    /// Re-time `clip`, with its linked sound, to fill the gap after it. One
    /// undo step. Returns the speed it now plays at.
    pub fn fit_to_fill(&mut self, clip: ClipId) -> Result<Rational, EditorError> {
        let end = self.fill_end(clip).ok_or(EditorError::NoGapThere)?;
        self.fit_to_end(clip, end)
    }

    /// Re-time `clip`, with its linked sound, so it ends at `end` — faster to
    /// end sooner, slower to end later. One undo step. Returns the speed.
    ///
    /// Refused, changing nothing, when the clip has no motion to re-time, when
    /// `end` is not after its start, when the speed would be past the speed
    /// control's limits, or when the clip or its sound has no room to grow.
    pub fn fit_to_end(&mut self, clip: ClipId, end: TimelineTime) -> Result<Rational, EditorError> {
        if !self.can_retime(clip) {
            return Err(EditorError::NoMotionToRetime);
        }
        let speed = self
            .speed_to_end_at(clip, end)
            .ok_or(EditorError::FillOutOfRange)?;
        // Cross-multiplied, so the comparison is as exact as the speed.
        let too_slow = speed.num() * MIN_SPEED.den() < MIN_SPEED.num() * speed.den();
        let too_fast = speed.num() * MAX_SPEED.den() > MAX_SPEED.num() * speed.den();
        if too_slow || too_fast {
            return Err(EditorError::FillOutOfRange);
        }
        let sequence = self.active_sequence_id()?;

        // A speed change ripples what follows on the track; a fit must not, so
        // each clip it would push is put straight back, in the same step. When
        // something is in the way — the sound's track is busier than the
        // picture's — putting it back fails and the whole fit is refused.
        let mut speeds = Vec::new();
        let mut returns = Vec::new();
        {
            let active = self
                .active_sequence()
                .ok_or(EditorError::SequenceNotFound(sequence))?;
            for member in self.linked_with(clip) {
                let Some(span) = active.clip_span(member) else {
                    continue;
                };
                speeds.push(crate::command::Command::SetClipSpeed {
                    sequence,
                    track: span.track,
                    clip: member,
                    speed,
                });
                let mut later: Vec<_> = active
                    .clip_spans()
                    .filter(|other| {
                        other.track == span.track && other.timeline.start >= span.timeline.end
                    })
                    .map(|other| (other.clip, other.track, other.timeline.start))
                    .collect();
                // Moved back towards their places nearest-first when the clip
                // grew, farthest-first when it shrank, so none lands on another.
                let grew = end > span.timeline.end;
                later
                    .sort_by_key(|(_, _, start)| if grew { start.ticks() } else { -start.ticks() });
                returns.extend(later);
            }
        }
        if speeds.is_empty() {
            return Err(EditorError::ClipNotFound(clip));
        }
        self.staged("Fit to Fill", |editor, stage| {
            for command in speeds {
                editor.stage(stage, command)?;
            }
            for (other, track, start) in returns {
                editor.stage(
                    stage,
                    crate::command::Command::MoveClip {
                        sequence,
                        from_track: track,
                        to_track: track,
                        clip: other,
                        new_start: start,
                    },
                )?;
            }
            Ok(())
        })?;
        Ok(speed)
    }
}
