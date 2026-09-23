//! A new sequence made of the selected clips (`Editor::sequence_from_clips`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};

#[test]
fn the_selected_clips_start_a_new_sequence_at_zero_and_the_edit_is_untouched() {
    let (mut editor, _events) = Editor::new_project("Long");
    let mut placed = Vec::new();
    for name in ["a", "b", "c"] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(2),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        placed.push(editor.place_media(media).unwrap()[0]);
    }
    editor
        .add_markers(&[TimelineTime::from_seconds(3), TimelineTime::from_seconds(5)])
        .unwrap();
    let original = editor.active_sequence().unwrap().id;
    let before = editor.active_sequence().unwrap().clip_count();

    // b and c: 2–6 s on the original.
    let part = editor
        .sequence_from_clips(&[placed[1], placed[2]], "Short")
        .unwrap();
    assert_eq!(
        editor.active_sequence().unwrap().id,
        original,
        "not switched to"
    );
    assert_eq!(
        editor.active_sequence().unwrap().clip_count(),
        before,
        "the edit changed"
    );

    let short = editor.project().sequence(part).unwrap();
    assert_eq!(short.name, "Short");
    assert_eq!(short.clip_count(), 4, "two pictures and their sounds");
    let starts: Vec<TimelineTime> = short.video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.timeline.start)
        .collect();
    assert_eq!(starts, [TimelineTime::ZERO, TimelineTime::from_seconds(2)]);
    let marks: Vec<TimelineTime> = short.markers.iter().map(|m| m.time).collect();
    assert_eq!(
        marks,
        [TimelineTime::from_seconds(1), TimelineTime::from_seconds(3)]
    );

    editor.undo().unwrap();
    assert!(
        editor.project().sequence(part).is_none(),
        "one step to take it back"
    );
}
