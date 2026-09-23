//! A count-in leader at the head of the sequence: black, a countdown, and a
//! beep on every second.
//!
//! What goes in front of a delivery, or a piece that is going to be played in
//! alongside something else — a band, a broadcast, a second screen. The
//! numbers say how long until the programme starts; the beeps are what a
//! machine, or a drummer, actually lines up against.
//!
//! Made *on the timeline*, out of pieces the editor already has, rather than
//! bolted onto the export: the gap is `insert_time`, the picture is a colour
//! clip, the numbers are the counter title, the beeps are the one-frame sync
//! beep. That is what keeps it honest (§46) — the preview shows exactly what
//! the export will write, because they are reading the same clips — and what
//! keeps it optional: it is one undo step, and every piece of it is an
//! ordinary clip that can be trimmed, moved or deleted like any other.

use bettercut_foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_media::{Generated, GeneratedSound, MediaAsset};
use bettercut_timeline::{AudioClip, SourceRange, VideoClip};

use crate::command::{ClipPayload, Command};
use crate::editor::Editor;
use crate::error::EditorError;

/// How long a count-in runs unless told otherwise. Five is what a person can
/// read as it goes past; ten is a wait.
pub const DEFAULT_COUNT_IN_SECONDS: i64 = 5;

/// The longest count-in offered. Past this it is not a count-in but a leader
/// somebody will have to sit through.
pub const MAX_COUNT_IN_SECONDS: i64 = 10;

/// What a count-in put down, so the caller can select or name its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountIn {
    /// The black behind the numbers, on the first picture lane.
    pub black: ClipId,
    /// The numbers, on the first title lane.
    pub countdown: ClipId,
    /// One beep a second, on the first sound lane, earliest first.
    pub beeps: Vec<ClipId>,
}

