//! Clip properties: opacity, transform, volume (§59, §20a.4).
//!
//! The interesting part is not setting a float — it is that dragging a slider
//! produces a value per frame, and §11 says history holds user intentions
//! rather than mouse samples. One drag has to be one undo step, and undoing it
//! has to restore the value from *before* the drag, not from one frame earlier.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, ClipProperty, Editor};

fn editor_with_clips() -> (Editor, ClipId, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Properties");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();

    let video = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let video_id = video.id;
    let video_track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(video_track, ClipPayload::Video(Box::new(video)))
        .unwrap();

    let audio = AudioClip::new(media, TimelineTime::ZERO, source).unwrap();
    let audio_id = audio.id;
    let audio_track = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .add_clip(audio_track, ClipPayload::Audio(Box::new(audio)))
        .unwrap();

    (editor, video_id, audio_id)
}

fn opacity_of(editor: &Editor, clip: ClipId) -> f32 {
    editor.active_sequence().unwrap().video_tracks[0]
        .get(clip)
        .unwrap()
        .opacity
}

fn gain_of(editor: &Editor, clip: ClipId) -> f32 {
    editor.active_sequence().unwrap().audio_tracks[0]
        .get(clip)
        .unwrap()
        .gain
}

fn transform_of(editor: &Editor, clip: ClipId) -> bettercut_editor_core::timeline::Transform {
    editor.active_sequence().unwrap().video_tracks[0]
        .get(clip)
        .unwrap()
        .transform
}

#[test]
fn setting_opacity_changes_it_and_undoes() {
    let (mut editor, video, _) = editor_with_clips();
    assert_eq!(opacity_of(&editor, video), 1.0);

    editor
        .set_clip_property(video, ClipProperty::Opacity(0.4), false)
        .unwrap();
    assert!((opacity_of(&editor, video) - 0.4).abs() < 1e-6);

    editor.undo().unwrap();
    assert_eq!(opacity_of(&editor, video), 1.0);
}

#[test]
fn transform_position_scale_and_rotation_all_apply() {
    let (mut editor, video, _) = editor_with_clips();

    editor
        .set_clip_property(video, ClipProperty::Position { x: 0.25, y: -0.5 }, false)
        .unwrap();
    editor
        .set_clip_property(video, ClipProperty::Scale { x: 0.5, y: 0.5 }, false)
        .unwrap();
    editor
        .set_clip_property(video, ClipProperty::Rotation(90.0), false)
        .unwrap();

    let t = transform_of(&editor, video);
    assert!((t.position.x - 0.25).abs() < 1e-6);
    assert!((t.position.y + 0.5).abs() < 1e-6);
    assert!((t.scale.x - 0.5).abs() < 1e-6);
    assert!((t.rotation_degrees - 90.0).abs() < 1e-6);
    assert!(!t.is_identity());
}

/// The point of the coalescing path: sixty frames of dragging is one undo step,
/// and undo goes back to where the drag started.
#[test]
fn a_drag_collapses_into_one_undo_step() {
    let (mut editor, video, _) = editor_with_clips();
    let before_label = editor.undo_label();

    // First change of the gesture, then the rest of the drag.
    editor
        .set_clip_property(video, ClipProperty::Opacity(0.9), false)
        .unwrap();
    for step in 1..=20 {
        let value = 0.9 - (step as f32 * 0.04);
        editor
            .set_clip_property(video, ClipProperty::Opacity(value), true)
            .unwrap();
    }
    assert!((opacity_of(&editor, video) - 0.1).abs() < 1e-5);

    editor.undo().unwrap();
    assert_eq!(
        opacity_of(&editor, video),
        1.0,
        "undo went back one frame of the drag instead of before it"
    );
    assert_eq!(
        editor.undo_label(),
        before_label,
        "the drag left more than one entry in the history"
    );
}

/// Two separate gestures stay two steps — coalescing must not swallow a change
/// the user made deliberately later.
#[test]
fn separate_gestures_stay_separate() {
    let (mut editor, video, _) = editor_with_clips();

    editor
        .set_clip_property(video, ClipProperty::Opacity(0.5), false)
        .unwrap();
    editor
        .set_clip_property(video, ClipProperty::Opacity(0.2), false)
        .unwrap();

    editor.undo().unwrap();
    assert!((opacity_of(&editor, video) - 0.5).abs() < 1e-6);
    editor.undo().unwrap();
    assert_eq!(opacity_of(&editor, video), 1.0);
}

/// Dragging opacity then dragging scale must not merge: different properties
/// are different intentions even back to back.
#[test]
fn a_different_property_does_not_coalesce() {
    let (mut editor, video, _) = editor_with_clips();

    editor
        .set_clip_property(video, ClipProperty::Opacity(0.5), false)
        .unwrap();
    editor
        .set_clip_property(video, ClipProperty::Scale { x: 2.0, y: 2.0 }, true)
        .unwrap();

    editor.undo().unwrap();
    assert!((transform_of(&editor, video).scale.x - 1.0).abs() < 1e-6);
    assert!(
        (opacity_of(&editor, video) - 0.5).abs() < 1e-6,
        "undoing the scale also undid the opacity"
    );
}

