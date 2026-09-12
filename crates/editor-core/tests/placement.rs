//! Putting imported media on the timeline (§12).
//!
//! §8 keeps picture and sound on separate tracks, so a video file is *two*
//! clips. Placing only the picture is how an editor ends up exporting silent
//! video, and placing an audio file as a video clip is how a track ends up
//! holding something that cannot be drawn — both of which this used to do.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn editor() -> Editor {
    let (editor, _rx) = Editor::new_project("Placement");
    editor
}

/// A video file with a sound track, as `probe` would report one.
fn video_with_sound(name: &str) -> MediaAsset {
    let mut asset = MediaAsset::new(MediaKind::Video, name, MediaTime::from_seconds(10));
    asset.audio_codec = Some("aac".to_owned());
    asset.audio_sample_rate = Some(48_000);
    asset.audio_channels = Some(2);
    asset
}

fn silent_video(name: &str) -> MediaAsset {
    MediaAsset::new(MediaKind::Video, name, MediaTime::from_seconds(10))
}

fn music(name: &str) -> MediaAsset {
    let mut asset = MediaAsset::new(MediaKind::Audio, name, MediaTime::from_seconds(30));
    asset.audio_codec = Some("mp3".to_owned());
    asset
}

fn photo(name: &str) -> MediaAsset {
    // As `probe` reports one: a picture with no duration.
    MediaAsset::new(MediaKind::Image, name, MediaTime::ZERO)
}

#[test]
fn a_photo_places_for_five_seconds_on_the_picture_track() {
    let mut editor = editor();
    let media = editor.import_media(photo("C:/media/p.jpg"));

    let clips = editor.place_media(media).unwrap();
    assert_eq!(clips.len(), 1, "a photo has no sound to place");

    let sequence = editor.active_sequence().unwrap();
    let clip = &sequence.video_tracks[0].clips()[0];
    assert_eq!(
        clip.timeline.end,
        TimelineTime::from_ticks(bettercut_editor_core::media::STILL_DURATION.ticks())
    );
    assert_eq!(sequence.audio_tracks[0].len(), 0);
}

/// A photo has no end to run out of.
#[test]
fn a_photo_can_be_dragged_out_as_long_as_wanted() {
    let mut editor = editor();
    let media = editor.import_media(photo("C:/media/p.jpg"));
    let clip = editor.place_media(media).unwrap()[0];
    let track = editor.track_of(clip).unwrap();

    editor
        .trim_clip(
            track,
            clip,
            bettercut_editor_core::TrimEdge::End,
            TimelineTime::from_seconds(60),
        )
        .unwrap();

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.video_tracks[0].clips()[0].timeline.end,
        TimelineTime::from_seconds(60)
    );
}

/// A crossfade reads past both clips' edges, which a video trimmed to its
/// file's start cannot supply — but a photo shows the same picture there too.
#[test]
fn photos_crossfade_without_spare_footage() {
    let mut editor = editor();
    let first = editor.import_media(photo("C:/media/a.jpg"));
    let second = editor.import_media(photo("C:/media/b.jpg"));
    let a = editor.place_media(first).unwrap()[0];
    editor.place_media(second).unwrap();

    let room = editor
        .transition_room(
            a,
            bettercut_editor_core::timeline::TransitionKind::Crossfade,
        )
        .expect("there is a cut");
    assert_eq!(
        room,
        TimelineTime::from_ticks(bettercut_editor_core::media::STILL_DURATION.ticks()),
        "as long as the shots themselves"
    );
    editor
        .set_transition(
            a,
            bettercut_editor_core::timeline::TransitionKind::Crossfade,
        )
        .expect("a crossfade between photos");
}

#[test]
fn a_video_with_sound_places_both_halves() {
    let mut editor = editor();
    let media = editor.import_media(video_with_sound("C:/media/a.mp4"));

    let clips = editor.place_media(media).unwrap();
    assert_eq!(clips.len(), 2, "a video with sound is two clips");

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 1, "no picture");
    assert_eq!(sequence.audio_tracks[0].len(), 1, "no sound");
}

/// The pair has to start together. Started from each track's own end, a video's
/// sound would land somewhere other than its picture the moment the two tracks
/// had different lengths.
#[test]
fn the_picture_and_the_sound_start_together() {
    let mut editor = editor();

    // Something on the audio track first, so the two tracks are unequal.
    let bed = editor.import_media(music("C:/media/bed.mp3"));
    editor.place_media(bed).unwrap();

    let media = editor.import_media(video_with_sound("C:/media/a.mp4"));
    editor.place_media(media).unwrap();

    let sequence = editor.active_sequence().unwrap();
    let picture = sequence.video_tracks[0].clips()[0].timeline.start;
    let sound = sequence.audio_tracks[0]
        .clips()
        .iter()
        .find(|c| c.media_id == media)
        .expect("the video's sound")
        .timeline
        .start;

    assert_eq!(picture, sound, "the sound did not land under the picture");
    assert_eq!(
        picture,
        TimelineTime::from_seconds(30),
        "it should follow whichever track was longer"
    );
}

/// An audio file is sound only. Placed as a video clip it would sit on a video
/// track producing nothing to draw.
#[test]
fn an_audio_file_places_only_sound() {
    let mut editor = editor();
    let media = editor.import_media(music("C:/media/song.mp3"));

    let clips = editor.place_media(media).unwrap();
    assert_eq!(clips.len(), 1);

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.video_tracks[0].len(),
        0,
        "sound went on the picture track"
    );
    assert_eq!(sequence.audio_tracks[0].len(), 1);
}

