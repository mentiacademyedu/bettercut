//! Camera shake (`bettercut_timeline::shake`, `Editor::set_shake`).
//!
//! What a user would notice: the picture actually moves, within its strength
//! and never far enough to uncover the frame's edge; it starts and ends where
//! the shot was framed; an impact settles; the same clip shakes the same way
//! every time; it comes off in one step; and a hand-made animation is never
//! overwritten by it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{
    AnimatedParameter as A, IMPACT_SETTLE, Keyframe, SHAKE_STEP, ShakeStrength, SourceRange, Vec2,
};
use bettercut_editor_core::{ClipProperty, Editor, EditorError, Movement};

fn source(seconds: i64) -> SourceRange {
    SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(seconds)).unwrap()
}

/// A four-second clip placed on the timeline.
fn editor_with_clip() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Shake");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.width = 1920;
    asset.height = 1080;
    let media = editor.import_media(asset);
    let clip = editor.place_media(media).unwrap()[0];
    (editor, clip)
}

fn keys(editor: &Editor, clip: ClipId, parameter: A) -> Vec<Keyframe> {
    editor
        .video_clip(clip)
        .unwrap()
        .keyframes
        .track(parameter)
        .map(|t| t.keys().to_vec())
        .unwrap_or_default()
}

/// The furthest the keys stray from `base`.
fn furthest(keys: &[(A, Keyframe)], parameter: A, base: f32) -> f32 {
    keys.iter()
        .filter(|(p, _)| *p == parameter)
        .map(|(_, key)| (key.value - base).abs())
        .fold(0.0, f32::max)
}

/// Every strength moves the picture — a shake of all zeros would be keys for
/// nothing — and never further than its amplitude, which the overscan covers.
#[test]
fn a_shake_moves_within_what_its_overscan_covers() {
    let base = Vec2 { x: 0.1, y: -0.05 };
    let scale = Vec2 { x: 1.0, y: 1.0 };
    for strength in ShakeStrength::ALL {
        let keys = strength.keyframes(base, scale, source(4));
        for (axis, centre) in [(A::PositionX, base.x), (A::PositionY, base.y)] {
            let travel = furthest(&keys, axis, centre);
            assert!(travel > 0.0, "{strength:?} does not move on {axis:?}");
            assert!(
                travel <= strength.amplitude() + 1e-6,
                "{strength:?} strays {travel} on {axis:?}"
            );
            // Scaling by the overscan grows each side by (s - 1) / 2.
            assert!(
                (strength.overscan() - 1.0) / 2.0 >= travel - 1e-6,
                "{strength:?} uncovers the edge"
            );
        }
        let scales: Vec<f32> = keys
            .iter()
            .filter(|(p, _)| *p == A::ScaleX)
            .map(|(_, k)| k.value)
            .collect();
        assert_eq!(scales, vec![strength.overscan(); 2]);
    }
}

/// It starts and ends where the shot was framed, so a cut into and out of a
/// shaken clip lands on the framing the user chose.
#[test]
fn a_shake_starts_and_ends_at_the_clips_place() {
    let base = Vec2 { x: 0.2, y: 0.3 };
    let keys = ShakeStrength::Strong.keyframes(base, Vec2 { x: 1.0, y: 1.0 }, source(2));
    let x: Vec<&Keyframe> = keys
        .iter()
        .filter(|(p, _)| *p == A::PositionX)
        .map(|(_, k)| k)
        .collect();
    assert_eq!(x.first().unwrap().value, base.x);
    assert_eq!(x.last().unwrap().value, base.x);
    assert_eq!(x.first().unwrap().time, MediaTime::ZERO);
}

/// An impact is a jolt that settles: all its motion inside the settling time,
/// and bigger early than late.
#[test]
fn an_impact_settles() {
    let keys = ShakeStrength::Impact.keyframes(Vec2::ZERO, Vec2 { x: 1.0, y: 1.0 }, source(4));
    let x: Vec<&Keyframe> = keys
        .iter()
        .filter(|(p, _)| *p == A::PositionX)
        .map(|(_, k)| k)
        .collect();
    assert!(x.iter().all(|k| k.time <= IMPACT_SETTLE));
    let early = x[1..4].iter().map(|k| k.value.abs()).fold(0.0, f32::max);
    let late = x[x.len() - 3..]
        .iter()
        .map(|k| k.value.abs())
        .fold(0.0, f32::max);
    assert!(early > late * 2.0, "early {early}, late {late}");
}