/// Redo after a coalesced drag must land on the end of the drag.
#[test]
fn redo_restores_the_end_of_the_drag() {
    let (mut editor, video, _) = editor_with_clips();

    editor
        .set_clip_property(video, ClipProperty::Opacity(0.8), false)
        .unwrap();
    editor
        .set_clip_property(video, ClipProperty::Opacity(0.3), true)
        .unwrap();

    editor.undo().unwrap();
    assert_eq!(opacity_of(&editor, video), 1.0);

    editor.redo().unwrap();
    assert!(
        (opacity_of(&editor, video) - 0.3).abs() < 1e-6,
        "redo did not restore the value the drag ended on"
    );
}

// ---- colour --------------------------------------------------------------

fn color_of(editor: &Editor, clip: ClipId) -> bettercut_editor_core::timeline::ColorAdjust {
    editor.active_sequence().unwrap().video_tracks[0]
        .get(clip)
        .unwrap()
        .color
}

/// §45's cheap colour adjustment, and Milestone 8's "basic colour".
#[test]
fn brightness_contrast_and_saturation_apply_and_undo() {
    let (mut editor, video, _) = editor_with_clips();
    assert!(
        color_of(&editor, video).is_identity(),
        "default is identity"
    );

    editor
        .set_clip_property(video, ClipProperty::Brightness(1.4), false)
        .unwrap();
    editor
        .set_clip_property(video, ClipProperty::Contrast(1.2), false)
        .unwrap();
    editor
        .set_clip_property(video, ClipProperty::Saturation(0.0), false)
        .unwrap();

    let c = color_of(&editor, video);
    assert!((c.brightness - 1.4).abs() < 1e-6);
    assert!((c.contrast - 1.2).abs() < 1e-6);
    assert_eq!(c.saturation, 0.0, "saturation 0 is black and white");
    assert!(!c.is_identity());

    editor.undo().unwrap();
    editor.undo().unwrap();
    editor.undo().unwrap();
    assert!(
        color_of(&editor, video).is_identity(),
        "undoing every colour change did not return to the identity"
    );
}

/// A colour drag collapses like any other property drag.
#[test]
fn a_colour_drag_collapses_into_one_undo_step() {
    let (mut editor, video, _) = editor_with_clips();

    editor
        .set_clip_property(video, ClipProperty::Saturation(0.9), false)
        .unwrap();
    for step in 1..=15 {
        editor
            .set_clip_property(
                video,
                ClipProperty::Saturation(0.9 - step as f32 * 0.06),
                true,
            )
            .unwrap();
    }

    editor.undo().unwrap();
    assert!(
        color_of(&editor, video).is_identity(),
        "undo did not return to before the drag"
    );
}

/// Colour lives on video clips only.
#[test]
fn colour_is_refused_on_an_audio_clip() {
    let (mut editor, _, audio) = editor_with_clips();
    assert!(
        editor
            .set_clip_property(audio, ClipProperty::Brightness(1.5), false)
            .is_err()
    );
}

/// A saved project from before colour existed must still load, with the
/// identity rather than zeros — `serde(default)` on a struct whose Default is
/// all-1.0 is the only thing making that true.
#[test]
fn a_project_without_colour_loads_as_the_identity() {
    let (editor, video, _) = editor_with_clips();
    let json = serde_json::to_string(editor.project()).expect("serialize");

    // Strip the colour field, as an older file would not have had it.
    let mut value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    for sequence in value["sequences"].as_array_mut().expect("sequences") {
        for track in sequence["video_tracks"].as_array_mut().expect("tracks") {
            for clip in track["clips"].as_array_mut().expect("clips") {
                clip.as_object_mut().expect("object").remove("color");
            }
        }
    }

    let reloaded: bettercut_editor_core::project_format::Project =
        serde_json::from_value(value).expect("a file without colour must still load");
    let clip = reloaded.sequences[0].video_tracks[0].get(video).unwrap();
    assert!(
        clip.color.is_identity(),
        "an older project loaded with a non-identity colour: {:?}",
        clip.color
    );
}

// ---- blur ----------------------------------------------------------------

fn blur_of(editor: &Editor, clip: ClipId) -> f32 {
    editor.active_sequence().unwrap().video_tracks[0]
        .get(clip)
        .unwrap()
        .blur
}

/// §45's "blur → Medium", and the last of Milestone 8's four effects.
#[test]
fn blur_applies_and_undoes() {
    let (mut editor, video, _) = editor_with_clips();
    assert_eq!(blur_of(&editor, video), 0.0, "clips start unblurred");

    editor
        .set_clip_property(video, ClipProperty::Blur(35.0), false)
        .unwrap();
    assert!((blur_of(&editor, video) - 35.0).abs() < 1e-6);

    editor.undo().unwrap();
    assert_eq!(blur_of(&editor, video), 0.0);
    editor.redo().unwrap();
    assert!((blur_of(&editor, video) - 35.0).abs() < 1e-6);
}

