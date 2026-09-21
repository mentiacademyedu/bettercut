//! A magnetic main track: the first picture lane keeps its clips packed end to
//! end from the start, the way a phone editor's main track does. Delete a shot
//! or drag one away and the rest close up behind it.
//!
//! Not a different kind of track and not a rule every edit has to know: after
//! an edit, [`Editor::close_up_main_track`] packs the lane and folds the moves
//! into the step just taken, so one undo takes back the edit and its closing
//! up together.

use bettercut_foundation::{ClipId, TimelineTime, TrackId};
use bettercut_timeline::TimelineRange;

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Whether the project's main track is magnetic.
    pub fn is_magnetic(&self) -> bool {
        self.project().settings.magnetic_timeline
    }

    /// The main track: the first picture lane.
    pub fn main_track(&self) -> Option<TrackId> {
        self.active_sequence()?.video_tracks.first().map(|t| t.id)
    }

    /// Pack the main track's clips end to end from the start, their linked
    /// sound moving with them, as part of the last step taken. Returns how
    /// many clips moved.
    ///
    /// Stops at the first clip whose sound would land on another clip, packing
    /// what came before: a magnetic track must never overwrite anything.
    pub fn close_up_main_track(&mut self) -> Result<usize, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let Some(main) = self.main_track() else {
            return Ok(0);
        };
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let spans: Vec<_> = sequence.clip_spans().collect();
        let mut on_main: Vec<_> = spans.iter().filter(|s| s.track == main).collect();
        on_main.sort_by_key(|s| s.timeline.start);

        // Where everything is after the moves so far, to check each landing.
        let mut placed: Vec<(TrackId, ClipId, TimelineRange)> = spans
            .iter()
            .map(|s| (s.track, s.clip, s.timeline))
            .collect();
        let mut commands = Vec::new();
        let mut moved = 0;
        let mut at = TimelineTime::ZERO;
        for span in on_main {
            let shift = at.ticks() - span.timeline.start.ticks();
            if shift != 0 {
                let group = self.linked_with(span.clip);
                let mut landings = Vec::new();
                for member in &group {
                    let Some(&(track, _, range)) = placed.iter().find(|(_, c, _)| c == member)
                    else {
                        continue;
                    };
                    let landed = TimelineRange {
                        start: TimelineTime::from_ticks(range.start.ticks() + shift),
                        end: TimelineTime::from_ticks(range.end.ticks() + shift),
                    };
                    let blocked = landed.start.is_negative()
                        || placed.iter().any(|(t, c, r)| {
                            *t == track && !group.contains(c) && r.overlaps(landed)
                        });
                    if blocked {
                        landings.clear();
                        break;
                    }
                    landings.push((track, *member, landed));
                }
                if landings.is_empty() {
                    break;
                }
                for (track, clip, landed) in landings {
                    commands.push(Command::MoveClip {
                        sequence: sequence_id,
                        from_track: track,
                        to_track: track,
                        clip,
                        new_start: landed.start,
                    });
                    if let Some(entry) = placed.iter_mut().find(|(_, c, _)| *c == clip) {
                        entry.2 = landed;
                    }
                }
                moved += 1;
            }
            at = TimelineTime::from_ticks(span.timeline.end.ticks() + shift);
        }
        self.dispatch_amending(commands)?;
        Ok(moved)
    }
}
