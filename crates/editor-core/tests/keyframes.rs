//! Keyframes through the editor (§24, §10's `SetKeyframeCommand`).
//!
//! The model's own arithmetic is tested in the timeline crate. What matters
//! here is everything around it: that a key is undoable in both directions,
//! that the same slider writes a key or a static value depending on what the
//! parameter already is, and that a key survives the §38.2 journal — an
//! animation lost to a crash is worse than one that was never made.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{
    AnimatedParameter, Interpolation, Keyframe, SourceRange, VideoClip,
};
use bettercut_editor_core::{ClipPayload, ClipProperty, Editor};

/// A clip from 0 to 4 s on the timeline, reading the source from 1 s in — so a
/// test that confused timeline time with source time would fail rather than
/// pass by coincidence.
fn editor_with_clip() -> (Editor, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Keyframes");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let source = SourceRange::new(MediaTime::from_seconds(1), MediaTime::from_seconds(5)).unwrap();

    let clip = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let id = clip.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    (editor, id)
}

fn clip_of(editor: &Editor, id: ClipId) -> &VideoClip {
    editor.video_clip(id).unwrap()
}

fn opacity_at(editor: &Editor, id: ClipId, seconds: i64) -> f32 {
    let clip = clip_of(editor, id);
    clip.look_at(clip.source_time_at(TimelineTime::from_seconds(seconds)))
        .opacity
}

fn key(seconds: i64, value: f32) -> Keyframe {
    Keyframe::new(
        MediaTime::from_seconds(seconds),
        value,
        Interpolation::Linear,
    )
}

#[test]
fn a_keyframe_is_undoable() {
    let (mut editor, clip) = editor_with_clip();
    editor
        .set_keyframe(clip, AnimatedParameter::Opacity, key(1, 0.25))
        .unwrap();
    assert!(
        clip_of(&editor, clip)
            .keyframes
            .is_animated(AnimatedParameter::Opacity)
    );

    editor.undo().unwrap();
    assert!(
        !clip_of(&editor, clip)
            .keyframes
            .is_animated(AnimatedParameter::Opacity),
        "undo left the parameter animated"
    );

    editor.redo().unwrap();
    assert_eq!(
        clip_of(&editor, clip)
            .keyframes
            .value_at(AnimatedParameter::Opacity, MediaTime::from_seconds(1)),
        Some(0.25)
    );
}

/// Undoing a key that *replaced* another has to put the old one back, not
/// delete the pair — the case a naive "undo means remove" gets wrong.
#[test]
fn undoing_a_replaced_keyframe_restores_the_old_one() {
    let (mut editor, clip) = editor_with_clip();
    editor
        .set_keyframe(clip, AnimatedParameter::Opacity, key(1, 0.25))
        .unwrap();
    editor
        .set_keyframe(clip, AnimatedParameter::Opacity, key(1, 0.75))
        .unwrap();

    editor.undo().unwrap();

    let keyframes = &clip_of(&editor, clip).keyframes;
    assert_eq!(
        keyframes.len(),
        1,
        "the key was deleted rather than restored"
    );
    assert_eq!(
        keyframes.value_at(AnimatedParameter::Opacity, MediaTime::from_seconds(1)),
        Some(0.25)
    );
}

#[test]
fn removing_a_keyframe_is_undoable() {
    let (mut editor, clip) = editor_with_clip();
    editor
        .set_keyframe(clip, AnimatedParameter::Blur, key(2, 40.0))
        .unwrap();
    editor
        .remove_keyframe(clip, AnimatedParameter::Blur, MediaTime::from_seconds(2))
        .unwrap();
    assert!(clip_of(&editor, clip).keyframes.is_empty());

    editor.undo().unwrap();
    assert_eq!(
        clip_of(&editor, clip)
            .keyframes
            .value_at(AnimatedParameter::Blur, MediaTime::from_seconds(2)),
        Some(40.0)
    );
}

