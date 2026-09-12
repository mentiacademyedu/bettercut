//! The slow-zoom presets, written as keyframes (§24).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::AnimatedParameter;
use bettercut_editor_core::{Editor, Movement, ZOOM_AMOUNT};

fn editor_with_clip() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Movement");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(8),
    ));
    let clip = editor.place_media(media).unwrap()[0];
    (editor, clip)
}

fn scale_keys(editor: &Editor, clip: ClipId) -> Vec<(MediaTime, f32)> {
    editor
        .video_clip(clip)
        .unwrap()
        .keyframes
        .track(AnimatedParameter::ScaleX)
        .map(|t| t.keys().iter().map(|k| (k.time, k.value)).collect())
        .unwrap_or_default()
}

#[test]
fn a_zoom_in_keys_the_scale_across_the_clip() {
    let (mut editor, clip) = editor_with_clip();
    let depth = editor.undo_depth();

    editor.set_movement(clip, Movement::ZoomIn).unwrap();

    let keys = scale_keys(&editor, clip);
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].0, MediaTime::ZERO);
    assert_eq!(keys[0].1, 1.0);
    assert_eq!(keys[1].0, MediaTime::from_ticks(8 * 960_000 - 1));
    assert_eq!(keys[1].1, ZOOM_AMOUNT);
    // Both axes, or the picture would stretch as it zoomed.
    assert!(
        editor
            .video_clip(clip)
            .unwrap()
            .keyframes
            .is_animated(AnimatedParameter::ScaleY)
    );
    assert_eq!(editor.undo_depth(), depth + 1, "one undo step");

    editor.undo().unwrap();
    assert!(scale_keys(&editor, clip).is_empty());
}

#[test]
fn a_zoom_out_runs_the_other_way() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_movement(clip, Movement::ZoomOut).unwrap();
    let keys = scale_keys(&editor, clip);
    assert_eq!(keys[0].1, ZOOM_AMOUNT);
    assert_eq!(keys[1].1, 1.0);
}

/// Switching replaces rather than layering, and "none" takes it away.
#[test]
fn movements_replace_each_other_and_can_be_removed() {
    let (mut editor, clip) = editor_with_clip();

    editor.set_movement(clip, Movement::ZoomIn).unwrap();
    editor.set_movement(clip, Movement::ZoomOut).unwrap();
    let keys = scale_keys(&editor, clip);
    assert_eq!(keys.len(), 2, "the first zoom's keys were left behind");
    assert_eq!(keys[0].1, ZOOM_AMOUNT);

    editor.set_movement(clip, Movement::None).unwrap();
    assert!(scale_keys(&editor, clip).is_empty());
    assert!(
        !editor
            .video_clip(clip)
            .unwrap()
            .keyframes
            .is_animated(AnimatedParameter::ScaleY)
    );
}

/// A clip set to fill the frame keeps filling it while it moves.
#[test]
fn a_movement_is_relative_to_the_clips_own_framing() {
    use bettercut_editor_core::ClipProperty;

    let (mut editor, clip) = editor_with_clip();
    editor
        .set_clip_property(clip, ClipProperty::Scale { x: 1.5, y: 1.5 }, false)
        .unwrap();

    editor.set_movement(clip, Movement::ZoomIn).unwrap();

    let keys = scale_keys(&editor, clip);
    assert_eq!(keys[0].1, 1.5, "the zoom started from the wrong size");
    assert!((keys[1].1 - 1.5 * ZOOM_AMOUNT).abs() < 1e-5);
}

#[test]
fn the_movement_on_a_clip_can_be_read_back() {
    let (mut editor, clip) = editor_with_clip();
    assert_eq!(editor.movement_of(clip), Some(Movement::None));

    editor.set_movement(clip, Movement::ZoomIn).unwrap();
    assert_eq!(editor.movement_of(clip), Some(Movement::ZoomIn));

    editor.set_movement(clip, Movement::ZoomOut).unwrap();
    assert_eq!(editor.movement_of(clip), Some(Movement::ZoomOut));
}

/// The move is what actually gets rendered: the value the renderer asks for
/// grows across the clip (§24 resolves keys in the timeline crate).
#[test]
fn the_picture_really_grows_across_the_clip() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_movement(clip, Movement::ZoomIn).unwrap();

    let video = editor.video_clip(clip).unwrap();
    let at = |seconds: i64| {
        video
            .look_at(MediaTime::from_seconds(seconds))
            .transform
            .scale
            .x
    };
    assert!(at(0) < at(4), "{} was not less than {}", at(0), at(4));
    assert!(at(4) < at(7));
    assert_eq!(video.timeline.duration(), TimelineTime::from_seconds(8));
}