/// The whole point of the 0–100 scale: it is a fraction of frame height, so
/// the same number has to survive a change of sequence resolution untouched.
/// A pixel radius would have to be rewritten here, and rewriting stored values
/// on a format change is how projects get silently altered.
#[test]
fn blur_is_unaffected_by_the_sequence_resolution() {
    use bettercut_editor_core::timeline::Resolution;

    let (mut editor, video, _) = editor_with_clips();
    editor
        .set_clip_property(video, ClipProperty::Blur(60.0), false)
        .unwrap();

    let rate = editor.active_sequence().unwrap().frame_rate;
    editor
        .set_sequence_format(Resolution::HD_720, rate)
        .unwrap();
    assert!(
        (blur_of(&editor, video) - 60.0).abs() < 1e-6,
        "changing the sequence resolution rewrote the blur amount"
    );
}

#[test]
fn a_blur_drag_collapses_into_one_undo_step() {
    let (mut editor, video, _) = editor_with_clips();

    editor
        .set_clip_property(video, ClipProperty::Blur(2.0), false)
        .unwrap();
    for step in 1..=20 {
        editor
            .set_clip_property(video, ClipProperty::Blur(2.0 + step as f32 * 3.0), true)
            .unwrap();
    }

    editor.undo().unwrap();
    assert_eq!(
        blur_of(&editor, video),
        0.0,
        "undo did not return to before the drag"
    );
}

#[test]
fn blur_is_refused_on_an_audio_clip() {
    let (mut editor, _, audio) = editor_with_clips();
    assert!(
        editor
            .set_clip_property(audio, ClipProperty::Blur(20.0), false)
            .is_err()
    );
}

/// Blur is the one property whose "does nothing" value is zero rather than
/// one, so `serde(default)` is enough on its own — but only because of that.
#[test]
fn a_project_without_blur_loads_unblurred() {
    let (editor, video, _) = editor_with_clips();
    let json = serde_json::to_string(editor.project()).expect("serialize");

    let mut value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    for sequence in value["sequences"].as_array_mut().expect("sequences") {
        for track in sequence["video_tracks"].as_array_mut().expect("tracks") {
            for clip in track["clips"].as_array_mut().expect("clips") {
                clip.as_object_mut().expect("object").remove("blur");
            }
        }
    }

    let reloaded: bettercut_editor_core::project_format::Project =
        serde_json::from_value(value).expect("a file without blur must still load");
    let clip = reloaded.sequences[0].video_tracks[0].get(video).unwrap();
    assert_eq!(clip.blur, 0.0, "an older project loaded already blurred");
}

// ---- audio ---------------------------------------------------------------

#[test]
fn volume_applies_to_an_audio_clip() {
    let (mut editor, _, audio) = editor_with_clips();
    assert!((gain_of(&editor, audio) - 1.0).abs() < 1e-6);

    editor
        .set_clip_property(audio, ClipProperty::Gain(0.25), false)
        .unwrap();
    assert!((gain_of(&editor, audio) - 0.25).abs() < 1e-6);
}

/// A video clip has no volume and an audio clip has no opacity. Applying the
/// wrong one must fail rather than silently doing nothing.
#[test]
fn properties_are_refused_on_the_wrong_kind_of_clip() {
    let (mut editor, video, audio) = editor_with_clips();

    assert!(
        editor
            .set_clip_property(video, ClipProperty::Gain(0.5), false)
            .is_err(),
        "gain was accepted on a video clip"
    );
    assert!(
        editor
            .set_clip_property(audio, ClipProperty::Opacity(0.5), false)
            .is_err(),
        "opacity was accepted on an audio clip"
    );
}

/// §50: a hand-edited project or a replayed journal can carry anything. Values
/// are clamped rather than trusted.
#[test]
fn out_of_range_values_are_clamped() {
    let (mut editor, video, audio) = editor_with_clips();

    editor
        .set_clip_property(video, ClipProperty::Opacity(40.0), false)
        .unwrap();
    assert_eq!(opacity_of(&editor, video), 1.0);

    editor
        .set_clip_property(video, ClipProperty::Opacity(-3.0), false)
        .unwrap();
    assert_eq!(opacity_of(&editor, video), 0.0);

    // Zero scale renders nothing and cannot be dragged back out of.
    editor
        .set_clip_property(video, ClipProperty::Scale { x: 0.0, y: 0.0 }, false)
        .unwrap();
    assert!(transform_of(&editor, video).scale.x > 0.0);

    editor
        .set_clip_property(audio, ClipProperty::Gain(99.0), false)
        .unwrap();
    assert!(gain_of(&editor, audio) <= 4.0);

    // A blur far past the top of the slider would spend the whole tap budget
    // on a kernel wider than the frame, for no visible gain.
    editor
        .set_clip_property(video, ClipProperty::Blur(5000.0), false)
        .unwrap();
    assert_eq!(
        blur_of(&editor, video),
        bettercut_editor_core::timeline::MAX_BLUR
    );

    editor
        .set_clip_property(video, ClipProperty::Blur(-10.0), false)
        .unwrap();
    assert_eq!(blur_of(&editor, video), 0.0);
}