/// The same clip shakes the same way every time: a cut timed to a jolt stays
/// timed after a reload.
#[test]
fn a_shake_is_the_same_every_time() {
    let one = ShakeStrength::Strong.keyframes(Vec2::ZERO, Vec2 { x: 1.0, y: 1.0 }, source(3));
    let two = ShakeStrength::Strong.keyframes(Vec2::ZERO, Vec2 { x: 1.0, y: 1.0 }, source(3));
    assert_eq!(one, two);
    // And the two axes are not the same wobble, which would read as a slide.
    let x: Vec<f32> = one
        .iter()
        .filter(|(p, _)| *p == A::PositionX)
        .map(|(_, k)| k.value)
        .collect();
    let y: Vec<f32> = one
        .iter()
        .filter(|(p, _)| *p == A::PositionY)
        .map(|(_, k)| k.value)
        .collect();
    assert_ne!(x, y);
}

/// Through the editor: on, recognised, changed, and off, each one undo step,
/// and off leaves the clip exactly as it was.
#[test]
fn a_shake_goes_on_changes_and_comes_off_in_single_steps() {
    let (mut editor, clip) = editor_with_clip();
    let before = editor.video_clip(clip).unwrap().clone();
    let depth = editor.undo_depth();

    editor.set_shake(clip, Some(ShakeStrength::Gentle)).unwrap();
    assert_eq!(editor.shake_of(clip), Some(ShakeStrength::Gentle));
    assert_eq!(editor.undo_depth(), depth + 1);
    let spacing = keys(&editor, clip, A::PositionX);
    assert_eq!(
        spacing[1].time.ticks() - spacing[0].time.ticks(),
        SHAKE_STEP.ticks()
    );

    editor.set_shake(clip, Some(ShakeStrength::Impact)).unwrap();
    assert_eq!(editor.shake_of(clip), Some(ShakeStrength::Impact));
    assert_eq!(
        keys(&editor, clip, A::ScaleX).len(),
        2,
        "the old overscan was left beside the new one"
    );

    editor.set_shake(clip, None).unwrap();
    assert_eq!(editor.shake_of(clip), None);
    assert_eq!(
        editor.video_clip(clip).unwrap(),
        &before,
        "taking it off left keys behind"
    );

    editor.undo().unwrap();
    assert_eq!(editor.shake_of(clip), Some(ShakeStrength::Impact));
}

/// Position keys set by hand are work: a shake refuses rather than replacing
/// them, and changes nothing.
#[test]
fn a_hand_made_position_animation_is_not_overwritten() {
    let (mut editor, clip) = editor_with_clip();
    // The commonest animation there is: a key where the clip starts, and one
    // later. It must not pass for a shake's grid.
    for (millis, x) in [(0, 0.0), (700, 0.3)] {
        editor.set_playhead(TimelineTime::from_millis(millis));
        editor
            .toggle_keyframe(clip, ClipProperty::Position { x, y: 0.0 })
            .unwrap();
    }
    assert_eq!(
        keys(&editor, clip, A::PositionX).len(),
        2,
        "setup: two keys"
    );
    let before = editor.video_clip(clip).unwrap().clone();

    for strength in [Some(ShakeStrength::Strong), None] {
        let result = editor.set_shake(clip, strength);
        assert!(
            matches!(result, Err(EditorError::AlreadyAnimated("position"))),
            "{result:?}"
        );
        assert_eq!(editor.video_clip(clip).unwrap(), &before);
    }
}

/// A zoom movement's scale is not overwritten by a shake's overscan either —
/// but a shake already on can still be taken off after a zoom was added.
#[test]
fn a_zoom_is_kept() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_movement(clip, Movement::ZoomIn).unwrap();

    let result = editor.set_shake(clip, Some(ShakeStrength::Gentle));
    assert!(
        matches!(result, Err(EditorError::AlreadyAnimated("scale"))),
        "{result:?}"
    );
    assert_eq!(editor.movement_of(clip), Some(Movement::ZoomIn));

    // Shake first, then zoom (which replaces the overscan): off still works,
    // and the zoom survives it.
    let (mut editor, clip) = editor_with_clip();
    editor.set_shake(clip, Some(ShakeStrength::Strong)).unwrap();
    editor.set_movement(clip, Movement::ZoomOut).unwrap();
    editor.set_shake(clip, None).unwrap();
    assert!(keys(&editor, clip, A::PositionX).is_empty());
    assert_eq!(editor.movement_of(clip), Some(Movement::ZoomOut));
}
