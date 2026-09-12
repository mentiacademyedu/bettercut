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

/// A look is three values that only mean something together, so it goes on in
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
    };

    editor.set_color_adjust(None, look).unwrap();

    assert_eq!(editor.active_sequence().unwrap().master.color, look);
    editor.undo().unwrap();
    assert_eq!(
        editor.active_sequence().unwrap().master.color,
        ColorAdjust::default()
    );
}