/// §38.2: the journal replays these after a crash.
#[test]
fn the_command_round_trips_through_json() {
    for property in [
        ClipProperty::Opacity(0.5),
        ClipProperty::Gain(1.5),
        ClipProperty::Position { x: 0.1, y: -0.2 },
        ClipProperty::Scale { x: 2.0, y: 2.0 },
        ClipProperty::Rotation(45.0),
        ClipProperty::Brightness(1.2),
        ClipProperty::Contrast(0.8),
        ClipProperty::Saturation(0.0),
        ClipProperty::Blur(40.0),
    ] {
        let command = bettercut_editor_core::Command::SetClipProperty {
            sequence: bettercut_editor_core::foundation::SequenceId::new(),
            track: bettercut_editor_core::foundation::TrackId::new(),
            clip: ClipId::new(),
            property,
        };
        let json = serde_json::to_string(&command).expect("serialize");
        assert_eq!(
            serde_json::from_str::<bettercut_editor_core::Command>(&json).expect("deserialize"),
            command,
            "round trip failed for {property:?}"
        );
    }
}

/// A look is several values that only mean something together, so it goes on in
/// one step and comes off in one.
#[test]
fn a_look_applies_to_a_clip_as_one_undo_step() {
    use bettercut_editor_core::timeline::ColorAdjust;

    let (mut editor, clip, _) = editor_with_clips();
    let depth = editor.undo_depth();
    let look = ColorAdjust {
        brightness: 1.1,
        contrast: 0.8,
        saturation: 0.7,
        temperature: 0.4,
        tint: -0.15,
        vibrance: 0.0,
    };

    editor.set_color_adjust(Some(clip), look).unwrap();

    let color = editor.video_clip(clip).unwrap().color;
    assert_eq!(color, look);
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(
        editor.video_clip(clip).unwrap().color,
        ColorAdjust::default()
    );
}

#[test]
fn a_look_applies_to_the_whole_video_when_no_clip_is_given() {
    use bettercut_editor_core::timeline::ColorAdjust;

    let (mut editor, _, _) = editor_with_clips();
    let look = ColorAdjust {
        brightness: 0.9,
        contrast: 1.2,
        saturation: 0.0,
        temperature: -0.3,
        tint: 0.2,
        vibrance: 0.0,
    };

    editor.set_color_adjust(None, look).unwrap();

    assert_eq!(editor.active_sequence().unwrap().master.color, look);
    editor.undo().unwrap();
    assert_eq!(
        editor.active_sequence().unwrap().master.color,
        ColorAdjust::default()
    );
}

/// The chroma key, and the one thing about it the editor is responsible for:
/// the shader trusts what it is given, so nothing out of range may reach it.
mod chroma_key {
    use bettercut_editor_core::foundation::MediaTime;
    use bettercut_editor_core::media::{MediaAsset, MediaKind};
    use bettercut_editor_core::timeline::ChromaKey;
    use bettercut_editor_core::{ClipProperty, Editor, EditorError};

    fn editor_with_a_shot() -> (Editor, bettercut_editor_core::foundation::ClipId) {
        let (mut editor, _events) = Editor::new_project("Key");
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            "C:/media/screen.mp4",
            MediaTime::from_seconds(10),
        ));
        let placed = editor.place_media(media).unwrap();
        (editor, placed[0])
    }

    #[test]
    fn a_key_out_of_range_is_brought_back_in() {
        let (mut editor, clip) = editor_with_a_shot();

        editor
            .set_clip_property(
                clip,
                ClipProperty::ChromaKey(Some(ChromaKey {
                    color: [-4.0, 12.0, f32::NAN],
                    tolerance: 900.0,
                    softness: -3.0,
                    spill: 50.0,
                })),
                false,
            )
            .unwrap();

        let key = editor.video_clip(clip).unwrap().chroma_key.expect("a key");
        assert_eq!(key.tolerance, ChromaKey::MAX_SPREAD, "tolerance unclamped");
        assert_eq!(key.softness, 0.0, "a negative softness reached the shader");
        assert_eq!(key.spill, 1.0, "spill unclamped");
        assert_eq!(key.color[0], 0.0);
        assert_eq!(key.color[1], 1.0);
        // NaN turns the effect off rather than through: `f32::clamp` passes
        // it along, and one NaN in a project file would otherwise reach the
        // shader, where every comparison against it is false.
        assert_eq!(
            key.color[2], 0.0,
            "a NaN channel reached the shader: {:?}",
            key.color
        );
    }

    /// Sound has no screen behind it.
    #[test]
    fn a_sound_clip_takes_no_key() {
        let (mut editor, _picture) = editor_with_a_shot();
        let (mut editor2, _events) = Editor::new_project("Key");
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            "C:/media/talk.mp4",
            MediaTime::from_seconds(10),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor2.import_media(asset);
        let sound = editor2.place_media(media).unwrap()[1];

        let refused = editor2.set_clip_property(
            sound,
            ClipProperty::ChromaKey(Some(ChromaKey::default())),
            false,
        );
        assert!(
            matches!(refused, Err(EditorError::ClipKindMismatch)),
            "{refused:?}"
        );
        let _ = &mut editor;
    }
}