/// A key is placed at the frame the user is looking at, and the playhead is in
/// timeline time while the key is in source time. Getting that conversion wrong
/// puts the key somewhere the user did not ask for, so it is asserted directly.
#[test]
fn a_key_lands_at_the_playhead_in_source_time() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(2));

    editor
        .toggle_keyframe(clip, ClipProperty::Opacity(0.4))
        .unwrap();

    let keyframes = &clip_of(&editor, clip).keyframes;
    let times: Vec<i64> = keyframes.times().iter().map(|t| t.ticks()).collect();
    // Clip starts at timeline 0 reading source 1 s: two seconds in is 3 s.
    assert_eq!(times, vec![MediaTime::from_seconds(3).ticks()]);
}

/// The interface has one control per parameter, not two. Which of the two
/// things it writes is decided here, and this is that decision.
#[test]
fn a_slider_writes_a_static_value_until_the_parameter_is_animated() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(1));

    editor
        .set_clip_value(clip, ClipProperty::Opacity(0.5), false)
        .unwrap();
    assert_eq!(clip_of(&editor, clip).opacity, 0.5);
    assert!(clip_of(&editor, clip).keyframes.is_empty());

    // Animate it, then drag the same slider again.
    editor
        .toggle_keyframe(clip, ClipProperty::Opacity(0.5))
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    editor
        .set_clip_value(clip, ClipProperty::Opacity(0.0), false)
        .unwrap();

    assert_eq!(
        clip_of(&editor, clip).opacity,
        0.5,
        "the static value moved even though the parameter is animated"
    );
    assert_eq!(clip_of(&editor, clip).keyframes.len(), 2);
    assert_eq!(opacity_at(&editor, clip, 1), 0.5);
    assert_eq!(opacity_at(&editor, clip, 3), 0.0);
    let mid = opacity_at(&editor, clip, 2);
    assert!((mid - 0.25).abs() < 1e-6, "midpoint was {mid}");
}

/// §11: dragging a slider produces a value per frame, and one drag must be one
/// undo step — the same rule for a keyed value as for a static one.
#[test]
fn dragging_an_animated_slider_is_one_undo_step() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(2));
    editor
        .toggle_keyframe(clip, ClipProperty::Opacity(1.0))
        .unwrap();

    // The first frame of a drag, then the rest of it.
    editor
        .set_clip_value(clip, ClipProperty::Opacity(0.9), false)
        .unwrap();
    for step in 1..=20_u8 {
        let value = 0.9 - f32::from(step) * 0.04;
        editor
            .set_clip_value(clip, ClipProperty::Opacity(value), true)
            .unwrap();
    }
    assert!(opacity_at(&editor, clip, 2) < 0.2);

    editor.undo().unwrap();
    assert_eq!(
        opacity_at(&editor, clip, 2),
        1.0,
        "undo should reach back to before the drag, not one frame into it"
    );
}

/// Position is one control writing two parameters. Keying it has to be one
/// step, or undo takes the X back and leaves the Y where it was.
#[test]
fn keying_position_moves_both_axes_as_one_step() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(2));

    editor
        .toggle_keyframe(clip, ClipProperty::Position { x: 0.0, y: 0.0 })
        .unwrap();
    let keyframes = &clip_of(&editor, clip).keyframes;
    assert!(keyframes.is_animated(AnimatedParameter::PositionX));
    assert!(keyframes.is_animated(AnimatedParameter::PositionY));

    editor.undo().unwrap();
    let keyframes = &clip_of(&editor, clip).keyframes;
    assert!(
        !keyframes.is_animated(AnimatedParameter::PositionX)
            && !keyframes.is_animated(AnimatedParameter::PositionY),
        "one axis survived the undo"
    );
}

