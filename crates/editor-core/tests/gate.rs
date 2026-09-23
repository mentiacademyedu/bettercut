//! The noise gate as an edit (`ClipProperty::Gate`): a sound clip's and
//! nobody else's, held to 0–100, one step for a drag, and off by default.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};
use bettercut_editor_core::{ClipProperty, Editor, EditorError};

fn editor() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Gate");
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(3)).unwrap();
    let sound = AudioClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
    let shot = VideoClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
    let (sound_id, shot_id) = (sound.id, shot.id);
    let (lane, picture) = {
        let sequence = editor.active_sequence().unwrap();
        (sequence.audio_tracks[0].id, sequence.video_tracks[0].id)
    };
    editor
        .add_clip(lane, ClipPayload::Audio(Box::new(sound)))
        .unwrap();
    editor
        .add_clip(picture, ClipPayload::Video(Box::new(shot)))
        .unwrap();
    (editor, sound_id, shot_id)
}

#[test]
fn a_clip_starts_ungated_and_is_gated_by_its_property() {
    let (mut editor, sound, _) = editor();
    assert_eq!(editor.audio_clip(sound).unwrap().gate, 0.0);
    assert!(ClipProperty::Gate(0.0).is_default());
    assert!(!ClipProperty::Gate(40.0).is_default());

    editor
        .set_clip_property(sound, ClipProperty::Gate(40.0), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().gate, 40.0);
    editor.undo().unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().gate, 0.0);
}

#[test]
fn the_gate_is_held_to_its_range() {
    let (mut editor, sound, _) = editor();
    editor
        .set_clip_property(sound, ClipProperty::Gate(250.0), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().gate, 100.0);
    editor
        .set_clip_property(sound, ClipProperty::Gate(f32::NAN), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().gate, 0.0);
}

#[test]
fn a_drag_is_one_step() {
    let (mut editor, sound, _) = editor();
    let depth = editor.undo_depth();
    for step in 1..=10 {
        editor
            .set_clip_property(sound, ClipProperty::Gate(step as f32 * 10.0), step > 1)
            .unwrap();
    }
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(editor.audio_clip(sound).unwrap().gate, 100.0);
}

#[test]
fn a_picture_has_no_gate() {
    let (mut editor, _, shot) = editor();
    assert!(matches!(
        editor.set_clip_property(shot, ClipProperty::Gate(50.0), false),
        Err(EditorError::ClipKindMismatch)
    ));
}