/// The mask, and the same responsibility the chroma key has: the shader
/// trusts what the editor hands it.
mod mask {
    use bettercut_editor_core::foundation::MediaTime;
    use bettercut_editor_core::media::{MediaAsset, MediaKind};
    use bettercut_editor_core::timeline::{Mask, MaskShape};
    use bettercut_editor_core::{ClipProperty, Editor};

    #[test]
    fn a_mask_out_of_range_is_brought_back_in() {
        let (mut editor, _events) = Editor::new_project("Mask");
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            "C:/media/shot.mp4",
            MediaTime::from_seconds(10),
        ));
        let clip = editor.place_media(media).unwrap()[0];

        editor
            .set_clip_property(
                clip,
                ClipProperty::Mask(Some(Mask {
                    shape: MaskShape::Ellipse,
                    center: [f32::NAN, 900.0],
                    size: [-5.0, 1e9],
                    feather: f32::NAN,
                    rotation_degrees: f32::INFINITY,
                    invert: false,
                })),
                false,
            )
            .unwrap();

        let mask = editor.video_clip(clip).unwrap().mask.expect("a mask");
        assert!(
            mask.center.iter().all(|v| v.is_finite()),
            "a NaN centre reached the shader: {:?}",
            mask.center
        );
        assert!(
            mask.size.iter().all(|v| v.is_finite() && *v >= 0.0),
            "a negative or absurd size reached the shader: {:?}",
            mask.size
        );
        assert_eq!(mask.feather, 0.0, "a NaN feather reached the shader");
        assert!(
            mask.rotation_degrees.is_finite(),
            "an infinite angle reached the shader"
        );
    }
}

/// §22's background, and the trap it shares with volume: `is_default` falls
/// through to the animated parameters, and a property that has none reports
/// itself default whatever it holds — which greys its reset button out for
/// good.
#[test]
fn a_background_knows_when_it_is_not_the_default() {
    assert!(ClipProperty::Background([0.0, 0.0, 0.0]).is_default());
    assert!(!ClipProperty::Background([1.0, 1.0, 1.0]).is_default());
    assert!(
        !ClipProperty::Background([0.0, 0.0, 0.02]).is_default(),
        "a colour that is nearly black is still not black"
    );
}

/// A mirror is an ordinary undoable property, and the two axes are independent:
/// mirroring left-to-right must not also turn the picture over.
#[test]
fn each_mirror_goes_on_and_comes_off_on_its_own() {
    use bettercut_editor_core::timeline::FlipAxis;

    let (mut editor, video, _audio) = editor_with_clips();
    assert!(!transform_of(&editor, video).flip_h);

    editor
        .set_clip_property(
            video,
            ClipProperty::Flip {
                axis: FlipAxis::Horizontal,
                on: true,
            },
            false,
        )
        .unwrap();

    let after = transform_of(&editor, video);
    assert!(after.flip_h, "the horizontal mirror did not go on");
    assert!(
        !after.flip_v,
        "mirroring left to right also turned the picture over"
    );

    editor.undo().unwrap();
    assert!(
        !transform_of(&editor, video).flip_h,
        "undo left the clip mirrored"
    );
}

/// Mirroring one way and then the other is two undo steps, not one gesture that
/// collapses — they are different edits and the user expects to take back the
/// second without losing the first.
#[test]
fn the_two_mirrors_do_not_collapse_into_one_undo_step() {
    use bettercut_editor_core::timeline::FlipAxis;

    let (mut editor, video, _audio) = editor_with_clips();
    for axis in FlipAxis::ALL {
        editor
            .set_clip_property(video, ClipProperty::Flip { axis, on: true }, false)
            .unwrap();
    }

    let both = transform_of(&editor, video);
    assert!(both.flip_h && both.flip_v, "both mirrors should be on");

    editor.undo().unwrap();
    let one = transform_of(&editor, video);
    assert!(
        one.flip_h && !one.flip_v,
        "one undo took back both mirrors: {one:?}"
    );
}

