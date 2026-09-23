//! Making several picture clips one length — the slideshow at three seconds
//! a photo — with what follows each on its lane moved to suit.
//!
//! Only the edited lanes ripple (and the sound linked to what moves): music
//! under a slideshow is not cut or shifted because a photo got longer.

use std::collections::HashMap;

use bettercut_foundation::{ClipId, TICKS_PER_SECOND, TimelineTime, TrackId};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;
use crate::ops::TrimEdge;

impl Editor {
    /// Make each picture or title clip in `clips` last `length` — photos,
    /// held frames and titles any length, footage as far as it runs — and
    /// move whatever
    /// follows on their lanes (with its linked sound) so nothing overlaps and
    /// no gap opens. One undo step. Returns how many changed length.
    pub fn set_clip_lengths(
        &mut self,
        clips: &[ClipId],
        length: TimelineTime,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        if length.ticks() <= 0 {
            return Ok(0);
        }
        let Some(active) = self.active_sequence() else {
            return Ok(0);
        };
        let spans: Vec<(TrackId, ClipId, i64, i64)> = active
            .clip_spans()
            .map(|s| {
                (
                    s.track,
                    s.clip,
                    s.timeline.start.ticks(),
                    s.timeline.end.ticks(),
                )
            })
            .collect();
        let far = spans.iter().map(|s| s.3).max().unwrap_or(0) + 3_600 * TICKS_PER_SECOND;

        // What each changed clip becomes, as a change in length.
        let mut delta: HashMap<ClipId, i64> = HashMap::new();
        for clip in clips {
            // Pictures and titles: a sound clip's length is its recording.
            let lengthens = self.video_clip(*clip).is_some() || self.text_clip(*clip).is_some();
            if !lengthens || delta.contains_key(clip) {
                continue;
            }
            let Some(&(_, _, start, end)) = spans.iter().find(|s| s.1 == *clip) else {
                continue;
            };
            let now = end - start;
            let mut target = length.ticks();
            if target > now
                && let Some(room) = self.growth_room(*clip, TrimEdge::End)
            {
                target = target.min(now + room.max(0));
            }
            if target != now {
                delta.insert(*clip, target - now);
            }
        }
        if delta.is_empty() {
            return Ok(0);
        }

        // How far everything after a changed clip on its lane moves.
        let mut lanes: Vec<TrackId> = spans
            .iter()
            .filter(|s| delta.contains_key(&s.1))
            .map(|s| s.0)
            .collect();
        lanes.dedup();
        let mut shifts: Vec<(ClipId, i64)> = Vec::new();
        for lane in lanes {
            let mut on_lane: Vec<_> = spans.iter().filter(|s| s.0 == lane).collect();
            on_lane.sort_by_key(|s| s.2);
            let mut shift = 0;
            for &&(_, clip, _, _) in &on_lane {
                if shift != 0 && !shifts.iter().any(|(c, _)| *c == clip) {
                    shifts.push((clip, shift));
                }
                shift += delta.get(&clip).copied().unwrap_or(0);
            }
        }
        for (clip, shift) in shifts.clone() {
            for partner in self.linked_with(clip) {
                if !shifts.iter().any(|(c, _)| *c == partner) {
                    shifts.push((partner, shift));
                }
            }
        }
        let start_of = |clip: ClipId| spans.iter().find(|s| s.1 == clip).map_or(0, |s| s.2);
        let track_of = |clip: ClipId| spans.iter().find(|s| s.1 == clip).map(|s| s.0);
        let mut changed: Vec<(ClipId, i64)> = delta.iter().map(|(c, d)| (*c, *d)).collect();
        changed.sort_by_key(|(clip, _)| start_of(*clip));
        let count = changed.len();

        self.staged("Set Length", |editor, stage| {
            let trim =
                |editor: &mut Self, stage: &mut crate::editor::Stage, clip: ClipId, by: i64| {
                    let mut members = vec![clip];
                    members.extend(editor.linked_with(clip).into_iter().filter(|c| *c != clip));
                    for member in members {
                        let (Some(track), Some(end)) =
                            (editor.track_of(member), editor.clip_end(member))
                        else {
                            continue;
                        };
                        editor.stage(
                            stage,
                            Command::TrimClip {
                                sequence,
                                track,
                                clip: member,
                                edge: TrimEdge::End,
                                to: TimelineTime::from_ticks(end.ticks() + by),
                            },
                        )?;
                    }
                    Ok::<(), EditorError>(())
                };
            // Shorter first, where they are: that only makes room.
            for (clip, by) in changed.iter().filter(|(_, by)| *by < 0) {
                trim(editor, stage, *clip, *by)?;
            }
            // Everything that moves goes out of the way, latest first, then
            // comes back to where it belongs, earliest first — whichever way
            // each has to go, nothing lands on something not yet moved.
            let mut moving = shifts.clone();
            moving.sort_by_key(|(clip, _)| std::cmp::Reverse(start_of(*clip)));
            for (clip, _) in &moving {
                let Some(track) = track_of(*clip) else {
                    continue;
                };
                editor.stage(
                    stage,
                    Command::MoveClip {
                        sequence,
                        from_track: track,
                        to_track: track,
                        clip: *clip,
                        new_start: TimelineTime::from_ticks(start_of(*clip) + far),
                    },
                )?;
            }
            moving.reverse();
            for (clip, shift) in &moving {
                let Some(track) = track_of(*clip) else {
                    continue;
                };
                editor.stage(
                    stage,
                    Command::MoveClip {
                        sequence,
                        from_track: track,
                        to_track: track,
                        clip: *clip,
                        new_start: TimelineTime::from_ticks(start_of(*clip) + shift),
                    },
                )?;
            }
            // Longer last, into the room the moves made.
            for (clip, by) in changed.iter().filter(|(_, by)| *by > 0) {
                trim(editor, stage, *clip, *by)?;
            }
            Ok(count)
        })
    }
}
