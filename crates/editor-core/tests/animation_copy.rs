//! Copy and paste animation between clips (`Editor::copy_animation`,
//! `paste_animation`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AnimatedParameter, Interpolation, Keyframe};

/// Two 6 s shots placed one after the other.
fn two_shots() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Moves");
    let mut ids = Vec::new();
    for name in ["a", "b"] {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(6),
        ));
        ids.push(editor.place_media(media).unwrap()[0]);
    }
    (editor, ids[0], ids[1])
}

fn key(seconds: i64, value: f32) -> Keyframe {
    Keyframe::new(
        MediaTime::from_seconds(seconds),
        value,
        Interpolation::EaseInOut,
    )
}

/// A push in on the first shot lands on the second a second and four seconds
/// into it, replacing the scale animation it had, in one undo step.
#[test]
fn a_move_is_pasted_timed_from_the_clips_start() {
    let (mut editor, first, second) = two_shots();
    editor
        .set_keyframe(first, AnimatedParameter::ScaleX, key(1, 1.0))
        .unwrap();
    editor
        .set_keyframe(first, AnimatedParameter::ScaleX, key(4, 1.4))
        .unwrap();
    editor
        .set_keyframe(second, AnimatedParameter::ScaleX, key(2, 3.0))
        .unwrap();
    editor
        .set_keyframe(second, AnimatedParameter::Opacity, key(3, 0.5))
        .unwrap();

    let copied = editor.copy_animation(first).unwrap();
    assert_eq!(copied.keys.len(), 2);
    let depth = editor.undo_depth();
    assert_eq!(editor.paste_animation(&copied, [second]).unwrap(), 1);
    assert_eq!(editor.undo_depth(), depth + 1);

    let clip = editor.video_clip(second).unwrap();
    let scale: Vec<(MediaTime, f32)> = clip
        .keyframes
        .track(AnimatedParameter::ScaleX)
        .unwrap()
        .keys()
        .iter()
        .map(|k| (k.time, k.value))
        .collect();
    let start = clip.source.start;
    assert_eq!(
        scale,
        vec![
            (
                MediaTime::from_ticks(start.ticks() + TimelineTime::from_seconds(1).ticks()),
                1.0
            ),
            (
                MediaTime::from_ticks(start.ticks() + TimelineTime::from_seconds(4).ticks()),
                1.4
            ),
        ],
        "the old scale key was kept, or the new ones landed elsewhere"
    );
    assert!(
        clip.keyframes.is_animated(AnimatedParameter::Opacity),
        "a parameter the paste did not carry was wiped"
    );

    editor.undo().unwrap();
    let back = editor.video_clip(second).unwrap();
    assert_eq!(
        back.keyframes
            .track(AnimatedParameter::ScaleX)
            .unwrap()
            .keys()
            .len(),
        1
    );
}

/// Keys past the end of a shorter clip are left off.
#[test]
fn keys_beyond_a_short_clip_are_left_off() {
    let (mut editor, first, _) = two_shots();
    editor
        .set_keyframe(first, AnimatedParameter::Opacity, key(1, 0.2))
        .unwrap();
    editor
        .set_keyframe(first, AnimatedParameter::Opacity, key(5, 1.0))
        .unwrap();
    let copied = editor.copy_animation(first).unwrap();

    let short = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/short.mp4",
        MediaTime::from_seconds(2),
    ));
    let target = editor.place_media(short).unwrap()[0];
    editor.paste_animation(&copied, [target]).unwrap();
    let keys = editor
        .video_clip(target)
        .unwrap()
        .keyframes
        .track(AnimatedParameter::Opacity)
        .unwrap()
        .keys()
        .len();
    assert_eq!(keys, 1, "the key at 5 s was put on a 2 s clip");
}

#[test]
fn a_clip_without_keys_has_nothing_to_copy() {
    let (editor, first, _) = two_shots();
    assert!(editor.copy_animation(first).is_none());
}