/// Clicking the button again where a key already is deletes it. Without this
/// the only way to remove a key would be to know some other gesture.
#[test]
fn toggling_twice_at_the_same_frame_removes_the_key() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(2));

    editor
        .toggle_keyframe(clip, ClipProperty::Blur(30.0))
        .unwrap();
    assert_eq!(clip_of(&editor, clip).keyframes.len(), 1);

    editor
        .toggle_keyframe(clip, ClipProperty::Blur(30.0))
        .unwrap();
    assert!(clip_of(&editor, clip).keyframes.is_empty());
}

/// A keyframe belongs to a frame, so there has to be one under the playhead.
#[test]
fn keying_with_the_playhead_off_the_clip_is_refused() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(30));

    assert!(
        editor
            .toggle_keyframe(clip, ClipProperty::Opacity(0.5))
            .is_err()
    );
    assert!(clip_of(&editor, clip).keyframes.is_empty());
}

/// Off the clip the slider still works — it writes the static value, which is
/// the only thing it could mean there.
#[test]
fn a_slider_off_the_clip_still_sets_the_static_value() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(30));

    editor
        .set_clip_value(clip, ClipProperty::Blur(12.0), false)
        .unwrap();
    assert_eq!(clip_of(&editor, clip).blur, 12.0);
}

/// But an *animated* parameter has no static value the renderer reads, so the
/// same edit off the clip would appear to do nothing. Refused instead.
#[test]
fn an_animated_slider_off_the_clip_is_refused_rather_than_ignored() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(2));
    editor
        .toggle_keyframe(clip, ClipProperty::Blur(30.0))
        .unwrap();

    editor.set_playhead(TimelineTime::from_seconds(30));
    assert!(
        editor
            .set_clip_value(clip, ClipProperty::Blur(80.0), false)
            .is_err()
    );
    assert_eq!(
        clip_of(&editor, clip)
            .keyframes
            .value_at(AnimatedParameter::Blur, MediaTime::from_seconds(3)),
        Some(30.0),
        "the animation changed from off the clip"
    );
}

/// Reset has to clear the keys too. Resetting the numbers alone would leave a
/// clip that still fades while every control claims it does not.
#[test]
fn reset_clears_the_animation_in_one_step() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(1));
    editor
        .toggle_keyframe(clip, ClipProperty::Opacity(1.0))
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    editor
        .set_clip_value(clip, ClipProperty::Opacity(0.0), false)
        .unwrap();
    editor
        .toggle_keyframe(clip, ClipProperty::Position { x: 0.5, y: 0.5 })
        .unwrap();

    editor.reset_clip_look(clip).unwrap();
    assert!(clip_of(&editor, clip).keyframes.is_empty());
    assert_eq!(opacity_at(&editor, clip, 3), 1.0);

    // One undo step, not one per key.
    editor.undo().unwrap();
    assert_eq!(opacity_at(&editor, clip, 3), 0.0);
    assert_eq!(clip_of(&editor, clip).keyframes.len(), 4);
}

/// §38.2: the journal replays commands to rebuild state after a crash. An
/// animation that does not survive that is an animation the user loses.
#[test]
fn keyframes_survive_a_project_save_and_load() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(2));
    editor
        .toggle_keyframe(clip, ClipProperty::Opacity(0.6))
        .unwrap();
    editor
        .set_keyframe(
            clip,
            AnimatedParameter::Opacity,
            Keyframe::new(MediaTime::from_seconds(4), 0.1, Interpolation::EaseInOut),
        )
        .unwrap();

    let json = serde_json::to_string(editor.project()).expect("serialize");
    let reloaded: bettercut_editor_core::project_format::Project =
        serde_json::from_str(&json).expect("deserialize");

    let restored = reloaded.active().unwrap().video_tracks[0]
        .get(clip)
        .unwrap();
    assert_eq!(restored.keyframes.len(), 2);
    assert_eq!(
        restored
            .keyframes
            .get(AnimatedParameter::Opacity, MediaTime::from_seconds(4))
            .map(|k| k.interpolation),
        Some(Interpolation::EaseInOut),
        "the easing did not survive the round trip"
    );
}

