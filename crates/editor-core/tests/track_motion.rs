//! Attaching a clip to a tracked path (`Editor::attach_to_path`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AnimatedParameter, SourceRange, VideoClip};
use bettercut_editor_core::track_motion::TrackedPoint;
use bettercut_editor_core::{ClipPayload, Editor, EditorError};

fn secs(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn place(editor: &mut Editor, track: TrackId, name: &str, start: i64, length: i64) -> ClipId {
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        format!("C:/media/{name}.mp4"),
        MediaTime::from_seconds(60),
    ));
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(length)).unwrap();
    let clip = VideoClip::new(media, secs(start), source).unwrap();
    let id = clip.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    id
}

/// Footage on V1 and a small sticker over it on V2, both 0–10 s.
fn footage_and_sticker() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Track");
    editor.add_video_track("V2").unwrap();
    let sequence = editor.active_sequence().unwrap();
    let (first, second) = (sequence.video_tracks[0].id, sequence.video_tracks[1].id);
    let footage = place(&mut editor, first, "shot", 0, 10);
    let sticker = place(&mut editor, second, "sticker", 0, 10);
    // A quarter-size sticker, a little left of centre.
    editor
        .set_clip_property(
            sticker,
            bettercut_editor_core::ClipProperty::Scale { x: 0.25, y: 0.25 },
            false,
        )
        .unwrap();
    editor
        .set_clip_property(
            sticker,
            bettercut_editor_core::ClipProperty::Position { x: -0.1, y: 0.0 },
            false,
        )
        .unwrap();
    (editor, footage, sticker)
}

#[test]
fn the_clips_own_box_is_what_gets_followed() {
    let (editor, _, sticker) = footage_and_sticker();
    let (center, half) = editor.clip_box(sticker).unwrap();
    assert!((center[0] - 0.4).abs() < 1e-5, "{center:?}");
    assert!((center[1] - 0.5).abs() < 1e-5, "{center:?}");
    assert!((half[0] - 0.125).abs() < 1e-5, "{half:?}");
}

#[test]
fn a_tracked_path_becomes_position_keyframes_that_keep_the_offset() {
    let (mut editor, _, sticker) = footage_and_sticker();
    let depth = editor.undo_depth();

    // Whatever was under the sticker moved right across two seconds.
    let path = vec![
        TrackedPoint {
            at: secs(0),
            center: [0.4, 0.5],
        },
        TrackedPoint {
            at: secs(1),
            center: [0.5, 0.6],
        },
        TrackedPoint {
            at: secs(2),
            center: [0.6, 0.7],
        },
    ];
    let written = editor.attach_to_path(sticker, &path).unwrap();
    assert_eq!(written, 3);
    assert_eq!(editor.undo_depth(), depth + 1);

    let clip = editor.video_clip(sticker).unwrap();
    // The first key leaves the sticker exactly where it was — the offset it
    // had from the thing it is following is kept.
    let x0 = clip
        .keyframes
        .value_at(AnimatedParameter::PositionX, MediaTime::ZERO)
        .unwrap();
    assert!((x0 + 0.1).abs() < 1e-5, "{x0}");
    // And by the last one it has moved by as much as the patch did.
    let x2 = clip
        .keyframes
        .value_at(AnimatedParameter::PositionX, MediaTime::from_seconds(2))
        .unwrap();
    assert!((x2 - 0.1).abs() < 1e-5, "{x2}");
    // Down the frame in tracker units is down the frame on screen, which is
    // negative in the clip's own units.
    let y2 = clip
        .keyframes
        .value_at(AnimatedParameter::PositionY, MediaTime::from_seconds(2))
        .unwrap();
    assert!((y2 + 0.2).abs() < 1e-5, "{y2}");

    editor.undo().unwrap();
    assert!(editor.video_clip(sticker).unwrap().keyframes.is_empty());
}

/// A second track replaces the first rather than leaving two sets of keys
/// fighting each other.
#[test]
fn tracking_again_replaces_the_keys() {
    let (mut editor, _, sticker) = footage_and_sticker();
    let path = |x: f32| {
        vec![
            TrackedPoint {
                at: secs(0),
                center: [0.4, 0.5],
            },
            TrackedPoint {
                at: secs(3),
                center: [x, 0.5],
            },
        ]
    };
    editor.attach_to_path(sticker, &path(0.9)).unwrap();
    editor.attach_to_path(sticker, &path(0.2)).unwrap();

    let clip = editor.video_clip(sticker).unwrap();
    assert_eq!(
        clip.keyframes
            .track(AnimatedParameter::PositionX)
            .unwrap()
            .len(),
        2
    );
    let last = clip
        .keyframes
        .value_at(AnimatedParameter::PositionX, MediaTime::from_seconds(3))
        .unwrap();
    assert!((last + 0.3).abs() < 1e-5, "{last}");
}

#[test]
fn a_track_with_nothing_in_it_says_so() {
    let (mut editor, _, sticker) = footage_and_sticker();
    assert!(matches!(
        editor.attach_to_path(sticker, &[]),
        Err(EditorError::NothingTracked)
    ));
}

#[test]
fn the_span_is_the_overlap_from_the_playhead_and_never_too_long() {
    let (mut editor, footage, sticker) = footage_and_sticker();
    editor.set_playhead(secs(2));
    let (start, end) = editor.track_span(sticker, footage).unwrap();
    assert_eq!((start, end), (secs(2), secs(10)));

    // Past the end of both, there is nothing to follow.
    editor.set_playhead(secs(11));
    assert_eq!(editor.track_span(sticker, footage), None);
}
