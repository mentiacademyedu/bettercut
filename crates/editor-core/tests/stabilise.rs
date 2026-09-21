//! Steadying a shaky shot (`Editor::stabilise`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TICKS_PER_SECOND, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::stabilise::MAX_ZOOM;
use bettercut_editor_core::timeline::AnimatedParameter;
use bettercut_editor_core::track_motion::TrackedPoint;
use bettercut_editor_core::{Editor, EditorError};

/// A ten-second shot on V1.
fn shot() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Steady");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/handheld.mp4",
        MediaTime::from_seconds(10),
    ));
    let clip = editor.place_media(media).unwrap()[0];
    (editor, clip)
}

/// A path that pans steadily right with a wobble on top of it.
fn shaky(points: usize, wobble: f32) -> Vec<TrackedPoint> {
    (0..points)
        .map(|index| {
            let through = index as f32 / (points - 1) as f32;
            let shake = if index % 2 == 0 { wobble } else { -wobble };
            TrackedPoint {
                at: TimelineTime::from_ticks(
                    (through * 4.0 * TICKS_PER_SECOND as f32).round() as i64
                ),
                center: [0.3 + through * 0.4 + shake, 0.5 + shake],
            }
        })
        .collect()
}

fn key_at(editor: &Editor, clip: ClipId, parameter: AnimatedParameter, at: MediaTime) -> f32 {
    editor
        .video_clip(clip)
        .unwrap()
        .keyframes
        .value_at(parameter, at)
        .unwrap()
}

#[test]
fn the_wobble_is_cancelled_and_the_pan_is_kept() {
    let (mut editor, clip) = shot();
    let depth = editor.undo_depth();
    let path = shaky(21, 0.02);

    let done = editor.stabilise(clip, &path).unwrap();

    assert_eq!(done.keys, path.len());
    assert!(done.zoom > 1.0 && done.zoom <= MAX_ZOOM, "{done:?}");
    // Cropped in by twice the worst correction, so the edges stay hidden.
    let scale = editor.video_clip(clip).unwrap().transform.scale;
    assert!((scale.x - done.zoom).abs() < 1e-4, "{scale:?}");
    assert_eq!(editor.undo_depth(), depth + 1);

    // Two neighbouring frames wobble opposite ways, so their corrections do
    // too — that is the shake being cancelled rather than followed.
    let (a, b) = (
        key_at(
            &editor,
            clip,
            AnimatedParameter::PositionX,
            MediaTime::from_ticks(path[8].at.ticks()),
        ),
        key_at(
            &editor,
            clip,
            AnimatedParameter::PositionX,
            MediaTime::from_ticks(path[9].at.ticks()),
        ),
    );
    assert!(a * b < 0.0, "corrections {a} and {b} point the same way");
    // And the pan itself is not fought: the correction never grows with it.
    let worst = path
        .iter()
        .map(|point| {
            key_at(
                &editor,
                clip,
                AnimatedParameter::PositionX,
                MediaTime::from_ticks(point.at.ticks()),
            )
            .abs()
        })
        .fold(0.0_f32, f32::max);
    assert!(worst < 0.06, "the pan was fought: {worst}");

    editor.undo().unwrap();
    assert!(editor.video_clip(clip).unwrap().keyframes.is_empty());
    assert!((editor.video_clip(clip).unwrap().transform.scale.x - 1.0).abs() < 1e-6);
}

#[test]
fn a_shot_that_moves_too_much_is_refused_and_left_alone() {
    let (mut editor, clip) = shot();
    let path = shaky(21, 0.4);
    assert!(matches!(
        editor.stabilise(clip, &path),
        Err(EditorError::TooShakyToSteady)
    ));
    assert!(editor.video_clip(clip).unwrap().keyframes.is_empty());
    assert!((editor.video_clip(clip).unwrap().transform.scale.x - 1.0).abs() < 1e-6);
}

#[test]
fn a_path_too_short_to_tell_a_wobble_from_a_pan_says_so() {
    let (mut editor, clip) = shot();
    assert!(matches!(
        editor.stabilise(clip, &shaky(3, 0.01)[..2]),
        Err(EditorError::NothingTracked)
    ));
}