/// A sound has no picture to mirror, and the refusal is the model's, not the
/// interface's — the journal replays commands without an interface in sight.
#[test]
fn a_sound_cannot_be_mirrored() {
    use bettercut_editor_core::timeline::FlipAxis;

    let (mut editor, _video, audio) = editor_with_clips();
    assert!(
        editor
            .set_clip_property(
                audio,
                ClipProperty::Flip {
                    axis: FlipAxis::Horizontal,
                    on: true,
                },
                false,
            )
            .is_err(),
        "a sound accepted a mirror"
    );
}

/// The whole video's vignette: set and undone — and a clip's own, which is
/// separate from it and leaves the master's alone.
///
/// The `is_default` check is the one that matters most and would pass unnoticed
/// if wrong: a vignette has no animated parameter behind it, so without its own
/// case every vignette reads as untouched and its reset stays greyed out.
#[test]
fn the_whole_video_and_a_clip_each_take_their_own_vignette() {
    assert!(ClipProperty::Vignette(0.0).is_default());
    assert!(
        !ClipProperty::Vignette(0.4).is_default(),
        "a vignette reads as untouched, so its reset would never be offered"
    );

    let (mut editor, video, _) = editor_with_clips();
    editor
        .set_sequence_value(ClipProperty::Vignette(0.5), false)
        .unwrap();
    assert_eq!(editor.active_sequence().unwrap().master.vignette, 0.5);

    editor.undo().unwrap();
    assert_eq!(editor.active_sequence().unwrap().master.vignette, 0.0);

    editor
        .set_clip_property(video, ClipProperty::Vignette(2.0), false)
        .unwrap();
    assert_eq!(
        editor.video_clip(video).unwrap().vignette,
        bettercut_editor_core::timeline::MAX_VIGNETTE,
        "a clip's vignette was not held to its range"
    );
    assert_eq!(
        editor.active_sequence().unwrap().master.vignette,
        0.0,
        "a clip's vignette reached the whole video"
    );
}

/// §50: whatever the command carries, the master holds a vignette in range.
#[test]
fn a_runaway_vignette_is_brought_into_range() {
    let (mut editor, _, _) = editor_with_clips();
    editor
        .set_sequence_value(ClipProperty::Vignette(40.0), false)
        .unwrap();
    assert_eq!(editor.active_sequence().unwrap().master.vignette, 1.0);

    editor
        .set_sequence_value(ClipProperty::Vignette(f32::NAN), false)
        .unwrap();
    assert_eq!(editor.active_sequence().unwrap().master.vignette, 0.0);
}

/// A sound clip's voice clean-up: set, undone, clamped, and refused by a
/// picture and by the whole video.
#[test]
fn a_sound_clip_takes_voice_clean_up_and_a_picture_does_not() {
    assert!(ClipProperty::Denoise(0.0).is_default());
    assert!(!ClipProperty::Denoise(40.0).is_default());

    let (mut editor, video, audio) = editor_with_clips();
    editor
        .set_clip_property(audio, ClipProperty::Denoise(500.0), false)
        .unwrap();
    assert_eq!(editor.audio_clip(audio).unwrap().denoise, 100.0);
    editor.undo().unwrap();
    assert_eq!(editor.audio_clip(audio).unwrap().denoise, 0.0);

    assert!(
        editor
            .set_clip_property(video, ClipProperty::Denoise(50.0), false)
            .is_err()
    );
    assert!(
        editor
            .set_sequence_value(ClipProperty::Denoise(50.0), false)
            .is_err()
    );
}

/// The whole video's film grain: set, undone, clamped, refused by a clip, and
/// with the `is_default` case that keeps its reset button honest.
#[test]
fn the_whole_video_takes_grain_and_a_clip_does_not() {
    assert!(ClipProperty::Grain(0.0).is_default());
    assert!(!ClipProperty::Grain(0.3).is_default());

    let (mut editor, video, _) = editor_with_clips();
    editor
        .set_sequence_value(ClipProperty::Grain(0.5), false)
        .unwrap();
    assert_eq!(editor.active_sequence().unwrap().master.grain, 0.5);
    editor.undo().unwrap();
    assert_eq!(editor.active_sequence().unwrap().master.grain, 0.0);

    for runaway in [40.0, f32::NAN] {
        editor
            .set_sequence_value(ClipProperty::Grain(runaway), false)
            .unwrap();
        let grain = editor.active_sequence().unwrap().master.grain;
        assert!((0.0..=1.0).contains(&grain), "{runaway} became {grain}");
    }

    assert!(
        editor
            .set_clip_property(video, ClipProperty::Grain(0.5), false)
            .is_err(),
        "a clip accepted grain, which is the film the whole frame is on"
    );
}

