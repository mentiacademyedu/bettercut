//! Voiceovers: a recording from the microphone, put on the timeline where the
//! playhead was when recording began.

use std::path::{Path, PathBuf};

use bettercut_foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_timeline::{AudioClip, AudioTrack, SourceRange};

use crate::command::{ClipPayload, Command, TrackPayload};
use crate::editor::Editor;
use crate::error::EditorError;

/// Where recordings for the project at `project` are kept: a folder beside
/// it, like its versions, so moving the project folder takes its voice with
/// it. An unsaved project records into the user's application data.
pub fn voiceover_folder(project: Option<&Path>) -> PathBuf {
    match project {
        Some(project) => {
            let stem = project.file_stem().map_or_else(
                || "project".to_owned(),
                |s| s.to_string_lossy().into_owned(),
            );
            project.with_file_name(format!("{stem} voiceovers"))
        }
        None => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(std::env::temp_dir)
            .join("bettercut")
            .join("voiceovers"),
    }
}

/// The first free `Voiceover N.wav` in `folder`.
pub fn next_voiceover_file(folder: &Path) -> PathBuf {
    (1..)
        .map(|n| folder.join(format!("Voiceover {n}.wav")))
        .find(|path| !path.exists())
        .unwrap_or_else(|| folder.join("Voiceover.wav"))
}

impl Editor {
    /// Put `visualizer` over the sequence on screen, replacing any there, or
    /// take it off with `None` — one undo step, or one per drag when
    /// `continuing`.
    pub fn set_visualizer(
        &mut self,
        visualizer: Option<bettercut_timeline::visualizer::Visualizer>,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        self.dispatch_gesture(
            "Visualizer".to_owned(),
            vec![Command::SetVisualizer {
                sequence,
                visualizer: visualizer.map(Box::new),
            }],
            continuing,
        )
    }

    /// Put `watermark` on the sequence on screen, or take it off with `None` —
    /// one undo step, or one per drag when `continuing`. Refused for a
    /// watermark whose media is not a picture in the project.
    pub fn set_watermark(
        &mut self,
        watermark: Option<bettercut_timeline::watermark::Watermark>,
        continuing: bool,
    ) -> Result<(), EditorError> {
        if let Some(mark) = watermark {
            let asset = self
                .project()
                .media_asset(mark.media)
                .ok_or(EditorError::MediaNotFound(mark.media))?;
            if !asset.kind.has_video() {
                return Err(EditorError::ClipKindMismatch);
            }
        }
        let sequence = self.active_sequence_id()?;
        self.dispatch_gesture(
            "Watermark".to_owned(),
            vec![Command::SetWatermark {
                sequence,
                watermark,
            }],
            continuing,
        )
    }

    /// Import the recording at `path` and put it at `at` on the lowest sound
    /// lane with room for it, adding a "Voiceover" lane when there is none —
    /// one undo step for the placing. Returns the clip.
    pub fn add_voiceover(&mut self, path: &Path, at: TimelineTime) -> Result<ClipId, EditorError> {
        let media = self.import_file(path)?;
        self.place_voiceover(media, at)
    }

    /// [`Self::add_voiceover`] for a file already imported.
    pub fn place_voiceover(
        &mut self,
        media: bettercut_foundation::MediaId,
        at: TimelineTime,
    ) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let length = self
            .project()
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?
            .duration;
        if length <= MediaTime::ZERO {
            return Err(EditorError::NothingRecorded);
        }
        let end = at + TimelineTime::from_ticks(length.ticks());
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        // The targeted lane first, if the take fits on it (§10's track
        // targeting), then the first lane with room, then a new one.
        let targeted = sequence.targeted_track(bettercut_timeline::TrackKind::Audio);
        let free = sequence
            .audio_tracks
            .iter()
            .filter(|track| {
                !track
                    .clips()
                    .iter()
                    .any(|c| c.timeline.start < end && c.timeline.end > at)
            })
            .map(|track| track.id)
            .min_by_key(|id| usize::from(Some(*id) != targeted));
        let index = sequence.audio_tracks.len();

        let clip = AudioClip::new(media, at, SourceRange::new(MediaTime::ZERO, length)?)?;
        let id = clip.id;
        self.staged("Add Voiceover", |editor, stage| {
            let track: TrackId = match free {
                Some(track) => track,
                None => {
                    let track = AudioTrack::new("Voiceover");
                    let track_id = track.id;
                    editor.stage(
                        stage,
                        Command::InsertTrack {
                            sequence: sequence_id,
                            index,
                            track: TrackPayload::Audio(Box::new(track)),
                        },
                    )?;
                    track_id
                }
            };
            editor.stage(
                stage,
                Command::AddClip {
                    sequence: sequence_id,
                    track,
                    clip: ClipPayload::Audio(Box::new(clip)),
                },
            )
        })?;
        Ok(id)
    }
}
