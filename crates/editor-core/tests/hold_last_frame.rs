//! Holding a clip's last frame (`Editor::hold_last_frame`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A 4 s shot then a 3 s shot: holding the first one's last frame for 2 s
/// puts a frozen clip at 4–6 s showing its final frame, and the next shot
/// moves to 6 s — in one undo step.
#[test]
fn the_last_frame_is_held_and_the_rest_moves_along() {
    let (mut editor, _events) = Editor::new_project("Hold");
    let mut ids = Vec::new();
    for (name, length) in [("a", 4), ("b", 3)] {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(length),
        ));
        ids.push(editor.place_media(media).unwrap()[0]);
    }
    let (first, second) = (ids[0], ids[1]);
    let depth = editor.undo_depth();

    let held = editor.hold_last_frame(first, seconds(2)).unwrap();
    assert_eq!(editor.undo_depth(), depth + 1);
    let clip = editor.video_clip(held).unwrap();
    assert!(clip.frozen);
    assert_eq!(
        (clip.timeline.start, clip.timeline.end),
        (seconds(4), seconds(6))
    );
    let original = editor.video_clip(first).unwrap();
    assert!(
        clip.source.start < original.source.end && clip.source.start >= original.source.start,
        "the held frame is not one of the clip's own"
    );
    assert!(
        original.source.end.ticks() - clip.source.start.ticks()
            <= TimelineTime::from_millis(100).ticks(),
        "the held frame is not the last one"
    );
    let next = editor.active_sequence().unwrap().clip_span(second).unwrap();
    assert_eq!(next.timeline.start, seconds(6));

    editor.undo().unwrap();
    assert!(editor.video_clip(held).is_none());
    let next = editor.active_sequence().unwrap().clip_span(second).unwrap();
    assert_eq!(next.timeline.start, seconds(4));
}

#[test]
fn sound_cannot_hold_a_frame() {
    let (mut editor, _events) = Editor::new_project("Hold");
    let mut song = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/s.mp3",
        MediaTime::from_seconds(4),
    );
    song.audio_codec = Some("mp3".to_owned());
    let song = editor.import_media(song);
    let clip = editor.place_media(song).unwrap()[0];
    assert!(matches!(
        editor.hold_last_frame(clip, seconds(1)),
        Err(EditorError::ClipKindMismatch)
    ));
}
