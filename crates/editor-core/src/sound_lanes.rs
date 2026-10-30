//! Sounds over sounds: music under a voice, an effect over the music. A
//! sound file dropped on the timeline goes on the sound lane it was dropped
//! on — or, where that lane is taken there, the next one down with room, and
//! a new lane under the rest where none has — and nothing already there
//! moves or is covered. A sound already on the timeline, dragged, lands
//! the same way.

use bettercut_foundation::{ClipId, MediaId, MediaTime, TimelineTime, TrackId};
use bettercut_timeline::TimelineRange;

use bettercut_timeline::TrackKind;

use crate::command::{ClipPayload, Command, TrackKindRepr};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// The sound lane a sound `length` long dropped at `at` on `lane` lands
    /// on: `lane` itself where it is free there, else the first lane below
    /// it that is, else `None` — a new lane under the rest. `lane` `None` is
    /// a drop below every lane.
    pub fn sound_lane_for(
        &self,
        lane: Option<TrackId>,
        at: TimelineTime,
        length: TimelineTime,
    ) -> Option<TrackId> {
        let sequence = self.active_sequence()?;
        let from = sequence
            .audio_tracks
            .iter()
            .position(|t| Some(t.id) == lane)?;
        let span =
            TimelineRange::new(at, TimelineTime::from_ticks(at.ticks() + length.ticks())).ok()?;
        sequence.audio_tracks[from..]
            .iter()
            .find(|t| t.clips_in_range(span).is_empty())
            .map(|t| t.id)
    }

    /// The sound lane sound clip `clip`, moved to start at `start` on `lane`,
    /// lands on: as [`Self::sound_lane_for`], but the clip and everything
    /// moving with it are out of the way — a sound moved along its own lane
    /// stays there. `None` is a new lane (or `clip` is no sound).
    pub fn sound_move_lane(
        &self,
        clip: ClipId,
        lane: Option<TrackId>,
        start: TimelineTime,
    ) -> Option<TrackId> {
        let sequence = self.active_sequence()?;
        let span = sequence.clip_span(clip)?;
        if span.kind != TrackKind::Audio {
            return None;
        }
        let length = span.timeline.duration().ticks().max(1);
        let span =
            TimelineRange::new(start, TimelineTime::from_ticks(start.ticks() + length)).ok()?;
        let moving = self.moves_with(clip);
        let from = sequence
            .audio_tracks
            .iter()
            .position(|t| Some(t.id) == lane)?;
        sequence.audio_tracks[from..]
            .iter()
            .find(|t| {
                t.clips_in_range(span)
                    .iter()
                    .all(|c| c.id == clip || moving.contains(&c.id))
            })
            .map(|t| t.id)
    }

    /// Move sound clip `clip` to start at `start`, on `lane` or the lane
    /// [`Self::sound_move_lane`] picks — a new one under the rest where
    /// none has room, `lane` `None` asking for one outright. What is linked
    /// or grouped with it moves by as much on its own lane (§12). One undo
    /// step; returns the lane it is on.
    pub fn move_sound(
        &mut self,
        clip: ClipId,
        lane: Option<TrackId>,
        start: TimelineTime,
    ) -> Result<TrackId, EditorError> {
        let sequence = self.active_sequence_id()?;
        let from = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        if self.audio_clip(clip).is_none() {
            return Err(EditorError::ClipKindMismatch);
        }
        if let Some(track) = lane.and_then(|lane| self.sound_move_lane(clip, Some(lane), start)) {
            self.move_clip(from, track, clip, start)?;
            return Ok(track);
        }
        let lanes = self.active_sequence().map_or(0, |s| s.audio_tracks.len());
        let track = TrackId::new();
        let primary = Command::MoveClip {
            sequence,
            from_track: from,
            to_track: track,
            clip,
            new_start: start,
        };
        let moves = self.move_commands(sequence, clip, primary, start);
        self.staged("Move Clip", |editor, stage| {
            editor.stage(
                stage,
                Command::AddTrack {
                    sequence,
                    kind: TrackKindRepr::Audio,
                    name: format!("A{}", lanes + 1),
                    id: track,
                },
            )?;
            for command in moves {
                editor.stage(stage, command)?;
            }
            Ok(())
        })?;
        Ok(track)
    }

    /// Put sound file `media` — or the `part` of it marked in the browser —
    /// down at `at`, on `lane` or the lane [`Self::sound_lane_for`] picks
    /// (a new one, named after the rest, where none has room). A file with
    /// several sound tracks brings the rest on lanes of their own, linked.
    /// Nothing already on the timeline moves. One undo step; returns the
    /// clips made.
    pub fn place_sound_at(
        &mut self,
        media: MediaId,
        part: Option<(MediaTime, MediaTime)>,
        at: TimelineTime,
        lane: Option<TrackId>,
    ) -> Result<Vec<ClipId>, EditorError> {
        let sequence = self.active_sequence_id()?;
        let at = self.snap_to_frame(sequence, TimelineTime::from_ticks(at.ticks().max(0)));
        let asset = self
            .project()
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?;
        if asset.audio_codec.is_none() && asset.generated_sound.is_none() {
            return Err(EditorError::ClipKindMismatch);
        }
        let more_tracks = asset.track_of.is_none() && asset.audio_tracks > 1;
        let duration = asset.placement_duration_with(self.project().settings.photo_length);
        let source = match part {
            None => bettercut_timeline::SourceRange::new(MediaTime::ZERO, duration)?,
            Some((from, to)) => {
                let (from, to) = if to < from { (to, from) } else { (from, to) };
                let limit = duration.ticks().max(1);
                let from = from.ticks().clamp(0, limit - 1);
                let to = to.ticks().clamp(from + 1, limit);
                bettercut_timeline::SourceRange::new(
                    MediaTime::from_ticks(from),
                    MediaTime::from_ticks(to),
                )?
            }
        };
        let length = TimelineTime::from_ticks(source.duration().ticks().max(1));
        let lanes = self.active_sequence().map_or(0, |s| s.audio_tracks.len());

        let mut commands = Vec::new();
        let track = match self.sound_lane_for(lane, at, length) {
            Some(track) => track,
            None => {
                let id = TrackId::new();
                commands.push(Command::AddTrack {
                    sequence,
                    kind: TrackKindRepr::Audio,
                    name: format!("A{}", lanes + 1),
                    id,
                });
                id
            }
        };
        let clip = bettercut_timeline::AudioClip::new(media, at, source)?;
        let mut placed = vec![clip.id];
        commands.push(Command::AddClip {
            sequence,
            track,
            clip: ClipPayload::Audio(Box::new(clip)),
        });
        self.staged("Add Sound", |editor, stage| {
            for command in commands {
                editor.stage(stage, command)?;
            }
            if more_tracks && let Ok((more, clips)) = editor.other_sound_track_commands(placed[0]) {
                for command in more {
                    editor.stage(stage, command)?;
                }
                placed.extend(clips);
            }
            Ok(())
        })?;
        Ok(placed)
    }

    /// Add an empty sound lane under the rest, named after them. One undo
    /// step; returns the lane.
    pub fn add_sound_lane(&mut self) -> Result<TrackId, EditorError> {
        let sequence = self.active_sequence_id()?;
        let lanes = self.active_sequence().map_or(0, |s| s.audio_tracks.len());
        let id = TrackId::new();
        self.dispatch(Command::AddTrack {
            sequence,
            kind: TrackKindRepr::Audio,
            name: format!("A{}", lanes + 1),
            id,
        })?;
        Ok(id)
    }
}
