//! Holding a sound clip's pitch through a speed change.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, Rational};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{ClipProperty, Editor, EditorError};

/// A shot with sound: the picture and its linked audio.
fn editor_with_sound() -> (
    Editor,
    bettercut_editor_core::foundation::ClipId,
    bettercut_editor_core::foundation::ClipId,
) {
    let (mut editor, _events) = Editor::new_project("Pitch");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/talk.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let picture = editor.place_media(media).unwrap()[0];
    let sound = editor
        .linked_with(picture)
        .into_iter()
        .find(|c| *c != picture)
        .unwrap();
    (editor, picture, sound)
}

#[test]
fn a_sound_clip_holds_its_pitch_when_asked() {
    let (mut editor, _picture, sound) = editor_with_sound();
    assert!(!editor.audio_clip(sound).unwrap().keep_pitch);
    let depth = editor.undo_depth();

    editor
        .set_clip_property(sound, ClipProperty::KeepPitch(true), false)
        .unwrap();
    assert!(editor.audio_clip(sound).unwrap().keep_pitch);
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert!(!editor.audio_clip(sound).unwrap().keep_pitch);
}

/// It survives a speed change — the thing it exists for.
#[test]
fn holding_the_pitch_outlasts_a_re_time() {
    let (mut editor, picture, sound) = editor_with_sound();
    editor
        .set_clip_property(sound, ClipProperty::KeepPitch(true), false)
        .unwrap();
    editor
        .set_clip_speed(picture, Rational::new(2, 1).unwrap(), false)
        .unwrap();
    let audio = editor.audio_clip(sound).unwrap();
    assert!(audio.keep_pitch);
    assert_eq!(audio.speed, Rational::new(2, 1).unwrap());
}

/// A picture has no pitch to hold, and neither has the whole video.
#[test]
fn only_sound_has_a_pitch_to_hold() {
    let (mut editor, picture, _sound) = editor_with_sound();
    assert!(matches!(
        editor.set_clip_property(picture, ClipProperty::KeepPitch(true), false),
        Err(EditorError::ClipKindMismatch)
    ));
    assert!(matches!(
        editor.set_sequence_value(ClipProperty::KeepPitch(true), false),
        Err(EditorError::ClipKindMismatch)
    ));
}

/// Off is the default, and "off" is what an untouched clip reports.
#[test]
fn holding_the_pitch_is_not_the_default() {
    assert!(ClipProperty::KeepPitch(false).is_default());
    assert!(!ClipProperty::KeepPitch(true).is_default());
}
