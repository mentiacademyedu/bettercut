//! Putting made sound on the timeline: a line-up tone, a sync beep, silence
//! (`bettercut_media::generated_sound`).
//!
//! These are the three pieces of sound an edit needs that no recording
//! provides. A tone at the head of a delivery says what the levels mean; a
//! beep on the frame the picture mark sits on is how two machines are lined
//! up; a stretch of silence is a gap somebody *chose*, which reads differently
//! from a gap nobody noticed.
//!
//! Placed as an ordinary sound clip on an ordinary sound lane, so every tool
//! that works on sound works on these: trim it, fade it, move it, change its
//! volume, mute it. The only unusual thing about them is that there is no file
//! underneath, which only the mixer ever has to know.

use bettercut_foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_media::{GeneratedSound, MediaAsset};
use bettercut_timeline::{AudioClip, AudioTrack, SourceRange};

use crate::command::{ClipPayload, Command, TrackPayload};
use crate::editor::Editor;
use crate::error::EditorError;

/// How long a sync beep runs: one frame, as it has always been. The frame the
/// beep is on is the frame the picture mark lines up with, and a beep two
/// frames long has two candidates.
pub const BEEP_FRAMES: i64 = 1;

/// How long a made sound can be drawn out to. An hour is longer than anything
/// a tone is ever needed for, and having *some* end keeps a clip's arithmetic
/// ordinary — trimming out past the end of the material still means something.
pub const MADE_SOUND_LENGTH_SECONDS: i64 = 3_600;

impl Editor {
    /// Put made sound at the playhead, `length` long, as one undo step.
    ///
    /// It goes on the first sound lane with room at the playhead, and on a new
    /// lane at the bottom when every lane is busy — a tone laid over a take
    /// would be an edit nobody asked for.
    pub fn add_generated_sound(
        &mut self,
        sound: GeneratedSound,
        length: TimelineTime,
    ) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let start = self.playhead();
        // At least a sample: a clip of no length is one that cannot be
        // selected, trimmed or deleted, which is a worse outcome than a short
        // beep.
        let length = TimelineTime::from_ticks(
            length
                .ticks()
                .max(bettercut_foundation::TICKS_PER_AUDIO_SAMPLE),
        );
        let end = start + length;

        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let free = sequence
            .audio_tracks
            .iter()
            .find(|track| {
                !track.locked
                    && !track
                        .clips()
                        .iter()
                        .any(|clip| clip.timeline.start < end && clip.timeline.end > start)
            })
            .map(|track| track.id);
        // Read now, while the sequence is still borrowed: importing the media
        // below takes the editor mutably.
        let lanes = sequence.audio_tracks.len();

        let media = self.import_media(MediaAsset::generated_sound(
            sound,
            MediaTime::from_seconds(MADE_SOUND_LENGTH_SECONDS),
        ));
        let clip = AudioClip::new(
            media,
            start,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(length.ticks()))?,
        )?;
        let id = clip.id;

        let mut commands = Vec::new();
        let track = match free {
            Some(track) => track,
            None => {
                let track = AudioTrack::new("Tone");
                let track_id: TrackId = track.id;
                commands.push(Command::InsertTrack {
                    sequence: sequence_id,
                    index: lanes,
                    track: TrackPayload::Audio(Box::new(track)),
                });
                track_id
            }
        };
        commands.push(Command::AddClip {
            sequence: sequence_id,
            track,
            clip: ClipPayload::Audio(Box::new(clip)),
        });

        // Staged, so the lane exists by the time the clip is checked against
        // it.
        self.staged(label_for(sound), |editor, stage| {
            for command in commands {
                editor.stage(stage, command)?;
            }
            Ok(())
        })?;
        Ok(id)
    }

    /// A one-frame beep at the playhead, at the sequence's own frame rate.
    pub fn add_sync_beep(&mut self) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let rate = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?
            .frame_rate;
        let frame = bettercut_foundation::ticks_per_frame(rate)
            .unwrap_or(bettercut_foundation::TICKS_PER_SECOND / 30);
        self.add_generated_sound(
            GeneratedSound::LINE_UP,
            TimelineTime::from_ticks(frame * BEEP_FRAMES),
        )
    }

    /// What made sound a clip carries, if it is made sound at all.
    pub fn generated_sound_of(&self, clip: ClipId) -> Option<GeneratedSound> {
        let media = self.audio_clip(clip)?.media_id;
        self.project().media_asset(media)?.generated_sound
    }
}

