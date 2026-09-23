//! The colour wheels as an edit (`ClipProperty::Wheels`): one undo step for
//! the whole set, held to their range, a picture's and never a sound's, and
//! reset to rest with the rest of the grade.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::timeline::ColorWheels;
use bettercut_editor_core::{ClipProperty, Editor, EditorError};

/// An editor with one shot on the first picture lane and a sound clip on the
/// first sound lane.
fn with_a_shot() -> (Editor, ClipId, ClipId) {
    use bettercut_editor_core::command::ClipPayload;
    use bettercut_editor_core::foundation::{MediaId, TimelineTime};
    use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};

    let (mut editor, _events) = Editor::new_project("Wheels");
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(3)).unwrap();
    let shot = VideoClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
    let sound = AudioClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
    let (shot_id, sound_id) = (shot.id, sound.id);
    let (picture, lane) = {
        let sequence = editor.active_sequence().unwrap();
        (sequence.video_tracks[0].id, sequence.audio_tracks[0].id)
    };
    editor
        .add_clip(picture, ClipPayload::Video(Box::new(shot)))
        .unwrap();
    editor
        .add_clip(lane, ClipPayload::Audio(Box::new(sound)))
        .unwrap();
    (editor, shot_id, sound_id)
}

fn turned() -> ColorWheels {
    ColorWheels {
        lift: [0.2, 0.0, -0.1],
        gamma: [0.0, 0.3, 0.0],
        gain: [-0.2, 0.0, 0.4],
    }
}

#[test]
fn the_wheels_are_one_undo_step() {
    let (mut editor, shot, _) = with_a_shot();
    assert!(editor.video_clip(shot).unwrap().color.wheels.is_identity());
    let depth = editor.undo_depth();

    editor
        .set_clip_property(shot, ClipProperty::Wheels(turned()), false)
        .unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().color.wheels, turned());
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert!(editor.video_clip(shot).unwrap().color.wheels.is_identity());
}

/// A drag on a wheel is one step, not a hundred.
#[test]
fn a_drag_on_a_wheel_collapses_into_one_step() {
    let (mut editor, shot, _) = with_a_shot();
    let depth = editor.undo_depth();
    for step in 1..=20 {
        let mut wheels = ColorWheels::IDENTITY;
        wheels.gain[0] = step as f32 / 20.0;
        editor
            .set_clip_property(shot, ClipProperty::Wheels(wheels), step > 1)
            .unwrap();
    }
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(editor.video_clip(shot).unwrap().color.wheels.gain[0], 1.0);
}

/// Past full is held to full, and nonsense is rest: the shader is never handed
/// a value it would turn into NaN.
#[test]
fn wheels_are_held_to_their_range_as_an_edit() {
    let (mut editor, shot, _) = with_a_shot();
    let wild = ColorWheels {
        lift: [4.0, -4.0, f32::NAN],
        gamma: [0.5; 3],
        gain: [f32::INFINITY, 0.0, 0.0],
    };
    editor
        .set_clip_property(shot, ClipProperty::Wheels(wild), false)
        .unwrap();
    let held = editor.video_clip(shot).unwrap().color.wheels;
    assert_eq!(held.lift, [1.0, -1.0, 0.0]);
    assert_eq!(held.gain, [0.0, 0.0, 0.0]);
    assert_eq!(held.gamma, [0.5; 3]);
}

/// Sound has no colour to grade.
#[test]
fn a_sound_clip_has_no_wheels() {
    let (mut editor, _, sound) = with_a_shot();
    assert!(matches!(
        editor.set_clip_property(sound, ClipProperty::Wheels(turned()), false),
        Err(EditorError::ClipKindMismatch)
    ));
}

/// Wheels at rest read as untouched, so the control's reset is only offered
/// when there is something to reset — and takes them back to rest when there
/// is, as one step.
#[test]
fn wheels_read_as_untouched_at_rest_and_reset_to_rest() {
    assert!(ClipProperty::Wheels(ColorWheels::IDENTITY).is_default());
    assert!(!ClipProperty::Wheels(turned()).is_default());

    let (mut editor, shot, _) = with_a_shot();
    editor
        .set_clip_property(shot, ClipProperty::Wheels(turned()), false)
        .unwrap();
    editor
        .reset_clip_parameter(shot, ClipProperty::Wheels(turned()))
        .unwrap();
    assert!(editor.video_clip(shot).unwrap().color.wheels.is_identity());
    editor.undo().unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().color.wheels, turned());
}