/// A project written before keyframes existed has no field for them, and must
/// still load (§39 — the format is versioned, not fragile).
#[test]
fn a_clip_saved_without_keyframes_loads_as_unanimated() {
    let json = r#"{
        "id": "00000000-0000-0000-0000-000000000001",
        "media_id": "00000000-0000-0000-0000-000000000002",
        "timeline": {"start": 0, "end": 96000},
        "source": {"start": 0, "end": 96000},
        "opacity": 1.0,
        "enabled": true
    }"#;
    let clip: VideoClip = serde_json::from_str(json).expect("older projects must still load");
    assert!(clip.keyframes.is_empty());
    assert_eq!(clip.look_at(MediaTime::ZERO).opacity, 1.0);
}

/// Resetting one control puts it back to default and clears *its* keyframes,
/// leaving every other control alone. Resetting the number while an animation
/// still drove it would look like the button had done nothing.
#[test]
fn resetting_one_control_clears_only_its_own_animation() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(2));

    // Animate opacity, and set blur to something non-default.
    editor
        .toggle_keyframe(clip, ClipProperty::Opacity(1.0))
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    editor
        .set_clip_value(clip, ClipProperty::Opacity(0.0), false)
        .unwrap();
    editor
        .set_clip_value(clip, ClipProperty::Blur(40.0), false)
        .unwrap();

    assert_eq!(clip_of(&editor, clip).keyframes.len(), 2);

    editor
        .reset_clip_parameter(clip, ClipProperty::Opacity(0.0))
        .unwrap();

    let after = clip_of(&editor, clip);
    assert_eq!(after.opacity, 1.0, "opacity should be back to default");
    assert!(
        !after.keyframes.is_animated(AnimatedParameter::Opacity),
        "its keyframes should be gone"
    );
    assert_eq!(after.blur, 40.0, "blur is a different control; leave it");
}

/// Position owns two parameters, and resetting it must take both — a reset that
/// left Y where it was would be worse than none.
#[test]
fn resetting_position_takes_both_axes() {
    let (mut editor, clip) = editor_with_clip();
    editor
        .set_clip_value(clip, ClipProperty::Position { x: 0.3, y: -0.2 }, false)
        .unwrap();
    editor
        .set_clip_value(clip, ClipProperty::Scale { x: 2.0, y: 2.0 }, false)
        .unwrap();

    editor
        .reset_clip_parameter(clip, ClipProperty::Position { x: 0.0, y: 0.0 })
        .unwrap();

    let after = clip_of(&editor, clip);
    assert_eq!(after.transform.position.x, 0.0);
    assert_eq!(after.transform.position.y, 0.0);
    assert_eq!(after.transform.scale.x, 2.0, "scale is a different control");
}

/// One undo step, whatever it had to remove.
#[test]
fn resetting_a_control_undoes_in_one_step() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(1));
    editor
        .toggle_keyframe(clip, ClipProperty::Blur(20.0))
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    editor
        .set_clip_value(clip, ClipProperty::Blur(80.0), false)
        .unwrap();

    let before = clip_of(&editor, clip).keyframes.len();
    assert_eq!(before, 2);

    editor
        .reset_clip_parameter(clip, ClipProperty::Blur(0.0))
        .unwrap();
    assert!(clip_of(&editor, clip).keyframes.is_empty());

    editor.undo().unwrap();
    assert_eq!(
        clip_of(&editor, clip).keyframes.len(),
        before,
        "one undo should bring the whole reset back"
    );
}

