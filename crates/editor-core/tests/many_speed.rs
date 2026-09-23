//! Several clips re-timed at once (`Editor::set_clips_speed`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, Rational};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn every_shot_and_its_sound_takes_the_speed_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Montage");
    let mut placed = Vec::new();
    for name in ["a", "b"] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(4),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        placed.extend(editor.place_media(media).unwrap());
    }
    let depth = editor.undo_depth();
    let double = Rational::new(2, 1).unwrap();
    // Pictures and sounds both handed over: still two shots.
    assert_eq!(editor.set_clips_speed(&placed, double).unwrap(), 2);
    for clip in &placed {
        let speed = editor
            .video_clip(*clip)
            .map(|c| c.speed)
            .or_else(|| editor.audio_clip(*clip).map(|c| c.speed))
            .unwrap();
        assert_eq!(speed, double, "a clip was left at its old speed");
    }
    assert_eq!(editor.undo_depth(), depth + 1, "one step");
}

#[test]
fn every_shot_reverses_with_its_sound_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Backwards");
    let mut placed = Vec::new();
    for name in ["a", "b"] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(4),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        placed.extend(editor.place_media(media).unwrap());
    }
    let depth = editor.undo_depth();
    assert_eq!(editor.set_clips_reversed(&placed, true).unwrap(), 2);
    assert!(placed.iter().all(|c| editor.is_reversed(*c)));
    assert_eq!(editor.undo_depth(), depth + 1, "one step");
    assert_eq!(
        editor.set_clips_reversed(&placed, true).unwrap(),
        0,
        "already so"
    );
}
