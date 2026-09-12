//! Fading a clip's sound in and out.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::MAX_FADE;
use bettercut_editor_core::{Editor, EditorError};

fn millis(ms: i64) -> TimelineTime {
    TimelineTime::from_millis(ms)
}

fn with_video(sound: bool) -> (Editor, Vec<bettercut_editor_core::foundation::ClipId>) {
    let (mut editor, _events) = Editor::new_project("Fades");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(10),
    );
    if sound {
        asset.audio_codec = Some("aac".to_owned());
    }
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed)
}

#[test]
fn a_fade_is_set_and_undone() {
    let (mut editor, placed) = with_video(true);
    let sound = placed[1];

    editor
        .set_clip_fades(sound, millis(500), millis(2000), false)
        .unwrap();
    let clip = editor.audio_clip(sound).unwrap();
    assert_eq!((clip.fade_in, clip.fade_out), (millis(500), millis(2000)));

    editor.undo().unwrap();
    let clip = editor.audio_clip(sound).unwrap();
    assert_eq!(
        (clip.fade_in, clip.fade_out),
        (TimelineTime::ZERO, TimelineTime::ZERO)
    );
}

/// Selecting the picture and fading means the picture's sound (§12).
#[test]
fn fading_a_picture_fades_its_sound() {
    let (mut editor, placed) = with_video(true);
    let (picture, sound) = (placed[0], placed[1]);

    editor
        .set_clip_fades(picture, TimelineTime::ZERO, millis(1000), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().fade_out, millis(1000));
}

#[test]
fn a_clip_with_no_sound_cannot_fade() {
    let (mut editor, placed) = with_video(false);
    let err = editor
        .set_clip_fades(placed[0], millis(500), TimelineTime::ZERO, false)
        .unwrap_err();
    assert!(matches!(err, EditorError::ClipKindMismatch), "{err}");
}

#[test]
fn a_fade_is_kept_in_range() {
    let (mut editor, placed) = with_video(true);
    editor
        .set_clip_fades(
            placed[1],
            millis(-5),
            TimelineTime::from_seconds(600),
            false,
        )
        .unwrap();
    let clip = editor.audio_clip(placed[1]).unwrap();
    assert_eq!(clip.fade_in, TimelineTime::ZERO);
    assert_eq!(clip.fade_out, MAX_FADE);
}

/// A drag is one undo step (§11).
#[test]
fn dragging_a_fade_is_one_undo_step() {
    let (mut editor, placed) = with_video(true);
    let depth = editor.undo_depth();
    for (i, ms) in [100, 200, 300, 400].into_iter().enumerate() {
        editor
            .set_clip_fades(placed[1], millis(ms), TimelineTime::ZERO, i > 0)
            .unwrap();
    }
    assert_eq!(editor.undo_depth(), depth + 1);
    editor.undo().unwrap();
    assert_eq!(
        editor.audio_clip(placed[1]).unwrap().fade_in,
        TimelineTime::ZERO
    );
}

/// Fades survive the journal and a crash (§38.2).
#[test]
fn a_fade_survives_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fades.vproj");
    let sound;
    {
        let (mut editor, placed) = with_video(true);
        sound = placed[1];
        editor.save_as(&path).unwrap();
        editor
            .set_clip_fades(sound, millis(300), millis(700), false)
            .unwrap();
        std::mem::forget(editor);
    }
    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).unwrap();
    let clip = session
        .project
        .active()
        .unwrap()
        .audio_tracks
        .iter()
        .find_map(|t| t.get(sound))
        .unwrap();
    assert_eq!((clip.fade_in, clip.fade_out), (millis(300), millis(700)));
}
