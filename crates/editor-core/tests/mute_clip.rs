//! Muting one clip (`Editor::set_muted`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

/// Muting a shot from its picture silences its own sound, in one step, and
/// nothing moves; unmuting brings it back.
#[test]
fn a_shot_is_muted_through_its_picture() {
    let (mut editor, _events) = Editor::new_project("Mute");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/talk.mp4",
        MediaTime::from_seconds(5),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    let sequence = editor.active_sequence().unwrap().clone();
    let picture = sequence.video_tracks[0].clips()[0].id;
    let sound = sequence.audio_tracks[0].clips()[0].clone();
    let depth = editor.undo_depth();

    assert!(!editor.is_muted(picture));
    assert_eq!(editor.set_muted(picture, true).unwrap(), 1);
    assert_eq!(editor.undo_depth(), depth + 1);
    assert!(editor.is_muted(picture));
    let now = editor.audio_clip(sound.id).unwrap();
    assert!(now.muted);
    assert_eq!(now.timeline, sound.timeline, "muting moved the sound");
    assert_eq!(
        editor.set_muted(picture, true).unwrap(),
        0,
        "muting again was a step"
    );

    editor.set_muted(sound.id, false).unwrap();
    assert!(!editor.is_muted(picture));
}

/// A silent picture has no sound to mute.
#[test]
fn a_silent_picture_is_refused() {
    let (mut editor, _events) = Editor::new_project("Mute");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/b-roll.mp4",
        MediaTime::from_seconds(5),
    ));
    let picture = editor.place_media(media).unwrap()[0];
    assert!(matches!(
        editor.set_muted(picture, true),
        Err(EditorError::ClipKindMismatch)
    ));
}