/// Whether a control offers its reset button comes from the model's defaults,
/// so the button cannot appear on an untouched control or hide on a changed one.
#[test]
fn a_control_knows_whether_it_is_at_its_default() {
    assert!(ClipProperty::Opacity(1.0).is_default());
    assert!(!ClipProperty::Opacity(0.5).is_default());

    assert!(ClipProperty::Scale { x: 1.0, y: 1.0 }.is_default());
    assert!(!ClipProperty::Scale { x: 1.0, y: 1.5 }.is_default());

    assert!(ClipProperty::Position { x: 0.0, y: 0.0 }.is_default());
    assert!(!ClipProperty::Position { x: 0.0, y: 0.01 }.is_default());

    assert!(ClipProperty::Blur(0.0).is_default());
    assert!(ClipProperty::Rotation(0.0).is_default());
    assert!(ClipProperty::Brightness(1.0).is_default());
    assert!(!ClipProperty::Brightness(1.2).is_default());
}

/// Whole-video adjustments live on the sequence and are undoable like any edit.
#[test]
fn a_sequence_adjustment_applies_to_the_whole_video() {
    let (mut editor, _clip) = editor_with_clip();
    assert!(editor.active_sequence().unwrap().master.is_identity());

    editor
        .set_sequence_value(ClipProperty::Brightness(1.4), false)
        .unwrap();

    let master = editor.active_sequence().unwrap().master;
    assert_eq!(master.color.brightness, 1.4);
    assert!(!master.is_identity());

    editor.undo().unwrap();
    assert!(
        editor.active_sequence().unwrap().master.is_identity(),
        "undo left the whole-video adjustment in place"
    );
}

/// It must not touch the clips: that is the entire distinction between the two.
#[test]
fn a_sequence_adjustment_leaves_the_clips_alone() {
    let (mut editor, clip) = editor_with_clip();
    editor
        .set_clip_value(clip, ClipProperty::Brightness(0.8), false)
        .unwrap();

    editor
        .set_sequence_value(ClipProperty::Brightness(1.5), false)
        .unwrap();

    assert_eq!(
        clip_of(&editor, clip).color.brightness,
        0.8,
        "the sequence adjustment changed the clip"
    );
    assert_eq!(
        editor.active_sequence().unwrap().master.color.brightness,
        1.5
    );
}

/// Dragging a whole-video slider is one undo step, as it is for a clip (§11).
#[test]
fn dragging_a_sequence_slider_is_one_undo_step() {
    let (mut editor, _clip) = editor_with_clip();

    editor
        .set_sequence_value(ClipProperty::Opacity(0.9), false)
        .unwrap();
    for step in 1..=20_u8 {
        let value = 0.9 - f32::from(step) * 0.04;
        editor
            .set_sequence_value(ClipProperty::Opacity(value), true)
            .unwrap();
    }
    assert!(editor.active_sequence().unwrap().master.opacity < 0.2);

    editor.undo().unwrap();
    assert_eq!(
        editor.active_sequence().unwrap().master.opacity,
        1.0,
        "undo should reach back to before the drag"
    );
}

/// Resetting one whole-video control leaves the others.
#[test]
fn resetting_one_sequence_control_leaves_the_rest() {
    let (mut editor, _clip) = editor_with_clip();
    editor
        .set_sequence_value(ClipProperty::Blur(30.0), false)
        .unwrap();
    editor
        .set_sequence_value(ClipProperty::Saturation(0.2), false)
        .unwrap();

    editor
        .reset_sequence_parameter(ClipProperty::Blur(0.0))
        .unwrap();

    let master = editor.active_sequence().unwrap().master;
    assert_eq!(master.blur, 0.0);
    assert_eq!(master.color.saturation, 0.2, "saturation was reset too");
}

/// The master survives a save and load, because it is project data.
#[test]
fn the_sequence_adjustment_survives_a_round_trip() {
    let (mut editor, _clip) = editor_with_clip();
    editor
        .set_sequence_value(ClipProperty::Contrast(1.3), false)
        .unwrap();

    let json = serde_json::to_string(editor.project()).expect("serialize");
    let reloaded: bettercut_editor_core::project_format::Project =
        serde_json::from_str(&json).expect("deserialize");

    assert_eq!(reloaded.active().unwrap().master.color.contrast, 1.3);
}

