//! A sound clip's pan: static, or a line of keys read at the playhead
//! (`AnimatedParameter::Pan`, the volume envelope's twin).
//!
//! The rules that have to hold: pan is a sound clip's and nobody else's; the
//! static value and the line are two ways of asking one question, and the line
//! wins while it exists; and every way of writing it is one undo step.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{AnimatedParameter, AudioClip, SourceRange, VideoClip};
use bettercut_editor_core::{ClipProperty, Editor, EditorError};
use bettercut_project_format::Project;

/// An editor with a ten-second sound clip and a ten-second shot, both from
/// timeline zero.
fn editor() -> (Editor, ClipId, ClipId) {
    let mut editor = Editor::from_project(Project::new("Pan")).0;
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap();
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
fn a_clip_starts_centred_and_is_panned_by_its_property() {
    let (mut editor, sound, _) = editor();
    assert_eq!(editor.audio_clip(sound).unwrap().pan, 0.0);

    editor
        .set_clip_property(sound, ClipProperty::Pan(-0.6), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().pan, -0.6);
    assert_eq!(
        editor
            .audio_clip(sound)
            .unwrap()
            .pan_at(TimelineTime::from_seconds(3)),
        -0.6,
        "with no line, the static pan is the pan everywhere"
    );

    editor.undo().unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().pan, 0.0);
}

/// Past hard left or hard right is not more of a side; nonsense is the middle.
#[test]
fn a_pan_is_held_to_its_range() {
    let (mut editor, sound, _) = editor();
    editor
        .set_clip_property(sound, ClipProperty::Pan(7.0), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().pan, 1.0);
    editor
        .set_clip_property(sound, ClipProperty::Pan(f32::NAN), false)
        .unwrap();
    assert_eq!(editor.audio_clip(sound).unwrap().pan, 0.0);
}

/// A picture has no side to sit on: pan on a shot is refused, and so is a
/// pan line.
#[test]
fn a_picture_cannot_be_panned() {
    let (mut editor, _, shot) = editor();
    assert!(matches!(
        editor.set_clip_property(shot, ClipProperty::Pan(0.5), false),
        Err(EditorError::ClipKindMismatch)
    ));
    assert!(matches!(
        editor.set_pan_envelope(shot, &[(TimelineTime::ZERO, 0.5)], false),
        Err(EditorError::ClipKindMismatch)
    ));
}

/// A line of keys is read at the playhead, between its points, and replaces
/// the static value while it exists.
#[test]
fn a_pan_line_crosses_between_its_points() {
    let (mut editor, sound, _) = editor();
    editor
        .set_clip_property(sound, ClipProperty::Pan(0.9), false)
        .unwrap();
    let written = editor
        .set_pan_envelope(
            sound,
            &[
                (TimelineTime::ZERO, -1.0),
                (TimelineTime::from_seconds(4), 1.0),
            ],
            false,
        )
        .unwrap();
    assert_eq!(written, 2);

    let clip = editor.audio_clip(sound).unwrap();
    assert!(clip.keyframes.is_animated(AnimatedParameter::Pan));
    assert_eq!(clip.pan_at(TimelineTime::ZERO), -1.0);
    assert!(
        clip.pan_at(TimelineTime::from_seconds(2)).abs() < 1e-5,
        "half way should be the middle: {}",
        clip.pan_at(TimelineTime::from_seconds(2))
    );
    assert_eq!(clip.pan_at(TimelineTime::from_seconds(4)), 1.0);
    assert_eq!(clip.pan, 0.9, "the static pan is kept underneath the line");

    // Cleared, the static value takes over again — and that is an undo step
    // of its own.
    editor.set_pan_envelope(sound, &[], false).unwrap();
    assert_eq!(
        editor.audio_clip(sound).unwrap().pan_at(TimelineTime::ZERO),
        0.9
    );
    editor.undo().unwrap();
    assert_eq!(
        editor.audio_clip(sound).unwrap().pan_at(TimelineTime::ZERO),
        -1.0
    );
}

/// Clearing a line that is not there is not an edit, so it leaves nothing to
/// undo.
#[test]
fn clearing_no_line_is_nothing() {
    let (mut editor, sound, _) = editor();
    assert_eq!(editor.set_pan_envelope(sound, &[], false).unwrap(), 0);
    assert!(editor.undo().is_err() || editor.audio_clip(sound).unwrap().pan == 0.0);
}

/// A key at the playhead is how a line is begun from the Inspector: the first
/// key starts the line, the next grows it, one in the same place replaces.
#[test]
fn keying_at_the_playhead_grows_the_line() {
    let (mut editor, sound, _) = editor();
    editor.set_playhead(TimelineTime::from_seconds(1));
    editor.key_pan_at_playhead(sound, -1.0, false).unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    editor.key_pan_at_playhead(sound, 1.0, false).unwrap();
    editor.key_pan_at_playhead(sound, 0.5, false).unwrap();

    let clip = editor.audio_clip(sound).unwrap();
    assert!(clip.keyframes.is_animated(AnimatedParameter::Pan));
    assert_eq!(clip.pan_at(TimelineTime::from_seconds(1)), -1.0);
    assert_eq!(clip.pan_at(TimelineTime::from_seconds(3)), 0.5);
    assert!(
        (clip.pan_at(TimelineTime::from_seconds(2)) - (-0.25)).abs() < 1e-5,
        "{}",
        clip.pan_at(TimelineTime::from_seconds(2))
    );
}

/// Off the clip there is no frame to key.
#[test]
fn keying_off_the_clip_is_refused() {
    let (mut editor, sound, _) = editor();
    editor.set_playhead(TimelineTime::from_seconds(30));
    assert!(matches!(
        editor.key_pan_at_playhead(sound, 0.3, false),
        Err(EditorError::PlayheadOffClip)
    ));
}