/// What the undo list calls putting one of these down.
fn label_for(sound: GeneratedSound) -> &'static str {
    match sound {
        GeneratedSound::Tone { .. } => "Add Tone",
        GeneratedSound::Silence => "Add Silence",
        GeneratedSound::Whoosh => "Add Whoosh",
        GeneratedSound::Click => "Add Click",
        GeneratedSound::Riser => "Add Riser",
        GeneratedSound::Pop => "Add Pop",
        GeneratedSound::Ding => "Add Ding",
        GeneratedSound::Boom => "Add Boom",
        GeneratedSound::Chime => "Add Chime",
        GeneratedSound::Shutter => "Add Shutter",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_project_format::Project;

    fn editor() -> Editor {
        Editor::from_project(Project::new("Tone")).0
    }

    fn sound_lanes(editor: &Editor) -> usize {
        editor
            .active_sequence()
            .map_or(0, |sequence| sequence.audio_tracks.len())
    }

    #[test]
    fn a_tone_lands_at_the_playhead_on_a_free_lane() {
        let mut editor = editor();
        editor.set_playhead(TimelineTime::from_seconds(5));
        let lanes = sound_lanes(&editor);

        let clip = editor
            .add_generated_sound(GeneratedSound::LINE_UP, TimelineTime::from_seconds(2))
            .unwrap();

        let placed = editor.audio_clip(clip).unwrap();
        assert_eq!(placed.timeline.start, TimelineTime::from_seconds(5));
        assert_eq!(placed.timeline.end, TimelineTime::from_seconds(7));
        assert_eq!(sound_lanes(&editor), lanes, "a free lane was not used");
        assert_eq!(
            editor.generated_sound_of(clip),
            Some(GeneratedSound::LINE_UP)
        );
    }

    /// A tone must never land on top of a take: when every lane is busy under
    /// the playhead, it gets a lane of its own.
    #[test]
    fn a_busy_playhead_gets_a_new_lane() {
        let mut editor = editor();
        editor
            .add_generated_sound(GeneratedSound::LINE_UP, TimelineTime::from_seconds(2))
            .unwrap();
        let lanes = sound_lanes(&editor);

        // The playhead has not moved, so the first lane is taken.
        let second = editor
            .add_generated_sound(GeneratedSound::Silence, TimelineTime::from_seconds(1))
            .unwrap();

        assert_eq!(sound_lanes(&editor), lanes + 1);
        assert!(editor.audio_clip(second).is_some());
    }

    /// One undo step, lane and all: adding a tone and then undoing must not
    /// leave an empty lane behind.
    #[test]
    fn adding_and_undoing_leaves_nothing_behind() {
        let mut editor = editor();
        editor
            .add_generated_sound(GeneratedSound::LINE_UP, TimelineTime::from_seconds(2))
            .unwrap();
        let lanes = sound_lanes(&editor);
        // A second one, which has to make its own lane.
        editor
            .add_generated_sound(GeneratedSound::Silence, TimelineTime::from_seconds(2))
            .unwrap();
        assert_eq!(sound_lanes(&editor), lanes + 1);

        editor.undo().unwrap();
        assert_eq!(sound_lanes(&editor), lanes, "the new lane stayed");
        let clips: usize = editor
            .active_sequence()
            .unwrap()
            .audio_tracks
            .iter()
            .map(|track| track.clips().len())
            .sum();
        assert_eq!(clips, 1);
    }

    /// A beep is one frame of the sequence's own rate, not a fixed number of
    /// milliseconds: the frame it is on is the frame it marks.
    #[test]
    fn a_sync_beep_is_one_frame_long() {
        let mut editor = editor();
        let rate = editor.active_sequence().unwrap().frame_rate;
        let frame = bettercut_foundation::ticks_per_frame(rate).unwrap();

        let clip = editor.add_sync_beep().unwrap();
        let placed = editor.audio_clip(clip).unwrap();
        assert_eq!(placed.timeline.duration().ticks(), frame);
    }

    /// Silence is a clip like any other, so it can be trimmed, moved and
    /// faded — the whole reason it is placed rather than left as a gap.
    #[test]
    fn silence_is_an_ordinary_clip() {
        let mut editor = editor();
        let clip = editor
            .add_generated_sound(GeneratedSound::Silence, TimelineTime::from_seconds(3))
            .unwrap();

        assert_eq!(
            editor.generated_sound_of(clip),
            Some(GeneratedSound::Silence)
        );
        let placed = editor.audio_clip(clip).unwrap();
        assert_eq!(placed.gain, 1.0, "silence arrived at some other volume");
        assert!(!placed.muted);
    }

    /// A length of nothing would be a clip that cannot be selected or
    /// deleted, so it is held to at least one sample.
    #[test]
    fn a_length_of_nothing_still_makes_a_clip() {
        let mut editor = editor();
        let clip = editor
            .add_generated_sound(GeneratedSound::LINE_UP, TimelineTime::ZERO)
            .unwrap();
        assert!(editor.audio_clip(clip).unwrap().timeline.duration().ticks() > 0);
    }
}
