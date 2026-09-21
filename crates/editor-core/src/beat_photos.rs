//! Photos cut to the beat: one photo from each marker to the next, so a pile of
//! pictures changes exactly on the music's beats — mark the beats of a song,
//! pick the photos, done. One undo step.

use bettercut_foundation::{ClipId, MediaId, MediaTime, TimelineTime, TrackId};
use bettercut_timeline::{Movement, SourceRange, TimelineRange, VideoClip, VideoTrack};

use crate::command::{ClipPayload, Command, TrackPayload};
use crate::editor::Editor;
use crate::error::EditorError;

/// The shortest a photo may be held between two beats. Markers closer than
/// this are skipped over rather than flashing a picture by.
pub const MIN_BEAT_PHOTO: TimelineTime = TimelineTime::from_ticks(96_000);

impl Editor {
    /// Put `photos`, in order, one per beat: each from a marker to the next.
    /// The last photo runs one beat's length past its marker. Returns the
    /// clips; stops at whichever runs out first, photos or beats.
    ///
    /// On the lowest picture lane with room for the whole run, or a new one on
    /// top. Refused without at least two markers to time from, or with no
    /// photos among `photos`.
    pub fn photos_to_beats(
        &mut self,
        photos: &[MediaId],
        movement: Movement,
    ) -> Result<Vec<ClipId>, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let stills: Vec<MediaId> = photos
            .iter()
            .copied()
            .filter(|id| {
                self.project()
                    .media_asset(*id)
                    .is_some_and(|a| a.is_still())
            })
            .collect();
        if stills.is_empty() {
            return Err(EditorError::NothingToCutToBeats);
        }

        // Beats far enough apart to hold a photo.
        let mut beats: Vec<TimelineTime> = Vec::new();
        for marker in self.markers() {
            if beats
                .last()
                .is_none_or(|last| marker.time.ticks() - last.ticks() >= MIN_BEAT_PHOTO.ticks())
            {
                beats.push(marker.time);
            }
        }
        if beats.len() < 2 {
            return Err(EditorError::NotEnoughBeats);
        }
        let last_gap = beats[beats.len() - 1].ticks() - beats[beats.len() - 2].ticks();

        let mut spans = Vec::new();
        for (index, media) in stills.iter().enumerate() {
            let Some(&start) = beats.get(index) else {
                break;
            };
            let end = beats
                .get(index + 1)
                .copied()
                .unwrap_or(TimelineTime::from_ticks(start.ticks() + last_gap));
            spans.push((*media, TimelineRange { start, end }));
        }
        let (Some(first), Some(last)) = (spans.first(), spans.last()) else {
            return Err(EditorError::NothingToCutToBeats);
        };
        let whole = TimelineRange {
            start: first.1.start,
            end: last.1.end,
        };

        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let free = sequence
            .video_tracks
            .iter()
            .find(|track| !track.clips().iter().any(|c| c.timeline.overlaps(whole)))
            .map(|track| track.id);
        let index = sequence.video_tracks.len();

        let mut clips = Vec::with_capacity(spans.len());
        for (media, span) in &spans {
            let length = MediaTime::from_ticks(span.end.ticks() - span.start.ticks());
            let source = SourceRange::new(MediaTime::ZERO, length)?;
            let mut clip = VideoClip::new(*media, span.start, source)?;
            // The same keys the Movement buttons write, so each photo drifts
            // across its own beat.
            for (parameter, key) in
                movement.keyframes(clip.transform.scale, clip.transform.position, source)
            {
                clip.keyframes.set(parameter, key);
            }
            clips.push(clip);
        }
        let ids: Vec<ClipId> = clips.iter().map(|c| c.id).collect();

        self.staged("Photos to the Beat", |editor, stage| {
            let track: TrackId = match free {
                Some(track) => track,
                None => {
                    let track = VideoTrack::new("Beat Photos");
                    let id = track.id;
                    editor.stage(
                        stage,
                        Command::InsertTrack {
                            sequence: sequence_id,
                            index,
                            track: TrackPayload::Video(Box::new(track)),
                        },
                    )?;
                    id
                }
            };
            for clip in clips {
                editor.stage(
                    stage,
                    Command::AddClip {
                        sequence: sequence_id,
                        track,
                        clip: ClipPayload::Video(Box::new(clip)),
                    },
                )?;
            }
            Ok(())
        })?;
        Ok(ids)
    }
}