/// A project written before whole-video adjustments existed loads with none.
#[test]
fn an_older_project_loads_with_no_sequence_adjustment() {
    let json = r#"{
        "id": "00000000-0000-0000-0000-000000000009",
        "name": "Sequence 1",
        "resolution": {"width": 1920, "height": 1080},
        "frame_rate": {"num": 30, "den": 1},
        "video_tracks": [],
        "audio_tracks": []
    }"#;
    let sequence: bettercut_editor_core::timeline::Sequence =
        serde_json::from_str(json).expect("older sequences must still load");
    assert!(sequence.master.is_identity());
}

/// Removing the animation must leave the values alone.
///
/// The two operations were one for a while, behind a button labelled "clear all
/// keyframes" that also reset every colour control — so a graded clip lost its
/// grade to a button that said nothing about colour.
#[test]
fn clearing_keyframes_does_not_reset_the_values() {
    let (mut editor, clip) = editor_with_clip();

    // A colour grade that is *not* animated, and an animated opacity.
    editor
        .set_clip_value(clip, ClipProperty::Saturation(0.3), false)
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(1));
    editor
        .toggle_keyframe(clip, ClipProperty::Opacity(1.0))
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    editor
        .set_clip_value(clip, ClipProperty::Opacity(0.0), false)
        .unwrap();

    assert_eq!(clip_of(&editor, clip).keyframes.len(), 2);

    editor.clear_clip_keyframes(clip).unwrap();

    let after = clip_of(&editor, clip);
    assert!(after.keyframes.is_empty(), "the keys should be gone");
    assert_eq!(
        after.color.saturation, 0.3,
        "the colour grade was reset by a keyframe operation"
    );
}

/// Each control goes back to reading its own value, rather than baking in
/// whatever happened to be on screen.
#[test]
fn clearing_keyframes_returns_controls_to_their_own_value() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(1));
    editor
        .toggle_keyframe(clip, ClipProperty::Opacity(1.0))
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    editor
        .set_clip_value(clip, ClipProperty::Opacity(0.0), false)
        .unwrap();

    // Mid-fade, the picture is half transparent.
    editor.set_playhead(TimelineTime::from_seconds(2));
    let during = opacity_at(&editor, clip, 2);
    assert!(
        (during - 0.5).abs() < 1e-6,
        "expected mid-fade, got {during}"
    );

    editor.clear_clip_keyframes(clip).unwrap();
    assert_eq!(
        opacity_at(&editor, clip, 2),
        1.0,
        "the mid-fade value was baked in instead of the control's own"
    );
}

/// One undo step, however many keys it removed.
#[test]
fn clearing_keyframes_undoes_in_one_step() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(TimelineTime::from_seconds(1));
    editor
        .toggle_keyframe(clip, ClipProperty::Position { x: 0.0, y: 0.0 })
        .unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    editor
        .set_clip_value(clip, ClipProperty::Position { x: 0.4, y: 0.2 }, false)
        .unwrap();

    let before = clip_of(&editor, clip).keyframes.len();
    assert_eq!(before, 4, "two axes, two instants");

    editor.clear_clip_keyframes(clip).unwrap();
    assert!(clip_of(&editor, clip).keyframes.is_empty());

    editor.undo().unwrap();
    assert_eq!(clip_of(&editor, clip).keyframes.len(), before);
}

/// Clearing a clip that has no animation is a no-op, not an error and not an
/// empty entry cluttering the undo history.
#[test]
fn clearing_keyframes_on_an_unanimated_clip_does_nothing() {
    let (mut editor, clip) = editor_with_clip();
    let before = editor.can_undo();

    editor.clear_clip_keyframes(clip).unwrap();

    assert!(clip_of(&editor, clip).keyframes.is_empty());
    assert_eq!(
        editor.can_undo(),
        before,
        "an empty clear pushed something onto the undo stack"
    );
}

