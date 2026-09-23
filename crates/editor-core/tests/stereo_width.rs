//! Stereo width as an edit (`ClipProperty::StereoWidth`): a sound clip's and
//! nobody else's, held to the audio crate's range, one step for a drag, reset
//! to as recorded.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};
use bettercut_editor_core::{ClipProperty, Editor, EditorError};

fn editor() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Width");
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
fn a_clip_starts_as_recorded_and_is_widened_by_its_property() {
    let (mut editor, sound, _) = editor();
    assert_eq!(editor.audio_clip(sound).unwrap().stereo_width, 1.0);
    assert!(ClipProperty::StereoWidth(1.0).is_default());
    assert!(!ClipProperty::StereoWidth(1.5).is_default());

    editor
        .set_clip_property(sound, ClipProperty::StereoWidth(1.5), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().stereo_width, 1.5);
    editor.undo().unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().stereo_width, 1.0);
}

/// The editor's limit is the audio crate's limit: the two must never drift,
/// or the slider would promise a width the mixer refuses.
#[test]
fn the_width_is_held_to_the_mixers_range() {
    let (mut editor, sound, _) = editor();
    editor
        .set_clip_property(sound, ClipProperty::StereoWidth(9.0), false)
        .unwrap();
    // 2.0 is `bettercut_audio::MAX_STEREO_WIDTH`, which this crate cannot
    // see; the number is pinned here so a change there breaks this test.
    assert_eq!(editor.audio_clip(sound).unwrap().stereo_width, 2.0);
    editor
        .set_clip_property(sound, ClipProperty::StereoWidth(f32::NAN), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().stereo_width, 1.0);
    editor
        .set_clip_property(sound, ClipProperty::StereoWidth(-1.0), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().stereo_width, 0.0);
}

#[test]
fn a_drag_is_one_step() {
    let (mut editor, sound, _) = editor();
    let depth = editor.undo_depth();
    for step in 1..=10 {
        editor
            .set_clip_property(
                sound,
                ClipProperty::StereoWidth(1.0 + step as f32 / 10.0),
                step > 1,
            )
            .unwrap();
    }
    assert_eq!(editor.undo_depth(), depth + 1);
}

#[test]
fn a_picture_has_no_width() {
    let (mut editor, _, shot) = editor();
    assert!(matches!(
        editor.set_clip_property(shot, ClipProperty::StereoWidth(0.5), false),
        Err(EditorError::ClipKindMismatch)
    ));
}

/// As recorded is the rest position: what the control goes back to.
#[test]
fn as_recorded_is_the_rest_position() {
    let (mut editor, sound, _) = editor();
    editor
        .set_clip_property(sound, ClipProperty::StereoWidth(0.2), false)
        .unwrap();
    editor
        .set_clip_property(sound, ClipProperty::StereoWidth(1.0), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().stereo_width, 1.0);
    assert!(ClipProperty::StereoWidth(1.0).is_default());
}
