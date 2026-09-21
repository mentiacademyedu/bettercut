//! Moving and deleting a keyframe that is already there — what the animation
//! graph does to the curve it draws.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AnimatedParameter, Interpolation, Keyframe};
use bettercut_editor_core::{Editor, EditorError};

/// A ten-second shot whose opacity is keyed at 1 s and 5 s.
fn keyed_shot() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Keys");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(10),
    ));
    let clip = editor.place_media(media).unwrap()[0];
    for (at, value) in [(1, 1.0), (5, 0.0)] {
        editor
            .set_keyframe(
                clip,
                AnimatedParameter::Opacity,
                Keyframe::new(MediaTime::from_seconds(at), value, Interpolation::Linear),
            )
            .unwrap();
    }
    (editor, clip)
}

fn keys(editor: &Editor, clip: ClipId) -> Vec<(i64, f32)> {
    editor
        .keyframes_of(clip, AnimatedParameter::Opacity)
        .into_iter()
        .map(|key| (key.time.ticks(), key.value))
        .collect()
}

#[test]
fn a_key_dragged_to_another_time_and_value_lands_there() {
    let (mut editor, clip) = keyed_shot();
    let depth = editor.undo_depth();

    editor
        .move_keyframe(
            clip,
            AnimatedParameter::Opacity,
            MediaTime::from_seconds(5),
            MediaTime::from_seconds(7),
            0.4,
        )
        .unwrap();

    assert_eq!(
        keys(&editor, clip),
        vec![
            (MediaTime::from_seconds(1).ticks(), 1.0),
            (MediaTime::from_seconds(7).ticks(), 0.4),
        ]
    );
    // The easing the key was given is kept: a drag moves a point, it does not
    // reset how the curve leaves it.
    let moved = editor
        .keyframes_of(clip, AnimatedParameter::Opacity)
        .into_iter()
        .find(|key| key.time == MediaTime::from_seconds(7))
        .unwrap();
    assert_eq!(moved.interpolation, Interpolation::Linear);
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(
        keys(&editor, clip),
        vec![
            (MediaTime::from_seconds(1).ticks(), 1.0),
            (MediaTime::from_seconds(5).ticks(), 0.0),
        ]
    );
}

#[test]
fn a_value_dragged_past_the_parameters_limit_stops_at_it() {
    let (mut editor, clip) = keyed_shot();
    editor
        .move_keyframe(
            clip,
            AnimatedParameter::Opacity,
            MediaTime::from_seconds(1),
            MediaTime::from_seconds(1),
            4.0,
        )
        .unwrap();
    assert_eq!(keys(&editor, clip)[0].1, 1.0);
}

#[test]
fn a_key_can_be_deleted_where_it_stands() {
    let (mut editor, clip) = keyed_shot();
    editor
        .remove_keyframe_at(clip, AnimatedParameter::Opacity, MediaTime::from_seconds(1))
        .unwrap();
    assert_eq!(
        keys(&editor, clip),
        vec![(MediaTime::from_seconds(5).ticks(), 0.0)]
    );

    // And one that is not there says so rather than doing nothing quietly.
    assert!(matches!(
        editor.remove_keyframe_at(clip, AnimatedParameter::Opacity, MediaTime::from_seconds(1)),
        Err(EditorError::NoKeyframeThere)
    ));
}

#[test]
fn the_easing_of_one_key_can_be_changed_on_its_own() {
    let (mut editor, clip) = keyed_shot();
    editor
        .set_keyframe_interpolation(
            clip,
            AnimatedParameter::Opacity,
            MediaTime::from_seconds(1),
            Interpolation::EaseInOut,
        )
        .unwrap();
    let first = editor
        .keyframes_of(clip, AnimatedParameter::Opacity)
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(first.interpolation, Interpolation::EaseInOut);
    assert_eq!(first.value, 1.0);
}
