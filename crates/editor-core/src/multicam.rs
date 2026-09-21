//! Multicam: several cameras on one scene, played as one clip you cut between.
//!
//! A multicam clip *is* a compound clip (`crate::compound`) with one camera per
//! picture lane inside it, plus one number on the outside saying which lane is
//! on screen. Cutting to another angle is a split and a new number, which is
//! why the whole thing needs no new playback path: the compound expansion
//! already knows how to play what is inside, and simply leaves out the lanes
//! that are not the chosen one.
//!
//! Syncing the cameras is a separate job, and one already done:
//! `Editor::sync_to_sound` lines them up by what they heard, and this takes
//! them as it finds them.

use bettercut_foundation::{ClipId, TimelineTime};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Fold `clips` into a multicam clip: one camera per lane, angle one on
    /// screen. Returns the clip left in their place.
    ///
    /// Each clip keeps where it sits relative to the others, so cameras that
    /// were lined up stay lined up. The first one given becomes angle one.
    pub fn make_multicam(&mut self, clips: &[ClipId], name: &str) -> Result<ClipId, EditorError> {
        if clips.len() < 2 {
            return Err(EditorError::NotEnoughAngles);
        }
        // Each camera onto a lane of its own first: `make_compound` keeps the
        // lanes clips were on, so two cameras stacked on one lane would end up
        // on one lane inside as well — and one would cover the other.
        let sequence = self.active_sequence_id()?;
        let lanes: Vec<_> = self
            .project()
            .sequence(sequence)
            .map(|active| active.video_tracks.iter().map(|t| t.id).collect::<Vec<_>>())
            .unwrap_or_default();
        for (index, clip) in clips.iter().enumerate() {
            if self.video_clip(*clip).is_none() {
                return Err(EditorError::ClipKindMismatch);
            }
            let Some(&wanted) = lanes.get(index) else {
                return Err(EditorError::NotEnoughAngles);
            };
            let (Some(from), Some(start)) = (self.track_of(*clip), self.clip_start_of(*clip))
            else {
                continue;
            };
            if from != wanted {
                self.move_clip(from, wanted, *clip, start)?;
            }
        }

        let compound = self.make_compound(clips, name)?;
        self.set_angle(compound, Some(0))?;
        Ok(compound)
    }

    /// How many angles a multicam clip has. `None` when it is not one.
    pub fn angle_count(&self, clip: ClipId) -> Option<usize> {
        let inner = self.compound_of(clip)?;
        let sequence = self.project().sequence(inner)?;
        Some(sequence.video_tracks.len())
    }

    /// Which angle is on screen, or `None` for all of them at once.
    pub fn angle_of(&self, clip: ClipId) -> Option<usize> {
        self.video_clip(clip)?.angle
    }

    /// Put `angle` on screen for the whole of `clip`.
    ///
    /// `None` plays every lane at once, which is what an ordinary compound
    /// does. An angle the clip does not have is refused.
    pub fn set_angle(&mut self, clip: ClipId, angle: Option<usize>) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        if let Some(angle) = angle {
            let count = self.angle_count(clip).ok_or(EditorError::NotMulticam)?;
            if angle >= count {
                return Err(EditorError::NoSuchAngle {
                    angle: angle + 1,
                    count,
                });
            }
        }
        self.dispatch(Command::SetClipAngle {
            sequence,
            track,
            clip,
            angle,
        })
    }

    /// Cut to `angle` at `at`: the multicam is split there, and everything
    /// from the cut on shows the new camera. One undo step.
    ///
    /// Returns the clip that now plays after the cut — the same clip when the
    /// cut lands on its very start, which is a change of angle rather than a
    /// cut.
    pub fn cut_to_angle(
        &mut self,
        clip: ClipId,
        at: TimelineTime,
        angle: usize,
    ) -> Result<ClipId, EditorError> {
        let count = self.angle_count(clip).ok_or(EditorError::NotMulticam)?;
        if angle >= count {
            return Err(EditorError::NoSuchAngle {
                angle: angle + 1,
                count,
            });
        }
        let span = self
            .video_clip(clip)
            .ok_or(EditorError::ClipNotFound(clip))?
            .timeline;
        if !span.contains(at) {
            return Err(EditorError::PlayheadOffClip);
        }
        // On the clip's own start there is nothing to split.
        if at <= span.start {
            self.set_angle(clip, Some(angle))?;
            return Ok(clip);
        }

        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let at = self
            .project()
            .sequence(sequence)
            .map_or(at, |active| active.snap_to_frame(at));
        self.staged(format!("Cut to Angle {}", angle + 1), |editor, stage| {
            for command in editor.split_commands(sequence, at, &[clip]) {
                editor.stage(stage, command)?;
            }
            // The halves are new clips; the right one takes the new angle.
            let after = editor
                .project()
                .sequence(sequence)
                .and_then(|active| {
                    active
                        .clip_spans()
                        .find(|span| span.track == track && span.timeline.start == at)
                        .map(|span| span.clip)
                })
                .ok_or(EditorError::ClipNotFound(clip))?;
            editor.stage(
                stage,
                Command::SetClipAngle {
                    sequence,
                    track,
                    clip: after,
                    angle: Some(angle),
                },
            )?;
            Ok(after)
        })
    }

    fn clip_start_of(&self, clip: ClipId) -> Option<TimelineTime> {
        Some(self.project().active()?.clip_span(clip)?.timeline.start)
    }
}