/// A video with no sound track places one clip, not an empty audio clip beside
/// it — there is nothing in the file to play.
#[test]
fn a_silent_video_places_only_picture() {
    let mut editor = editor();
    let media = editor.import_media(silent_video("C:/media/silent.mp4"));

    assert_eq!(editor.place_media(media).unwrap().len(), 1);
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.audio_tracks[0].len(), 0);
}

/// "Add this file" is one thing the user did (§79), so it is one undo — not
/// "undo the sound, then undo the picture".
#[test]
fn placing_a_video_is_a_single_undo_step() {
    let mut editor = editor();
    let media = editor.import_media(video_with_sound("C:/media/a.mp4"));
    editor.place_media(media).unwrap();

    editor.undo().unwrap();

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 0);
    assert_eq!(sequence.audio_tracks[0].len(), 0, "one undo left the sound");

    editor.redo().unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 1);
    assert_eq!(sequence.audio_tracks[0].len(), 1);
}

/// Placing several files stacks them end to end rather than on top of each
/// other — §8's tracks do not overlap, and an import that refused itself would
/// be a poor way to find that out.
#[test]
fn several_files_stack_up_without_overlapping() {
    let mut editor = editor();
    for index in 0..3 {
        let media = editor.import_media(video_with_sound(&format!("C:/media/{index}.mp4")));
        editor.place_media(media).unwrap();
    }

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 3);
    assert_eq!(sequence.audio_tracks[0].len(), 3);

    for track_clips in [
        sequence.video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline)
            .collect::<Vec<_>>(),
        sequence.audio_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline)
            .collect::<Vec<_>>(),
    ] {
        for pair in track_clips.windows(2) {
            assert!(pair[0].end <= pair[1].start, "clips overlap");
        }
    }
}

/// Media that is not in the project is not something to place.
#[test]
fn placing_unknown_media_is_refused() {
    let mut editor = editor();
    assert!(
        editor
            .place_media(bettercut_editor_core::foundation::MediaId::new())
            .is_err()
    );
}

/// §12: a video file is a picture clip and a sound clip, and §51's speed
/// control has to treat them as one thing. Re-timing the picture alone is how
/// the two drift apart the first time anyone touches the slider.
#[test]
fn re_timing_a_video_re_times_its_sound_with_it() {
    use bettercut_editor_core::foundation::Rational;

    let mut editor = editor();
    let media = editor.import_media(video_with_sound("C:/media/a.mp4"));
    let clips = editor.place_media(media).unwrap();
    let picture = clips[0];

    let double = Rational::new(2, 1).unwrap();
    editor.set_clip_speed(picture, double, false).unwrap();

    let sequence = editor.active_sequence().unwrap();
    let video = &sequence.video_tracks[0].clips()[0];
    let audio = &sequence.audio_tracks[0].clips()[0];

    assert_eq!(video.speed, double);
    assert_eq!(audio.speed, double, "the sound was left at normal speed");
    assert_eq!(
        video.timeline, audio.timeline,
        "picture and sound ended up different lengths"
    );
}

/// And re-timing from the *sound* side does the same, because the link is
/// symmetric — selecting the audio clip is just as likely as selecting the
/// video one.
#[test]
fn re_timing_the_sound_re_times_the_picture() {
    use bettercut_editor_core::foundation::Rational;

    let mut editor = editor();
    let media = editor.import_media(video_with_sound("C:/media/a.mp4"));
    let clips = editor.place_media(media).unwrap();
    let sound = clips[1];

    editor
        .set_clip_speed(sound, Rational::new(1, 2).unwrap(), false)
        .unwrap();

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.video_tracks[0].clips()[0].timeline,
        sequence.audio_tracks[0].clips()[0].timeline
    );
}

/// One gesture, one undo (§79) — not "undo the sound, then undo the picture".
#[test]
fn re_timing_a_linked_pair_is_a_single_undo_step() {
    use bettercut_editor_core::foundation::Rational;

    let mut editor = editor();
    let media = editor.import_media(video_with_sound("C:/media/a.mp4"));
    let picture = editor.place_media(media).unwrap()[0];
    editor
        .set_clip_speed(picture, Rational::new(4, 1).unwrap(), false)
        .unwrap();

    editor.undo().unwrap();

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].clips()[0].speed, Rational::ONE);
    assert_eq!(
        sequence.audio_tracks[0].clips()[0].speed,
        Rational::ONE,
        "one undo left the sound re-timed"
    );
}

/// A clip with no sound beside it is not linked to anything, and re-timing it
/// must still work.
#[test]
fn an_unlinked_clip_re_times_alone() {
    use bettercut_editor_core::foundation::Rational;

    let mut editor = editor();
    let media = editor.import_media(silent_video("C:/media/silent.mp4"));
    let picture = editor.place_media(media).unwrap()[0];

    assert_eq!(editor.linked_with(picture), vec![picture]);
    editor
        .set_clip_speed(picture, Rational::new(2, 1).unwrap(), false)
        .unwrap();
    assert_eq!(
        editor.video_clip(picture).unwrap().speed,
        Rational::new(2, 1).unwrap()
    );
}
