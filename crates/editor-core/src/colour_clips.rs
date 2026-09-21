//! Colour clips: a solid fill or a gradient to put under titles and footage.
//!
//! Made as a generated media entry ([`bettercut_media::Generated`]) so it goes
//! on a picture lane like a photo — trimmed, faded, graded and keyframed with
//! the tools every picture clip already has.

use bettercut_foundation::{ClipId, MediaId, MediaTime, TimelineTime, TrackId};
use bettercut_media::{Generated, MediaAsset};
use bettercut_timeline::{SourceRange, VideoClip, VideoTrack};

use crate::command::{ClipPayload, Command, TrackPayload};
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Put a colour clip at the playhead, as one undo step. Returns the clip.
    ///
    /// It goes on the lowest picture lane with room for it there. When every
    /// lane is taken at the playhead, a new lane is made at the bottom — a
    /// background belongs under the footage, not over it.
    pub fn add_colour_clip(&mut self, colour: Generated) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let (width, height) = (sequence.resolution.width, sequence.resolution.height);
        let start = self.playhead();
        let length = self.project().settings.photo_length;
        let end = start + TimelineTime::from_ticks(length.ticks());
        let free = sequence.video_tracks.iter().find(|track| {
            !track
                .clips()
                .iter()
                .any(|c| c.timeline.start < end && c.timeline.end > start)
        });
        let free = free.map(|track| track.id);

        let media = self.import_media(MediaAsset::generated(colour, width, height));
        let source = SourceRange::new(MediaTime::ZERO, length)?;
        let clip = VideoClip::new(media, start, source)?;
        let id = clip.id;

        let mut commands = Vec::new();
        let track = match free {
            Some(track) => track,
            None => {
                let track = VideoTrack::new("Background");
                let track_id: TrackId = track.id;
                commands.push(Command::InsertTrack {
                    sequence: sequence_id,
                    index: 0,
                    track: TrackPayload::Video(Box::new(track)),
                });
                track_id
            }
        };
        commands.push(Command::AddClip {
            sequence: sequence_id,
            track,
            clip: ClipPayload::Video(Box::new(clip)),
        });
        // Staged, so the clip's lane exists by the time the clip is checked
        // against it.
        self.staged("Add Colour Clip", |editor, stage| {
            for command in commands {
                editor.stage(stage, command)?;
            }
            Ok(())
        })?;
        Ok(id)
    }

    /// The colour a picture clip draws, when it is a colour clip.
    pub fn colour_of(&self, clip: ClipId) -> Option<(MediaId, Generated)> {
        let media = self.video_clip(clip)?.media_id;
        let generated = self.project().media_asset(media)?.generated?;
        Some((media, generated))
    }

    /// Change what colour clip `media` draws, as one undo step. Every clip
    /// made from it changes together. Returns whether anything changed.
    ///
    /// `continuing` folds the change into the one before it, so dragging
    /// around a colour picker is one undo step rather than hundreds.
    pub fn set_colour(
        &mut self,
        media: MediaId,
        colour: Generated,
        continuing: bool,
    ) -> Result<bool, EditorError> {
        let asset = self
            .project()
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?;
        match asset.generated {
            None => Err(EditorError::NotAColourClip),
            Some(was) if was == colour => Ok(false),
            Some(_) => {
                self.dispatch_gesture(
                    "Change Colour".to_owned(),
                    vec![Command::SetColour { media, colour }],
                    continuing,
                )?;
                Ok(true)
            }
        }
    }
}
