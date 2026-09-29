//! Bouncing a sound lane: everything on it mixed down to one clip, in place.
//!
//! A lane carrying thirty voice clips, each with its own level, denoise, EQ and
//! fades, costs thirty decoders and thirty chains of processing every block. A
//! lane carrying one bounced clip costs one of each. That is the point of it —
//! and the other half of the point is that the thirty small decisions stop
//! being thirty things that can be knocked out of place by accident.
//!
//! The mixing itself is [`bettercut_export::BounceJob`], which is the export's
//! own mixer with every other lane silenced (§46). What lives here is what
//! happens either side: where the file goes, and putting it down.
//!
//! # What it costs
//!
//! It is one undo step, and it is a real edit: the clips are gone, their
//! effects are baked into the file, and sound that was linked to a picture
//! (§12) is no longer linked to anything — moving that shot will not move this
//! sound any more. The lane's own gain, pan and volume line are reset as the
//! clip goes down, because they were applied on the way into the file and
//! applying them again would be applying them twice.

use std::path::{Path, PathBuf};

use bettercut_foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_timeline::{AudioClip, SourceRange};

use crate::command::{ClipPayload, Command};
use crate::editor::Editor;
use crate::error::EditorError;

/// Where bounces for the project at `project` are kept: a folder beside it, as
/// its voiceovers are, so moving the project folder takes its sound with it.
///
/// Not the cache: a bounce is part of the edit from the moment it is made, and
/// a cache is a place things are deleted from.
pub fn bounce_folder(project: Option<&Path>) -> PathBuf {
    match project {
        Some(project) => {
            let stem = project.file_stem().map_or_else(
                || "project".to_owned(),
                |s| s.to_string_lossy().into_owned(),
            );
            project.with_file_name(format!("{stem} bounces"))
        }
        None => bettercut_foundation::places::local_home().join("bounces"),
    }
}

/// The first free `<name> bounce N.wav` in `folder`.
///
/// Numbered rather than overwritten: bouncing a lane twice is usually a
/// mistake being corrected, and the first file may still be under an undo.
pub fn next_bounce_file(folder: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let safe = safe.trim_matches('-');
    let stem = if safe.is_empty() { "Track" } else { safe };
    (1..)
        .map(|n| folder.join(format!("{stem} bounce {n}.wav")))
        .find(|path| !path.exists())
        .unwrap_or_else(|| folder.join(format!("{stem} bounce.wav")))
}

impl Editor {
    /// Where a bounce of `track` should be written. The folder is made by the
    /// caller, which is the only part that touches the disk.
    pub fn bounce_file(&self, track: TrackId) -> PathBuf {
        let name = self
            .active_sequence()
            .and_then(|sequence| sequence.track_name(track))
            .unwrap_or("Track")
            .to_owned();
        next_bounce_file(&bounce_folder(self.path()), &name)
    }

    /// Whether `track` has anything to bounce: it is a sound lane, it is not
    /// locked, and there is more than nothing on it.
    pub fn can_bounce(&self, track: TrackId) -> bool {
        self.active_sequence().is_some_and(|sequence| {
            sequence
                .audio_track(track)
                .is_some_and(|lane| !lane.locked && !lane.clips().is_empty())
        })
    }

    /// Put a finished bounce down on `track`, in place of everything that was
    /// there, starting at `at`. One undo step.
    ///
    /// The file is imported like any other sound, so it shows in the media
    /// browser: the edit depends on it now, and a file the project needs but
    /// never lists is a file someone deletes.
    pub fn place_bounce(
        &mut self,
        track: TrackId,
        path: &Path,
        at: TimelineTime,
    ) -> Result<ClipId, EditorError> {
        let sequence = self.active_sequence_id()?;
        let media = self.import_file(path)?;
        let length = self
            .project()
            .media_asset(media)
            .map(|asset| asset.duration)
            .filter(|length| *length > MediaTime::ZERO)
            .ok_or(EditorError::NothingRecorded)?;

        let existing: Vec<ClipId> = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence))?
            .audio_track(track)
            .ok_or(EditorError::TrackNotFound(track))?
            .clips()
            .iter()
            .map(|clip| clip.id)
            .collect();

        let clip = AudioClip::new(media, at, SourceRange::new(MediaTime::ZERO, length)?)?;
        let id = clip.id;
        self.staged("Bounce Track", |editor, stage| {
            for old in existing {
                editor.stage(
                    stage,
                    Command::RemoveClip {
                        sequence,
                        track,
                        clip: old,
                    },
                )?;
            }
            editor.stage(
                stage,
                Command::AddClip {
                    sequence,
                    track,
                    clip: ClipPayload::Audio(Box::new(clip)),
                },
            )?;
            // The lane's level is in the file now. Left where it was, it would
            // be applied to sound it has already been applied to.
            editor.stage(
                stage,
                Command::SetTrackMix {
                    sequence,
                    track,
                    gain: 1.0,
                    pan: 0.0,
                },
            )?;
            editor.stage(
                stage,
                Command::SetTrackVolume {
                    sequence,
                    track,
                    points: Vec::new(),
                },
            )?;
            Ok(())
        })?;
        Ok(id)
    }
}
