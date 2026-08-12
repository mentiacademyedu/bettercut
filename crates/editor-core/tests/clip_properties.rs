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