/// §24's easing, which the model has always had and nothing could reach: every
/// key a user places is linear, and a whole move built from linear keys is what
/// makes an animation look mechanical.
mod easing {
    use super::*;
    use bettercut_editor_core::timeline::{AnimatedParameter, Interpolation};

    /// A clip with a scale animated between two instants, playhead on the
    /// first key.
    fn editor_with_keys() -> (Editor, ClipId) {
        let (mut editor, clip) = editor_with_clip();
        editor
            .set_clip_value(clip, ClipProperty::Scale { x: 1.0, y: 1.0 }, false)
            .unwrap();
        editor
            .toggle_keyframe(clip, ClipProperty::Scale { x: 1.0, y: 1.0 })
            .unwrap();
        (editor, clip)
    }

    fn easing_at(editor: &Editor, clip: ClipId, parameter: AnimatedParameter) -> Interpolation {
        let at = editor.source_time_at_playhead(clip).expect("on the clip");
        editor
            .video_clip(clip)
            .expect("clip")
            .keyframes
            .get(parameter, at)
            .expect("a key here")
            .interpolation
    }

    /// Both halves of a scale, not one: easing X alone would let the shape
    /// stretch as it moved.
    #[test]
    fn easing_covers_every_parameter_keyed_at_that_instant() {
        let (mut editor, clip) = editor_with_keys();

        assert_eq!(
            editor
                .set_keyframe_easing(clip, Interpolation::EaseInOut)
                .expect("easing refused"),
            2,
            "expected both halves of the scale to move"
        );
        assert_eq!(
            easing_at(&editor, clip, AnimatedParameter::ScaleX),
            Interpolation::EaseInOut
        );
        assert_eq!(
            easing_at(&editor, clip, AnimatedParameter::ScaleY),
            Interpolation::EaseInOut
        );
    }

    /// §79: one choice is one undo step, however many parameters it covered.
    #[test]
    fn changing_the_easing_is_one_undo_step() {
        let (mut editor, clip) = editor_with_keys();
        editor
            .set_keyframe_easing(clip, Interpolation::EaseOut)
            .unwrap();

        editor.undo().unwrap();
        assert_eq!(
            easing_at(&editor, clip, AnimatedParameter::ScaleX),
            Interpolation::Linear,
            "one undo did not put both parameters back"
        );
        assert_eq!(
            easing_at(&editor, clip, AnimatedParameter::ScaleY),
            Interpolation::Linear
        );
    }

    /// Choosing what a key already has must not leave a step in the history
    /// that undoes nothing.
    #[test]
    fn choosing_the_easing_it_already_has_changes_nothing() {
        let (mut editor, clip) = editor_with_keys();
        let depth = editor.undo_depth();

        assert_eq!(
            editor
                .set_keyframe_easing(clip, Interpolation::Linear)
                .expect("easing refused"),
            0
        );
        assert_eq!(editor.undo_depth(), depth, "an empty change made a step");
    }

    /// The value is what it was: this changes how a key is *approached*, not
    /// what it holds.
    #[test]
    fn easing_leaves_the_keys_value_alone() {
        let (mut editor, clip) = editor_with_keys();
        let at = editor.source_time_at_playhead(clip).unwrap();
        let before = editor
            .video_clip(clip)
            .unwrap()
            .keyframes
            .get(AnimatedParameter::ScaleX, at)
            .unwrap()
            .value;

        editor
            .set_keyframe_easing(clip, Interpolation::EaseIn)
            .unwrap();

        let after = editor
            .video_clip(clip)
            .unwrap()
            .keyframes
            .get(AnimatedParameter::ScaleX, at)
            .unwrap()
            .value;
        assert_eq!(before, after, "easing a key moved its value");
    }
}
