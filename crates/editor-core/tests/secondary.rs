//! The secondary as an edit (`ClipProperty::Secondary`): one undo step for
//! the pick and its shifts, held to their range, a picture's and never a
//! sound's, reset to rest by its own control.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{AudioClip, HslSecondary, SourceRange, VideoClip};
use bettercut_editor_core::{ClipProperty, Editor, EditorError};

fn with_a_shot() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Secondary");
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

fn sky_bluer() -> HslSecondary {
    HslSecondary {
        hue: 0.58,
        width: 0.1,
        hue_shift: 0.05,
        saturation: 0.4,
        luminance: -0.1,
    }
}

#[test]
fn the_secondary_is_one_undo_step() {
    let (mut editor, shot, _) = with_a_shot();
    assert!(
        editor
            .video_clip(shot)
            .unwrap()
            .color
            .secondary
            .is_identity()
    );
    let depth = editor.undo_depth();

    editor
        .set_clip_property(shot, ClipProperty::Secondary(sky_bluer()), false)
        .unwrap();
    assert_eq!(
        editor.video_clip(shot).unwrap().color.secondary,
        sky_bluer()
    );
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert!(
        editor
            .video_clip(shot)
            .unwrap()
            .color
            .secondary
            .is_identity()
    );
}

/// A drag across a slider is one step, not a hundred.
#[test]
fn a_drag_collapses_into_one_step() {
    let (mut editor, shot, _) = with_a_shot();
    let depth = editor.undo_depth();
    for step in 1..=20 {
        let mut pick = sky_bluer();
        pick.saturation = step as f32 / 20.0;
        editor
            .set_clip_property(shot, ClipProperty::Secondary(pick), step > 1)
            .unwrap();
    }
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(
        editor.video_clip(shot).unwrap().color.secondary.saturation,
        1.0
    );
}

/// Held to its range as an edit: the hue wraps, the reach is capped, nonsense
/// is rest.
#[test]
fn a_secondary_is_held_to_its_range_as_an_edit() {
    let (mut editor, shot, _) = with_a_shot();
    let wild = HslSecondary {
        hue: -0.25,
        width: 3.0,
        hue_shift: 7.0,
        saturation: f32::NAN,
        luminance: -2.0,
    };
    editor
        .set_clip_property(shot, ClipProperty::Secondary(wild), false)
        .unwrap();
    let held = editor.video_clip(shot).unwrap().color.secondary;
    assert!((held.hue - 0.75).abs() < 1e-6, "{}", held.hue);
    assert_eq!(held.width, HslSecondary::MAX_WIDTH);
    assert_eq!(held.hue_shift, 1.0);
    assert_eq!(held.saturation, 0.0);
    assert_eq!(held.luminance, -1.0);
}

#[test]
fn a_sound_clip_has_no_secondary() {
    let (mut editor, _, sound) = with_a_shot();
    assert!(matches!(
        editor.set_clip_property(sound, ClipProperty::Secondary(sky_bluer()), false),
        Err(EditorError::ClipKindMismatch)
    ));
}

/// A pick with no shift reads as untouched, so the control's reset is only
/// offered once something moves — and puts it back to rest when it does.
#[test]
fn a_secondary_reads_as_untouched_at_rest_and_resets_to_rest() {
    let mut picked = HslSecondary::IDENTITY;
    picked.hue = 0.4;
    assert!(ClipProperty::Secondary(picked).is_default());
    assert!(!ClipProperty::Secondary(sky_bluer()).is_default());

    let (mut editor, shot, _) = with_a_shot();
    editor
        .set_clip_property(shot, ClipProperty::Secondary(sky_bluer()), false)
        .unwrap();
    editor
        .reset_clip_parameter(shot, ClipProperty::Secondary(sky_bluer()))
        .unwrap();
    assert!(
        editor
            .video_clip(shot)
            .unwrap()
            .color
            .secondary
            .is_identity()
    );
    editor.undo().unwrap();
    assert_eq!(
        editor.video_clip(shot).unwrap().color.secondary,
        sky_bluer()
    );
}
