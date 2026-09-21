//! Fit music to the edit (`Editor::fit_music`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Ten seconds of silent pictures, and a song of `song_seconds` under them.
fn edit(song_seconds: i64) -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Music");
    let shot = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(10),
    ));
    editor.place_media(shot).unwrap();
    let mut song = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.mp3",
        MediaTime::from_seconds(song_seconds),
    );
    song.audio_codec = Some("mp3".to_owned());
    let song = editor.import_media(song);
    let music = editor.place_media(song).unwrap()[0];
    (editor, music)
}

#[test]
fn a_long_song_ends_with_the_pictures_faded_out() {
    let (mut editor, music) = edit(60);
    assert_eq!(editor.picture_end(), Some(seconds(10)));
    let depth = editor.undo_depth();

    let end = editor.fit_music(music).unwrap();
    assert_eq!(end, seconds(10));
    let clip = editor.audio_clip(music).unwrap();
    assert_eq!(clip.timeline.end, seconds(10));
    assert_eq!(clip.fade_out, seconds(2));
    assert_eq!(editor.undo_depth(), depth + 1, "not one step");

    editor.undo().unwrap();
    let clip = editor.audio_clip(music).unwrap();
    assert_eq!(clip.timeline.end, seconds(60));
    assert_eq!(clip.fade_out, TimelineTime::ZERO);
}

/// A shot's own sound is not music.
#[test]
fn a_shots_own_sound_is_refused() {
    let (mut editor, _events) = Editor::new_project("Music");
    let mut shot = MediaAsset::new(
        MediaKind::Video,
        "C:/media/talk.mp4",
        MediaTime::from_seconds(5),
    );
    shot.audio_codec = Some("aac".to_owned());
    let shot = editor.import_media(shot);
    let placed = editor.place_media(shot).unwrap();
    let sound = placed
        .into_iter()
        .find(|c| editor.audio_clip(*c).is_some())
        .unwrap();
    assert!(matches!(
        editor.fit_music(sound),
        Err(EditorError::MusicHasPicture)
    ));
}

/// A song that starts after the pictures end has nothing to fit to.
#[test]
fn a_song_after_the_pictures_is_refused() {
    let (mut editor, music) = edit(60);
    let track = editor.track_of(music).unwrap();
    editor.move_clip(track, track, music, seconds(20)).unwrap();
    assert!(matches!(
        editor.fit_music(music),
        Err(EditorError::NothingToFitTo)
    ));
}

/// A short song already inside the edit only takes the fade, a third of its
/// length at most.
#[test]
fn a_short_song_only_fades() {
    let (mut editor, music) = edit(3);
    editor.fit_music(music).unwrap();
    let clip = editor.audio_clip(music).unwrap();
    assert_eq!(clip.timeline.end, seconds(3));
    assert_eq!(clip.fade_out, seconds(1));
}
