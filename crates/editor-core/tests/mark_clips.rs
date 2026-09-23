//! In and out marked around clips (`Editor::mark_clips`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn in_and_out_span_the_chosen_clips() {
    let (mut editor, _events) = Editor::new_project("Marks");
    let mut placed = Vec::new();
    for name in ["a", "b", "c"] {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(2),
        ));
        placed.push(editor.place_media(media).unwrap()[0]);
    }
    let range = editor.mark_clips(&[placed[2], placed[1]]).unwrap().unwrap();
    assert_eq!(range.start, TimelineTime::from_seconds(2));
    assert_eq!(range.end, TimelineTime::from_seconds(6));
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.mark_in, Some(TimelineTime::from_seconds(2)));
    assert_eq!(sequence.mark_out, Some(TimelineTime::from_seconds(6)));
    assert_eq!(editor.mark_clips(&[]).unwrap(), None);
}
