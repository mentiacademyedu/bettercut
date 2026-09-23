//! Tidying lanes: the empty ones gone, and two sparse lanes made one — the
//! timeline after an edit that grew nine lanes for three lanes' worth of
//! clips.

use crate::command::{Command, TrackFlag};
use crate::editor::Editor;
use crate::error::EditorError;
use bettercut_foundation::TrackId;

impl Editor {
    /// Remove every picture and sound lane with nothing on it, as one undo
    /// step — keeping at least one of each kind, and leaving locked lanes
    /// alone. Returns how many went.
    pub fn remove_empty_lanes(&mut self) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some(active) = self.active_sequence() else {
            return Ok(0);
        };
        let pick = |lanes: Vec<(TrackId, bool)>| -> Vec<TrackId> {
            let mut empty: Vec<TrackId> = lanes
                .iter()
                .filter(|(_, is_empty)| *is_empty)
                .map(|(id, _)| *id)
                .collect();
            // Never the last of a kind: somewhere to drop the next file.
            if empty.len() == lanes.len() {
                empty.remove(0);
            }
            empty
        };
        let pictures = pick(
            active
                .video_tracks
                .iter()
                .map(|t| (t.id, t.clips().is_empty()))
                .collect(),
        );
        let sounds = pick(
            active
                .audio_tracks
                .iter()
                .map(|t| (t.id, t.clips().is_empty()))
                .collect(),
        );
        let commands: Vec<Command> = pictures
            .into_iter()
            .chain(sounds)
            .filter(|track| !self.track_flag(*track, TrackFlag::Locked))
            .map(|track| Command::RemoveTrack { sequence, track })
            .collect();
        let count = commands.len();
        if count > 0 {
            self.dispatch_group("Remove Empty Lanes", commands)?;
        }
        Ok(count)
    }

    /// Move `track` one place up (`up`) or down on screen among the lanes
    /// of its kind — a picture lane up draws over the one it passes. One
    /// undo step; `false` when it is already at that end.
    pub fn move_lane(&mut self, track: TrackId, up: bool) -> Result<bool, EditorError> {
        let sequence = self.active_sequence_id()?;
        let active = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence))?;
        // Picture lanes are stored lowest first and drawn highest first;
        // sound lanes are stored and drawn in the same order.
        let (index, count, higher_is_up) =
            if let Some(i) = active.video_tracks.iter().position(|t| t.id == track) {
                (i, active.video_tracks.len(), true)
            } else if let Some(i) = active.audio_tracks.iter().position(|t| t.id == track) {
                (i, active.audio_tracks.len(), false)
            } else {
                return Err(EditorError::TrackNotFound(track));
            };
        let to = if up == higher_is_up {
            index + 1
        } else {
            match index.checked_sub(1) {
                Some(to) => to,
                None => return Ok(false),
            }
        };
        if to >= count {
            return Ok(false);
        }
        self.dispatch(Command::MoveTrack {
            sequence,
            track,
            index: to,
        })?;
        Ok(true)
    }

    /// The lane drawn below `track` on screen, of the same kind.
    pub fn lane_below(&self, track: TrackId) -> Option<TrackId> {
        let sequence = self.active_sequence()?;
        // Picture lanes are drawn highest first; sound lanes in order.
        let on_screen: Vec<TrackId> = if sequence.video_track(track).is_some() {
            sequence.video_tracks.iter().rev().map(|t| t.id).collect()
        } else if sequence.audio_tracks.iter().any(|t| t.id == track) {
            sequence.audio_tracks.iter().map(|t| t.id).collect()
        } else {
            return None;
        };
        let index = on_screen.iter().position(|t| *t == track)?;
        on_screen.get(index + 1).copied()
    }

    /// Move every clip on `track` onto the lane below it, at the same times,
    /// and remove `track` — one undo step. Refused, changing nothing, when
    /// any clip would land on one already there. Returns how many moved.
    pub fn merge_lane_down(&mut self, track: TrackId) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let into = self
            .lane_below(track)
            .ok_or(EditorError::TrackNotFound(track))?;
        let Some(active) = self.active_sequence() else {
            return Ok(0);
        };
        let spans: Vec<_> = active.clip_spans().collect();
        let moving: Vec<_> = spans.iter().filter(|s| s.track == track).collect();
        let staying: Vec<_> = spans.iter().filter(|s| s.track == into).collect();
        if moving
            .iter()
            .any(|m| staying.iter().any(|s| s.timeline.overlaps(m.timeline)))
        {
            return Err(EditorError::LanesOverlap);
        }
        let mut commands: Vec<Command> = moving
            .iter()
            .map(|span| Command::MoveClip {
                sequence,
                from_track: track,
                to_track: into,
                clip: span.clip,
                new_start: span.timeline.start,
            })
            .collect();
        let count = commands.len();
        commands.push(Command::RemoveTrack { sequence, track });
        self.dispatch_group("Merge Lanes", commands)?;
        Ok(count)
    }
}
