//! Clip marks (`Editor::toggle_clip_mark`): kept on the footage, so they
//! travel when the clip moves and hide when it is trimmed past them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_media::{MediaAsset, MediaKind};

fn shot() -> (Editor, TrackId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Marks");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/take.mp4",
        MediaTime::from_seconds(20),
    ));
    let clip = editor.place_media(media).unwrap()[0];
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    (editor, track, clip)
}

#[test]
fn a_mark_travels_with_the_clip_and_toggles_off() {
    let (mut editor, track, clip) = shot();
    editor.set_playhead(TimelineTime::from_seconds(5));
    let depth = editor.undo_depth();

    assert!(editor.toggle_clip_mark(clip).unwrap());
    assert_eq!(editor.clip_marks(clip), [TimelineTime::from_seconds(5)]);
    assert_eq!(editor.undo_depth(), depth + 1);

    // Moved five seconds later, the mark is five seconds later too — where
    // a sequence marker would have stayed at five.
    editor
        .move_clip(track, track, clip, TimelineTime::from_seconds(5))
        .unwrap();
    assert_eq!(editor.clip_marks(clip), [TimelineTime::from_seconds(10)]);

    // The same frame again takes it off.
    editor.set_playhead(TimelineTime::from_seconds(10));
    assert!(!editor.toggle_clip_mark(clip).unwrap());
    assert!(editor.clip_marks(clip).is_empty());
    editor.undo().unwrap();
    assert_eq!(editor.clip_marks(clip).len(), 1);
}

#[test]
fn a_mark_trimmed_past_is_hidden_and_comes_back() {
    let (mut editor, track, clip) = shot();
    editor.set_playhead(TimelineTime::from_seconds(2));
    editor.toggle_clip_mark(clip).unwrap();
    assert_eq!(editor.clip_marks(clip).len(), 1);

    // Trim the head past the mark: it is not on the timeline any more.
    editor
        .trim_clip(
            track,
            clip,
            bettercut_editor_core::TrimEdge::Start,
            TimelineTime::from_seconds(4),
        )
        .unwrap();
    assert!(
        editor.clip_marks(clip).is_empty(),
        "the mark survived a trim past it"
    );

    editor.undo().unwrap();
    assert_eq!(
        editor.clip_marks(clip).len(),
        1,
        "the mark did not come back"
    );

    assert_eq!(editor.clear_clip_marks(clip).unwrap(), 1);
    assert_eq!(editor.clear_clip_marks(clip).unwrap(), 0);
    assert!(editor.toggle_clip_mark(ClipId::new()).is_err());
}