/// Cinematic bars: the whole video's, set, undone, clamped, refused by a clip.
#[test]
fn the_whole_video_takes_cinematic_bars_and_a_clip_does_not() {
    assert!(ClipProperty::Bars(0.0).is_default());
    assert!(!ClipProperty::Bars(2.39).is_default());

    let (mut editor, video, _) = editor_with_clips();
    editor
        .set_sequence_value(ClipProperty::Bars(2.39), false)
        .unwrap();
    assert_eq!(editor.active_sequence().unwrap().master.bars, 2.39);
    assert!(!editor.active_sequence().unwrap().master.is_identity());
    editor.undo().unwrap();
    assert_eq!(editor.active_sequence().unwrap().master.bars, 0.0);

    for runaway in [400.0, f32::NAN, -2.0] {
        editor
            .set_sequence_value(ClipProperty::Bars(runaway), false)
            .unwrap();
        let bars = editor.active_sequence().unwrap().master.bars;
        assert!((0.0..=4.0).contains(&bars), "{runaway} became {bars}");
    }

    assert!(
        editor
            .set_clip_property(video, ClipProperty::Bars(2.0), false)
            .is_err()
    );
}

/// A clip's sharpen: set, undone, clamped, and refused by sound and by the
/// whole video — with the `is_default` check that keeps its reset honest.
#[test]
fn a_clip_takes_a_sharpen_and_nothing_else_does() {
    assert!(ClipProperty::Sharpen(0.0).is_default());
    assert!(
        !ClipProperty::Sharpen(30.0).is_default(),
        "a sharpen reads as untouched, so its reset would never be offered"
    );

    let (mut editor, video, audio) = editor_with_clips();
    editor
        .set_clip_property(video, ClipProperty::Sharpen(40.0), false)
        .unwrap();
    assert_eq!(editor.video_clip(video).unwrap().sharpen, 40.0);
    editor.undo().unwrap();
    assert_eq!(editor.video_clip(video).unwrap().sharpen, 0.0);

    editor
        .set_clip_property(video, ClipProperty::Sharpen(9_000.0), false)
        .unwrap();
    assert_eq!(
        editor.video_clip(video).unwrap().sharpen,
        bettercut_editor_core::timeline::MAX_SHARPEN,
        "a runaway sharpen got through"
    );

    assert!(
        editor
            .set_clip_property(audio, ClipProperty::Sharpen(40.0), false)
            .is_err(),
        "a sound accepted a sharpen"
    );
    assert!(
        editor
            .set_sequence_value(ClipProperty::Sharpen(40.0), false)
            .is_err(),
        "the whole video accepted a sharpen, which is a decision about one shot"
    );
}

/// A reflection is set, undone and redone like any property, refused on sound,
/// and a project saved before reflections existed loads with none.
#[test]
fn a_reflection_applies_undoes_and_is_picture_only() {
    use bettercut_editor_core::timeline::Reflection;

    let (mut editor, video, audio) = editor_with_clips();
    let reflection_of = |editor: &Editor| editor.video_clip(video).unwrap().reflection;
    assert_eq!(reflection_of(&editor), Reflection::None);

    editor
        .set_clip_property(
            video,
            ClipProperty::Reflection(Reflection::Kaleidoscope),
            false,
        )
        .unwrap();
    assert_eq!(reflection_of(&editor), Reflection::Kaleidoscope);
    assert_eq!(editor.undo_label().as_deref(), Some("Change Mirror"));
    editor.undo().unwrap();
    assert_eq!(reflection_of(&editor), Reflection::None);
    editor.redo().unwrap();
    assert_eq!(reflection_of(&editor), Reflection::Kaleidoscope);

    assert!(
        editor
            .set_clip_property(audio, ClipProperty::Reflection(Reflection::FourWay), false)
            .is_err()
    );

    let mut json: serde_json::Value = serde_json::to_value(editor.project()).expect("serialize");
    let clip = &mut json["sequences"][0]["video_tracks"][0]["clips"][0];
    assert!(
        clip.get("reflection").is_some(),
        "the field is not where this test looks"
    );
    clip.as_object_mut().unwrap().remove("reflection");
    let project: bettercut_editor_core::project_format::Project =
        serde_json::from_value(json).expect("an old project loads");
    let old = project.sequences[0].video_tracks[0].clips()[0].reflection;
    assert_eq!(old, Reflection::None);
}

/// A sound clip's EQ is set, held to its ranges, undone, and refused on a
/// picture.
#[test]
fn an_eq_applies_to_sound_only() {
    use bettercut_editor_core::timeline::{ClipEq, EQ_LOW_CUT_MAX};

    let (mut editor, video, audio) = editor_with_clips();
    let eq = ClipEq {
        low_cut: 1_000.0,
        high_cut: 0.0,
        presence: 3.0,
        hum: 60.0,
    };
    editor
        .set_clip_property(audio, ClipProperty::Eq(eq), false)
        .unwrap();
    let stored = editor.audio_clip(audio).unwrap().eq;
    assert_eq!(
        stored.low_cut, EQ_LOW_CUT_MAX,
        "the low cut was not held to its range"
    );
    assert_eq!(stored.presence, 3.0);

    editor.undo().unwrap();
    assert!(editor.audio_clip(audio).unwrap().eq.is_flat());
    assert!(
        editor
            .set_clip_property(video, ClipProperty::Eq(eq), false)
            .is_err()
    );
}