impl Editor {
    /// Put a count-in of `seconds` at the head of the sequence, as one undo
    /// step: everything already there moves later by that much.
    ///
    /// `seconds` is held to 1–[`MAX_COUNT_IN_SECONDS`]. Needs a picture lane,
    /// a sound lane and a title lane, which every sequence starts with.
    pub fn add_count_in(&mut self, seconds: i64) -> Result<CountIn, EditorError> {
        let seconds = seconds.clamp(1, MAX_COUNT_IN_SECONDS);
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let picture = sequence
            .video_tracks
            .first()
            .map(|track| track.id)
            .ok_or(EditorError::NoTrackForMedia)?;
        let sound = sequence
            .audio_tracks
            .first()
            .map(|track| track.id)
            .ok_or(EditorError::NoTrackForMedia)?;
        let titles = sequence
            .text_tracks
            .first()
            .map(|track| track.id)
            .ok_or(EditorError::NoTextTrack)?;
        let (width, height) = (sequence.resolution.width, sequence.resolution.height);
        let frame = bettercut_foundation::ticks_per_frame(sequence.frame_rate)
            .unwrap_or(bettercut_foundation::TICKS_PER_SECOND / 30);
        let length = TimelineTime::from_seconds(seconds);

        // The media behind the black and the beeps, made once each. Media is
        // not part of the undo step — a colour clip's is not either — and an
        // unused entry costs nothing.
        let black_media = self.import_media(MediaAsset::generated(
            Generated::solid([0, 0, 0]),
            width,
            height,
        ));
        let beep_media = self.import_media(MediaAsset::generated_sound(
            GeneratedSound::LINE_UP,
            MediaTime::from_seconds(crate::tone::MADE_SOUND_LENGTH_SECONDS),
        ));

        let black = VideoClip::new(
            black_media,
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(length.ticks()))?,
        )?;
        let mut countdown =
            bettercut_timeline::TextClip::with_duration("Count-in", TimelineTime::ZERO, length)?;
        countdown.style = bettercut_text::TextStyle::title(bettercut_text::TitleLook::Headline);
        countdown.counter = Some(bettercut_timeline::Counter::countdown(length));
        let beeps: Vec<AudioClip> = (0..seconds)
            .map(|second| {
                AudioClip::new(
                    beep_media,
                    TimelineTime::from_seconds(second),
                    SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(frame))?,
                )
            })
            .collect::<Result<_, _>>()?;

        let placed = CountIn {
            black: black.id,
            countdown: countdown.id,
            beeps: beeps.iter().map(|beep| beep.id).collect(),
        };

        self.staged("Add Count-In", |editor, stage| {
            // The room first: everything from the head moves later, so the
            // clips below land on empty lanes.
            editor.stage_insert_time(stage, sequence_id, TimelineTime::ZERO, length)?;
            editor.stage(
                stage,
                Command::AddClip {
                    sequence: sequence_id,
                    track: picture,
                    clip: ClipPayload::Video(Box::new(black)),
                },
            )?;
            editor.stage(
                stage,
                Command::AddText {
                    sequence: sequence_id,
                    track: titles,
                    clip: Box::new(countdown),
                },
            )?;
            for beep in beeps {
                editor.stage(
                    stage,
                    Command::AddClip {
                        sequence: sequence_id,
                        track: sound,
                        clip: ClipPayload::Audio(Box::new(beep)),
                    },
                )?;
            }
            Ok(())
        })?;
        Ok(placed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::MediaId;
    use bettercut_project_format::Project;

    fn editor() -> Editor {
        Editor::from_project(Project::new("Count")).0
    }

    /// A ten-second shot from the head of the first picture lane.
    fn with_a_shot() -> (Editor, ClipId) {
        let mut editor = editor();
        let clip = VideoClip::new(
            MediaId::new(),
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
        )
        .unwrap();
        let id = clip.id;
        let track = editor.active_sequence().unwrap().video_tracks[0].id;
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .unwrap();
        (editor, id)
    }

    #[test]
    fn a_count_in_pushes_the_edit_later_by_its_length() {
        let (mut editor, shot) = with_a_shot();
        editor.add_count_in(5).unwrap();

        let moved = editor.video_clip(shot).unwrap();
        assert_eq!(moved.timeline.start, TimelineTime::from_seconds(5));
        assert_eq!(moved.timeline.end, TimelineTime::from_seconds(15));
    }

    /// Black under the numbers for the whole count, then the shot.
    #[test]
    fn the_black_covers_the_count_and_nothing_more() {
        let (mut editor, _) = with_a_shot();
        let count_in = editor.add_count_in(3).unwrap();

        let black = editor.video_clip(count_in.black).unwrap();
        assert_eq!(black.timeline.start, TimelineTime::ZERO);
        assert_eq!(black.timeline.end, TimelineTime::from_seconds(3));
        let media = editor.project().media_asset(black.media_id).unwrap();
        assert_eq!(media.generated, Some(Generated::solid([0, 0, 0])));
    }

    /// The numbers count down from the length to nothing, over the length.
    #[test]
    fn the_numbers_count_down_the_whole_way() {
        let mut editor = editor();
        let count_in = editor.add_count_in(4).unwrap();

        let title = editor.text_clip(count_in.countdown).unwrap();
        assert_eq!(title.timeline.start, TimelineTime::ZERO);
        assert_eq!(title.timeline.end, TimelineTime::from_seconds(4));
        let counter = title.counter.expect("the title is not a counter");
        assert_eq!(counter.direction, bettercut_timeline::CountDirection::Down);
        assert_eq!(counter.from, TimelineTime::from_seconds(4));
    }

    /// One beep a second, each one frame long, on the first sound lane: what
    /// a machine lines up against.
    #[test]
    fn there_is_one_beep_on_every_second() {
        let mut editor = editor();
        let rate = editor.active_sequence().unwrap().frame_rate;
        let frame = bettercut_foundation::ticks_per_frame(rate).unwrap();
        let count_in = editor.add_count_in(5).unwrap();

        assert_eq!(count_in.beeps.len(), 5);
        for (second, beep) in count_in.beeps.iter().enumerate() {
            let clip = editor.audio_clip(*beep).unwrap();
            assert_eq!(
                clip.timeline.start,
                TimelineTime::from_seconds(second as i64)
            );
            assert_eq!(clip.timeline.duration().ticks(), frame);
            assert_eq!(
                editor.generated_sound_of(*beep),
                Some(GeneratedSound::LINE_UP)
            );
        }
    }

    /// The whole thing is one undo step: the shot goes back where it was and
    /// none of the pieces stay behind.
    #[test]
    fn undo_takes_the_whole_count_in_away() {
        let (mut editor, shot) = with_a_shot();
        let count_in = editor.add_count_in(5).unwrap();
        editor.undo().unwrap();

        assert_eq!(
            editor.video_clip(shot).unwrap().timeline.start,
            TimelineTime::ZERO
        );
        assert!(editor.video_clip(count_in.black).is_none());
        assert!(editor.text_clip(count_in.countdown).is_none());
        assert!(
            count_in
                .beeps
                .iter()
                .all(|b| editor.audio_clip(*b).is_none())
        );
    }

    /// Held to something a person would sit through, and never to nothing.
    #[test]
    fn the_length_is_held_to_its_range() {
        let mut editor = editor();
        assert_eq!(editor.add_count_in(0).unwrap().beeps.len(), 1);
        let mut another = super::tests::editor();
        assert_eq!(
            another.add_count_in(99).unwrap().beeps.len(),
            MAX_COUNT_IN_SECONDS as usize
        );
    }

    /// A mark on the edit moves with the edit, or a beat grid would point at
    /// the count-in's black.
    #[test]
    fn marks_move_with_the_edit() {
        let mut editor = editor();
        editor
            .add_markers(&[TimelineTime::from_seconds(2)])
            .unwrap();
        editor.add_count_in(3).unwrap();

        let marks = editor.markers();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].time, TimelineTime::from_seconds(5));
    }
}
