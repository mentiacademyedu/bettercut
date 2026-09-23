//! Filling the gap after a clip with more of it (`Editor::extend_to_next`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{Editor, EditorError};

#[test]
fn the_end_meets_the_next_clip_as_far_as_the_footage_goes() {
    let (mut editor, _events) = Editor::new_project("Extend");
    // Ten seconds of file; the first clip plays 0-2 s of it.
    let media = editor.import_media(bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Video,
        "C:/media/long.mp4",
        MediaTime::from_seconds(10),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let piece = |from: i64, to: i64| {
        SourceRange::new(MediaTime::from_seconds(from), MediaTime::from_seconds(to)).unwrap()
    };
    let first = VideoClip::new(media, TimelineTime::ZERO, piece(0, 2)).unwrap();
    let near = VideoClip::new(media, TimelineTime::from_seconds(5), piece(0, 1)).unwrap();
    let (a, b) = (first.id, near.id);
    editor
        .add_clip(track, ClipPayload::Video(Box::new(first)))
        .unwrap();
    editor
        .add_clip(track, ClipPayload::Video(Box::new(near)))
        .unwrap();

    assert_eq!(
        editor.extend_to_next(a).unwrap(),
        TimelineTime::from_seconds(5)
    );
    assert_eq!(editor.clip_end(a), Some(TimelineTime::from_seconds(5)));
    assert!(matches!(
        editor.extend_to_next(a),
        Err(EditorError::NothingToExtendTo)
    ));

    // Nothing after the second: nothing to fill.
    assert!(matches!(
        editor.extend_to_next(b),
        Err(EditorError::NothingToExtendTo)
    ));
}
