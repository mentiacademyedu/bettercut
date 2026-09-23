//! Lens correction as an edit (`ClipProperty::Lens`): a picture's and never a
//! sound's, held to -1..1, one step for a drag, reset to none, and part of
//! the clip's look so it survives a save.

#![allow(clippy::unwrap_used, clippy::expect_used)]

/// Tilt-shift: band and centre held to 0–1, carried in the look, off by
/// default, refused on sound.
#[test]
fn a_tilt_shift_is_clamped_and_carried() {
    let (mut editor, shot, sound) = editor();
    assert!(
        ClipProperty::TiltShift {
            band: 0.0,
            centre: 0.5
        }
        .is_default()
    );
    assert!(
        !ClipProperty::TiltShift {
            band: 0.3,
            centre: 0.5
        }
        .is_default()
    );
    editor
        .set_clip_property(
            shot,
            ClipProperty::TiltShift {
                band: 1.7,
                centre: -2.0,
            },
            false,
        )
        .unwrap();
    let clip = editor.video_clip(shot).unwrap();
    assert_eq!((clip.tilt_band, clip.tilt_centre), (1.0, 0.0));
    let look = clip.look_at(MediaTime::ZERO);
    assert_eq!((look.tilt_band, look.tilt_centre), (1.0, 0.0));
    editor
        .reset_clip_parameter(
            shot,
            ClipProperty::TiltShift {
                band: 1.0,
                centre: 0.0,
            },
        )
        .unwrap();
    let clip = editor.video_clip(shot).unwrap();
    assert_eq!((clip.tilt_band, clip.tilt_centre), (0.0, 0.5));
    assert!(
        editor
            .set_clip_property(
                sound,
                ClipProperty::TiltShift {
                    band: 0.3,
                    centre: 0.5
                },
                false
            )
            .is_err()
    );
}

/// The luma key is set, clamped, carried in the look, and refused on sound.
#[test]
fn a_luma_key_is_clamped_and_carried() {
    use bettercut_editor_core::timeline::LumaKey;
    let (mut editor, shot, sound) = editor();
    assert!(ClipProperty::LumaKey(None).is_default());
    assert!(!ClipProperty::LumaKey(Some(LumaKey::default())).is_default());
    editor
        .set_clip_property(
            shot,
            ClipProperty::LumaKey(Some(LumaKey {
                threshold: 1.5,
                softness: -1.0,
                keep_bright: false,
            })),
            false,
        )
        .unwrap();
    let key = editor.video_clip(shot).unwrap().luma_key.unwrap();
    assert_eq!(
        (key.threshold, key.softness, key.keep_bright),
        (1.0, 0.0, false)
    );
    assert_eq!(
        editor
            .video_clip(shot)
            .unwrap()
            .look_at(MediaTime::ZERO)
            .luma_key,
        Some(key)
    );
    editor
        .reset_clip_parameter(shot, ClipProperty::LumaKey(Some(key)))
        .unwrap();
    assert!(editor.video_clip(shot).unwrap().luma_key.is_none());
    assert!(
        editor
            .set_clip_property(
                sound,
                ClipProperty::LumaKey(Some(LumaKey::default())),
                false
            )
            .is_err()
    );
}

/// Posterise rides beside the lens: whole levels between 2 and 16, off
/// below 2, in the look the renderer reads, reset with the rest of the look.
#[test]
fn posterise_is_whole_levels_or_off() {
    let (mut editor, shot, sound) = editor();
    assert!(ClipProperty::Posterise(0.0).is_default());
    assert!(!ClipProperty::Posterise(4.0).is_default());
    editor
        .set_clip_property(shot, ClipProperty::Posterise(4.6), false)
        .unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().posterise, 4.0);
    editor
        .set_clip_property(shot, ClipProperty::Posterise(40.0), false)
        .unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().posterise, 16.0);
    editor
        .set_clip_property(shot, ClipProperty::Posterise(1.0), false)
        .unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().posterise, 0.0);
    editor
        .set_clip_property(shot, ClipProperty::Posterise(3.0), false)
        .unwrap();
    assert_eq!(
        editor
            .video_clip(shot)
            .unwrap()
            .look_at(MediaTime::ZERO)
            .posterise,
        3.0
    );
    // The control's own reset clears it (the whole-look reset leaves the
    // effects alone, as it does the lens).
    editor
        .reset_clip_parameter(shot, ClipProperty::Posterise(3.0))
        .unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().posterise, 0.0);
    assert!(
        editor
            .set_clip_property(sound, ClipProperty::Posterise(3.0), false)
            .is_err()
    );
}

use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};
use bettercut_editor_core::{ClipProperty, Editor, EditorError};

fn editor() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Lens");
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

#[test]
fn a_shot_starts_with_its_lens_as_it_was() {
    let (mut editor, shot, _) = editor();
    assert_eq!(editor.video_clip(shot).unwrap().lens, 0.0);
    assert!(ClipProperty::Lens(0.0).is_default());
    assert!(!ClipProperty::Lens(0.2).is_default());

    editor
        .set_clip_property(shot, ClipProperty::Lens(0.35), false)
        .unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().lens, 0.35);
    // The look the renderer reads carries it.
    assert_eq!(
        editor
            .video_clip(shot)
            .unwrap()
            .look_at(MediaTime::ZERO)
            .lens,
        0.35
    );
    editor.undo().unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().lens, 0.0);
}

#[test]
fn the_lens_is_held_to_its_range() {
    let (mut editor, shot, _) = editor();
    editor
        .set_clip_property(shot, ClipProperty::Lens(5.0), false)
        .unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().lens, 1.0);
    editor
        .set_clip_property(shot, ClipProperty::Lens(f32::NAN), false)
        .unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().lens, 0.0);
}

#[test]
fn a_drag_is_one_step() {
    let (mut editor, shot, _) = editor();
    let depth = editor.undo_depth();
    for step in 1..=10 {
        editor
            .set_clip_property(shot, ClipProperty::Lens(step as f32 / 10.0), step > 1)
            .unwrap();
    }
    assert_eq!(editor.undo_depth(), depth + 1);
}

#[test]
fn sound_has_no_lens() {
    let (mut editor, _, sound) = editor();
    assert!(matches!(
        editor.set_clip_property(sound, ClipProperty::Lens(0.5), false),
        Err(EditorError::ClipKindMismatch)
    ));
}

#[test]
fn reset_puts_the_lens_back() {
    let (mut editor, shot, _) = editor();
    editor
        .set_clip_property(shot, ClipProperty::Lens(-0.4), false)
        .unwrap();
    editor
        .reset_clip_parameter(shot, ClipProperty::Lens(-0.4))
        .unwrap();
    assert_eq!(editor.video_clip(shot).unwrap().lens, 0.0);
}
