//! Slip edit: keep a clip where it is on the timeline and change which part of
//! its file it plays — its in and out points slide together, so its length and
//! position never change.
//!
//! The clip's keyframes are anchored to source time (§24), so they slide by the
//! same amount; otherwise a zoom set up on the timeline would start somewhere
//! else after the slip.

use bettercut_foundation::{ClipId, MediaTime, SequenceId, TrackId};
use bettercut_project_format::Project;
use bettercut_timeline::{Keyframes, SourceRange};

use crate::command::{Command, EditorCommand};
use crate::editor::Editor;
use crate::error::EditorError;

/// How far a clip's source range can slide: the most it can move earlier (zero
/// or negative) and later (zero or positive), in ticks of source.
///
/// `None` for a clip with nothing either side to show — a photo, a colour, a
/// held frame, a title — or one that isn't a picture or sound clip at all.
pub fn slip_room(project: &Project, sequence: SequenceId, clip: ClipId) -> Option<(i64, i64)> {
    let sequence = project.sequence(sequence)?;
    let (media, source, frozen) = sequence
        .video_tracks
        .iter()
        .find_map(|t| t.get(clip).map(|c| (c.media_id, c.source, c.frozen)))
        .or_else(|| {
            sequence
                .audio_tracks
                .iter()
                .find_map(|t| t.get(clip).map(|c| (c.media_id, c.source, false)))
        })?;
    if frozen {
        return None;
    }
    let asset = project.media_asset(media)?;
    if asset.generated.is_some() {
        return None;
    }
    let limit = asset.source_limit()?;
    let earlier = -source.start.ticks().max(0);
    let later = (limit.ticks() - source.end.ticks()).max(0);
    Some((earlier, later))
}

/// Slide one clip's source range (and its keys) by `offset` ticks, held inside
/// the file.
#[derive(Debug)]
pub struct SlipClip {
    pub sequence: SequenceId,
    pub track: TrackId,
    pub clip: ClipId,
    pub offset: i64,
    /// The range and keys as they were, for undo.
    previous: Option<(SourceRange, Keyframes)>,
}

impl SlipClip {
    pub fn new(sequence: SequenceId, track: TrackId, clip: ClipId, offset: i64) -> Self {
        Self {
            sequence,
            track,
            clip,
            offset,
            previous: None,
        }
    }

    /// The clip's source range and keys, whichever lane holds it.
    fn parts(
        project: &mut Project,
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
    ) -> Result<(&mut SourceRange, &mut Keyframes), EditorError> {
        let sequence = project
            .sequence_mut(sequence)
            .ok_or(EditorError::SequenceNotFound(sequence))?;
        if sequence.video_tracks.iter().any(|t| t.id == track) {
            let clip = sequence
                .video_track_mut(track)
                .and_then(|t| t.get_mut(clip))
                .ok_or(EditorError::ClipNotFound(clip))?;
            return Ok((&mut clip.source, &mut clip.keyframes));
        }
        if sequence.audio_tracks.iter().any(|t| t.id == track) {
            let clip = sequence
                .audio_track_mut(track)
                .and_then(|t| t.get_mut(clip))
                .ok_or(EditorError::ClipNotFound(clip))?;
            return Ok((&mut clip.source, &mut clip.keyframes));
        }
        Err(EditorError::TrackNotFound(track))
    }
}

impl EditorCommand for SlipClip {
    fn execute(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (earlier, later) =
            slip_room(project, self.sequence, self.clip).ok_or(EditorError::NothingToSlip)?;
        let offset = self.offset.clamp(earlier, later);
        let (source, keyframes) = Self::parts(project, self.sequence, self.track, self.clip)?;
        let previous = (*source, keyframes.clone());
        *source = SourceRange {
            start: MediaTime::from_ticks(source.start.ticks() + offset),
            end: MediaTime::from_ticks(source.end.ticks() + offset),
        };
        keyframes.shift(offset);
        self.previous = Some(previous);
        Ok(())
    }

    fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let (range, keys) = self.previous.take().ok_or(EditorError::NotExecuted)?;
        let (source, keyframes) = Self::parts(project, self.sequence, self.track, self.clip)?;
        *source = range;
        *keyframes = keys;
        Ok(())
    }

    fn label(&self) -> String {
        "Slip Clip".to_owned()
    }
}

impl Editor {
    /// How far `clip` can slip on the sequence on screen; see [`slip_room`].
    pub fn slip_room(&self, clip: ClipId) -> Option<(i64, i64)> {
        let sequence = self.active_sequence_id().ok()?;
        slip_room(self.project(), sequence, clip)
    }

    /// Play a different part of `clip`'s file without moving it: `offset`
    /// ticks of source later (earlier when negative), held inside the file.
    /// The sound or picture linked to it slips with it, so they stay in sync.
    ///
    /// Nothing happens, and no undo step is made, when the clip is already
    /// against the end of the file it is being slipped towards.
    pub fn slip_clip(&mut self, clip: ClipId, offset: i64) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let (earlier, later) = self.slip_room(clip).ok_or(EditorError::NothingToSlip)?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let mut offset = offset.clamp(earlier, later);

        let partners: Vec<(ClipId, TrackId)> = self
            .linked_with(clip)
            .into_iter()
            .filter(|c| *c != clip)
            .filter_map(|c| Some((c, self.track_of(c)?)))
            .collect();
        // A partner nearer its file's end holds both back: slipping one
        // further than the other would pull sound and picture out of sync.
        for (partner, _) in &partners {
            if let Some((e, l)) = self.slip_room(*partner) {
                offset = offset.clamp(e, l);
            }
        }
        if offset == 0 {
            return Ok(());
        }

        let mut commands = vec![Command::SlipClip {
            sequence,
            track,
            clip,
            offset,
        }];
        for (partner, track) in partners {
            if self.slip_room(partner).is_some() {
                commands.push(Command::SlipClip {
                    sequence,
                    track,
                    clip: partner,
                    offset,
                });
            }
        }
        if commands.len() == 1 {
            return self.dispatch(commands.remove(0));
        }
        self.dispatch_group("Slip Clip".to_owned(), commands)
    }
}