/// A sound clip's echo or reverb is set, held to its range, undone, refused
/// on a picture, and a dry kind stores as dry whatever the amount.
#[test]
fn echo_and_reverb_apply_to_sound_only() {
    use bettercut_editor_core::timeline::{ClipSpace, SpaceKind};

    let (mut editor, video, audio) = editor_with_clips();
    let hall = ClipSpace {
        kind: SpaceKind::Hall,
        mix: 3.0,
    };
    editor
        .set_clip_property(audio, ClipProperty::Space(hall), false)
        .unwrap();
    assert_eq!(
        editor.audio_clip(audio).unwrap().space,
        ClipSpace {
            kind: SpaceKind::Hall,
            mix: 1.0
        },
        "the amount was not held to its range"
    );
    editor.undo().unwrap();
    assert!(editor.audio_clip(audio).unwrap().space.is_dry());

    editor
        .set_clip_property(
            audio,
            ClipProperty::Space(ClipSpace {
                kind: SpaceKind::Dry,
                mix: 0.8,
            }),
            false,
        )
        .unwrap();
    assert_eq!(
        editor.audio_clip(audio).unwrap().space,
        ClipSpace::default()
    );
    assert!(
        editor
            .set_clip_property(video, ClipProperty::Space(hall), false)
            .is_err()
    );
}

/// Dragging a picture's fade handle sets its entrance or exit length: a fade
/// where there was none, the same kind where there was one, and nothing once
/// dragged under half the shortest motion.
#[test]
fn a_picture_fade_handle_sets_its_entrance_and_exit() {
    use bettercut_editor_core::foundation::TimelineTime;
    use bettercut_editor_core::timeline::{ClipMotion, MIN_MOTION, Motion, MotionKind};

    let (mut editor, video, _) = editor_with_clips();
    editor
        .set_clip_ramps(
            video,
            TimelineTime::from_millis(500),
            TimelineTime::ZERO,
            false,
        )
        .unwrap();
    let motion = editor.video_clip(video).unwrap().motion;
    assert_eq!(motion.intro.map(|m| m.kind), Some(MotionKind::Fade));
    assert_eq!(
        motion.intro.unwrap().duration,
        TimelineTime::from_millis(500)
    );
    assert_eq!(motion.outro, None);

    // A slide keeps being a slide when its length is dragged.
    editor
        .set_clip_property(
            video,
            ClipProperty::Motion(ClipMotion {
                intro: None,
                outro: Some(Motion::new(
                    MotionKind::SlideLeft,
                    TimelineTime::from_seconds(1),
                )),
            }),
            false,
        )
        .unwrap();
    editor
        .set_clip_ramps(
            video,
            TimelineTime::ZERO,
            TimelineTime::from_millis(1_500),
            false,
        )
        .unwrap();
    let outro = editor.video_clip(video).unwrap().motion.outro.unwrap();
    assert_eq!(outro.kind, MotionKind::SlideLeft);
    assert_eq!(outro.duration, TimelineTime::from_millis(1_500));

    // Dragged nearly to nothing: gone.
    editor
        .set_clip_ramps(
            video,
            TimelineTime::ZERO,
            TimelineTime::from_ticks(MIN_MOTION.ticks() / 3),
            false,
        )
        .unwrap();
    assert_eq!(editor.video_clip(video).unwrap().motion.outro, None);

    // A drag is one undo step.
    let depth = editor.undo_depth();
    for ms in [200, 300, 400] {
        editor
            .set_clip_ramps(
                video,
                TimelineTime::from_millis(ms),
                TimelineTime::ZERO,
                true,
            )
            .unwrap();
    }
    assert!(editor.undo_depth() <= depth + 1);
}

/// A sound clip's fade shape: set, undone, refused by a picture.
#[test]
fn a_sound_takes_a_fade_shape_and_a_picture_does_not() {
    use bettercut_editor_core::timeline::FadeShape;
    assert!(ClipProperty::FadeShape(FadeShape::Smooth).is_default());
    assert!(!ClipProperty::FadeShape(FadeShape::Fast).is_default());

    let (mut editor, video, sound) = editor_with_clips();
    editor
        .set_clip_property(sound, ClipProperty::FadeShape(FadeShape::Slow), false)
        .unwrap();
    assert_eq!(
        editor.audio_clip(sound).unwrap().fade_shape,
        FadeShape::Slow
    );
    editor.undo().unwrap();
    assert_eq!(
        editor.audio_clip(sound).unwrap().fade_shape,
        FadeShape::Smooth
    );
    assert!(
        editor
            .set_clip_property(video, ClipProperty::FadeShape(FadeShape::Fast), false)
            .is_err()
    );
}
