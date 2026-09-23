//! Moving a position key from the motion path (`Editor::move_position_key`):
//! both axes at one time, easing kept, one step for a drag, and refused
//! where there is no path to move.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{ClipId, MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{
    AnimatedParameter, Interpolation, Keyframe, SourceRange, Vec2, VideoClip,
};

fn key(seconds: i64, value: f32, interpolation: Interpolation) -> Keyframe {
    Keyframe::new(MediaTime::from_seconds(seconds), value, interpolation)
}

/// A ten-second shot sliding left to right, its second x key eased.
fn moving() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Path");
    let mut clip = VideoClip::new(
        MediaId::new(),
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    clip.keyframes.set(
        AnimatedParameter::PositionX,
        key(0, -0.4, Interpolation::Linear),
    );
    clip.keyframes.set(
        AnimatedParameter::PositionX,
        key(
            8,
            0.4,
            Interpolation::Bezier {
                x1: 0.4,
                y1: 0.0,
                x2: 0.6,
                y2: 1.0,
            },
        ),
    );
    let id = clip.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    (editor, id)
}

#[test]
fn a_key_moves_on_both_axes_at_its_own_time() {
    let (mut editor, clip) = moving();
    let at = MediaTime::from_seconds(8);
    editor
        .move_position_key(clip, at, Vec2::new(0.1, -0.3), false)
        .unwrap();

    let moved = editor.video_clip(clip).unwrap();
    let position = moved.look_at(at).transform.position;
    assert!((position.x - 0.1).abs() < 1e-6 && (position.y + 0.3).abs() < 1e-6);
    // The first key did not move: this is a key, not the whole line.
    let first = moved.look_at(MediaTime::ZERO).transform.position;
    assert!((first.x + 0.4).abs() < 1e-6, "{first:?}");
    // And y, which had no key there, now has one — the path gained a point.
    assert!(moved.keyframes.is_animated(AnimatedParameter::PositionY));
}

/// The key's easing is what the person chose; a drag changes where it goes,
/// not how it gets there.
#[test]
fn moving_a_key_keeps_its_easing() {
    let (mut editor, clip) = moving();
    let at = MediaTime::from_seconds(8);
    editor
        .move_position_key(clip, at, Vec2::new(0.2, 0.0), false)
        .unwrap();
    let key = editor
        .video_clip(clip)
        .unwrap()
        .keyframes
        .get(AnimatedParameter::PositionX, at)
        .expect("the key is still there");
    assert!(matches!(key.interpolation, Interpolation::Bezier { .. }));
}

/// A drag is one step, not a hundred, and undo puts the key back.
#[test]
fn a_drag_is_one_step_and_undoes_whole() {
    let (mut editor, clip) = moving();
    let at = MediaTime::from_seconds(8);
    let depth = editor.undo_depth();
    for step in 1..=10 {
        let x = 0.4 - step as f32 * 0.05;
        editor
            .move_position_key(clip, at, Vec2::new(x, 0.0), step > 1)
            .unwrap();
    }
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    let back = editor
        .video_clip(clip)
        .unwrap()
        .look_at(at)
        .transform
        .position;
    assert!((back.x - 0.4).abs() < 1e-6, "{back:?}");
}

/// A still picture has no path to move a key on.
#[test]
fn a_still_picture_refuses() {
    let (mut editor, _events) = Editor::new_project("Still");
    let clip = VideoClip::new(
        MediaId::new(),
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    let id = clip.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    assert!(
        editor
            .move_position_key(id, MediaTime::from_seconds(1), Vec2::new(0.1, 0.1), false)
            .is_err()
    );
}
