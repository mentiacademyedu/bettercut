//! Several picture clips made one length (`Editor::set_clip_lengths`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};

#[test]
fn photos_grow_and_shrink_and_the_lane_follows_but_the_music_stays() {
    let (mut editor, _events) = Editor::new_project("Slideshow");
    let photo = editor.import_media(bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Image,
        "C:/media/still.png",
        MediaTime::ZERO,
    ));
    let song = editor.import_media(bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Audio,
        "C:/media/song.wav",
        MediaTime::from_seconds(60),
    ));
    let sequence = editor.active_sequence().unwrap();
    let (pictures, sounds) = (sequence.video_tracks[0].id, sequence.audio_tracks[0].id);
    let at = TimelineTime::from_seconds;
    let two = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap();
    let mut photos = Vec::new();
    for index in 0..3 {
        let clip = VideoClip::new(photo, at(index * 2), two).unwrap();
        photos.push(clip.id);
        editor
            .add_clip(pictures, ClipPayload::Video(Box::new(clip)))
            .unwrap();
    }
    let music_range = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(6)).unwrap();
    let music = AudioClip::new(song, TimelineTime::ZERO, music_range).unwrap();
    let music_id = music.id;
    editor
        .add_clip(sounds, ClipPayload::Audio(Box::new(music)))
        .unwrap();
    let depth = editor.undo_depth();

    // The first two grow to 3 s: the third moves along by 2 s.
    assert_eq!(editor.set_clip_lengths(&photos[..2], at(3)).unwrap(), 2);
    let starts: Vec<_> = photos
        .iter()
        .map(|p| editor.clip_start(*p).unwrap())
        .collect();
    assert_eq!(starts, [at(0), at(3), at(6)]);
    assert_eq!(
        editor.clip_end(photos[2]),
        Some(at(8)),
        "the third kept its length"
    );
    assert_eq!(editor.clip_end(music_id), Some(at(6)), "the music moved");
    assert_eq!(editor.undo_depth(), depth + 1, "one step");

    // All three to 1 s: everything comes back up.
    assert_eq!(editor.set_clip_lengths(&photos, at(1)).unwrap(), 3);
    let starts: Vec<_> = photos
        .iter()
        .map(|p| editor.clip_start(*p).unwrap())
        .collect();
    assert_eq!(starts, [at(0), at(1), at(2)]);
    assert_eq!(editor.clip_end(photos[2]), Some(at(3)));

    assert_eq!(
        editor.set_clip_lengths(&photos, at(1)).unwrap(),
        0,
        "already so"
    );
}

#[test]
fn titles_take_a_length_too() {
    let (mut editor, _events) = Editor::new_project("Titles");
    let first = editor.add_text("One").unwrap();
    // Placed in the first free slot: straight after the first.
    let second = editor.add_text("Two").unwrap();
    let before = editor.clip_start(second).unwrap();

    assert_eq!(
        editor
            .set_clip_lengths(&[first], TimelineTime::from_seconds(5))
            .unwrap(),
        1
    );
    assert_eq!(
        editor.clip_end(first),
        Some(editor.clip_start(first).unwrap() + TimelineTime::from_seconds(5))
    );
    // The next title on the lane moved along by the difference.
    assert!(editor.clip_start(second).unwrap() > before);
}
