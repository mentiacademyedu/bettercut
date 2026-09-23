//! A clip moved a lane up or down, keeping its time — the keyboard-free way
//! to put a shot over another for a picture-in-picture, or to take a sound
//! onto its own lane for a separate volume line.

use bettercut_foundation::{ClipId, TrackId};
use bettercut_timeline::TrackKind;

use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// The lane on screen above (`up`) or below the one `clip` is on, of the
    /// same kind, if there is one. Picture lanes are drawn top-down from the
    /// highest, so "up" is the next picture lane; sound lanes are drawn in
    /// order, so "up" is the previous one.
    pub fn neighbour_lane(&self, clip: ClipId, up: bool) -> Option<TrackId> {
        let sequence = self.active_sequence()?;
        let span = sequence.clip_span(clip)?;
        let on_screen: Vec<TrackId> = match span.kind {
            TrackKind::Video => sequence.video_tracks.iter().rev().map(|t| t.id).collect(),
            TrackKind::Audio => sequence.audio_tracks.iter().map(|t| t.id).collect(),
            _ => return None,
        };
        let index = on_screen.iter().position(|t| *t == span.track)?;
        let target = if up { index.checked_sub(1)? } else { index + 1 };
        on_screen.get(target).copied()
    }

    /// Copy picture clip `clip` onto the lane above it at the same time — the
    /// lane directly above when it is free there, or a new lane put in
    /// between. The copy has no sound. One undo step; returns the copy.
    pub fn duplicate_onto_lane_above(&mut self, clip: ClipId) -> Result<ClipId, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some(crate::command::ClipPayload::Video(mut picture)) = self.clip_payload(clip) else {
            return Err(EditorError::ClipNotFound(clip));
        };
        picture.link = None;
        let span = picture.timeline;
        let active = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence))?;
        let home = active
            .video_tracks
            .iter()
            .position(|t| t.get(clip).is_some())
            .ok_or(EditorError::ClipNotFound(clip))?;
        let free = active
            .video_tracks
            .get(home + 1)
            .filter(|t| t.clips().iter().all(|c| !c.timeline.overlaps(span)))
            .map(|t| t.id);
        let new_id = ClipId::new();
        self.staged("Duplicate Above", |editor, stage| {
            let track = match free {
                Some(track) => track,
                None => {
                    let lane = bettercut_timeline::VideoTrack::new(format!("V{}", home + 2));
                    let id = lane.id;
                    editor.stage(
                        stage,
                        crate::command::Command::InsertTrack {
                            sequence,
                            index: home + 1,
                            track: crate::command::TrackPayload::Video(Box::new(lane)),
                        },
                    )?;
                    id
                }
            };
            editor.stage(
                stage,
                crate::command::Command::PasteClip {
                    sequence,
                    track,
                    clip: crate::command::ClipPayload::Video(picture),
                    at: span.start,
                    new_id,
                },
            )?;
            Ok(new_id)
        })
    }

    /// Move `clip` to the neighbouring lane, at the same time. Refused when
    /// there is no lane there or the place is taken — nothing is shuffled to
    /// make room, because a move nobody asked for is worse than none.
    pub fn move_to_neighbour_lane(&mut self, clip: ClipId, up: bool) -> Result<(), EditorError> {
        let span = self
            .active_sequence()
            .and_then(|s| s.clip_span(clip))
            .ok_or(EditorError::ClipNotFound(clip))?;
        let target = self
            .neighbour_lane(clip, up)
            .ok_or(EditorError::TrackNotFound(span.track))?;
        self.move_clip(span.track, target, clip, span.timeline.start)
    }
}
