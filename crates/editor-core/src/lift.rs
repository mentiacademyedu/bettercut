//! Lift and extract: taking the marked range out of the whole edit.
//!
//! Marks in and out bracket a stretch of the timeline — a fluffed line, a
//! dead moment. **Lift** takes everything inside the marks away on every lane
//! and leaves the gap where it was, so nothing else moves. **Extract** takes
//! it away and closes the gap, pulling everything after it left by the range's
//! length on every lane together, so picture, sound and titles stay in step.
//!
//! Clips the marks fall inside are split at the marks first, so only the part
//! inside goes. Locked lanes are left exactly as they are — and an extract is
//! refused if one of them has anything after the marks, because pulling the
//! rest of the edit left past a lane that cannot move would put it out of
//! sync. Markers inside the range go with it; an extract moves the later ones
//! along. One undo step either way.

use bettercut_timeline::{TimelineRange, TrackKind};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Take the marked range out and leave the gap. Returns how many clips
    /// (or parts of clips) were removed.
    pub fn lift_marked(&mut self) -> Result<usize, EditorError> {
        let range = self.marked_range_or_refuse()?;
        self.take_out(range, false)
    }

    /// Take the marked range out and close the gap on every lane. Returns how
    /// many clips (or parts of clips) were removed.
    pub fn extract_marked(&mut self) -> Result<usize, EditorError> {
        let range = self.marked_range_or_refuse()?;
        self.take_out(range, true)
    }

    fn marked_range_or_refuse(&self) -> Result<TimelineRange, EditorError> {
        self.active_sequence()
            .and_then(|s| s.marked_range())
            .ok_or(EditorError::NoMarkedRange)
    }

    fn take_out(&mut self, range: TimelineRange, close: bool) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let locked = |editor: &Editor, track| {
            editor
                .active_sequence()
                .is_some_and(|s| crate::gaps::track_locked(s, track))
        };
        if close
            && self.active_sequence().is_some_and(|s| {
                s.clip_spans().any(|span| {
                    span.timeline.end > range.start && crate::gaps::track_locked(s, span.track)
                })
            })
        {
            return Err(EditorError::LockedTrackInTheWay);
        }

        self.staged(if close { "Extract" } else { "Lift" }, |editor, stage| {
            // Cut at both marks, so only what is inside comes out.
            for at in [range.start, range.end] {
                for command in editor.split_commands(sequence, at, &[]) {
                    if let Command::SplitClip { track, .. } = &command
                        && locked(editor, *track)
                    {
                        continue;
                    }
                    editor.stage(stage, command)?;
                }
            }

            // Everything now wholly inside the marks.
            let inside: Vec<_> = editor
                .active_sequence()
                .map(|s| {
                    s.clip_spans()
                        .filter(|span| {
                            span.timeline.start >= range.start
                                && span.timeline.end <= range.end
                                && !crate::gaps::track_locked(s, span.track)
                        })
                        .map(|span| (span.track, span.clip, span.kind))
                        .collect()
                })
                .unwrap_or_default();
            let removed = inside.len();
            for (track, clip, kind) in inside {
                let command = match kind {
                    TrackKind::Text => Command::RemoveText {
                        sequence,
                        track,
                        clip,
                    },
                    TrackKind::Adjustment => Command::RemoveAdjustment {
                        sequence,
                        track,
                        clip,
                    },
                    _ => Command::RemoveClip {
                        sequence,
                        track,
                        clip,
                    },
                };
                editor.stage(stage, command)?;
            }

            let length = range.duration();
            if close {
                // Earliest first: each moves left into space already cleared.
                let mut after = editor.clips_from(range.end, false);
                after.sort_by_key(|(_, _, start)| start.ticks());
                for (track, clip, start) in after {
                    editor.stage(
                        stage,
                        Command::MoveClip {
                            sequence,
                            from_track: track,
                            to_track: track,
                            clip,
                            new_start: start - length,
                        },
                    )?;
                }
            }

            // Markers inside go; after the range, they follow the edit.
            let markers = editor.markers().to_vec();
            let kept: Vec<_> = markers
                .iter()
                .filter(|m| !(m.time >= range.start && m.time < range.end))
                .map(|m| {
                    let mut m = m.clone();
                    if close && m.time >= range.end {
                        m.time -= length;
                    }
                    m
                })
                .collect();
            if kept != markers {
                editor.stage(
                    stage,
                    Command::SetMarkers {
                        sequence,
                        markers: kept,
                    },
                )?;
            }
            if close {
                // The marks bracketed what is now gone.
                editor.stage(
                    stage,
                    Command::SetInOut {
                        sequence,
                        mark_in: None,
                        mark_out: None,
                    },
                )?;
            }
            Ok(removed)
        })
    }
}
