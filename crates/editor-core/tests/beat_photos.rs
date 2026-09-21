//! Photos cut to the beat (`Editor::photos_to_beats`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::Movement;
use bettercut_editor_core::{Editor, EditorError};

fn millis(n: i64) -> TimelineTime {
    TimelineTime::from_millis(n)
}

fn photos(editor: &mut Editor, count: usize) -> Vec<MediaId> {
    (0..count)
        .map(|i| {
            editor.import_media(MediaAsset::new(
                MediaKind::Image,
                format!("C:/photos/{i}.jpg"),
                MediaTime::ZERO,
            ))
        })
        .collect()
}

#[test]
fn each_photo_runs_from_its_beat_to_the_next() {
    let (mut editor, _events) = Editor::new_project("Beats");
    let pictures = photos(&mut editor, 3);
    editor
        .add_markers(&[
            millis(500),
            millis(1000),
            millis(1500),
            millis(2000),
            millis(2500),
        ])
        .unwrap();
    let depth = editor.undo_depth();

    let clips = editor.photos_to_beats(&pictures, Movement::ZoomIn).unwrap();
    assert_eq!(
        clips.len(),
        3,
        "one photo per beat, stopping when the photos run out"
    );
    let spans: Vec<_> = clips
        .iter()
        .map(|c| editor.video_clip(*c).unwrap().timeline)
        .collect();
    assert_eq!((spans[0].start, spans[0].end), (millis(500), millis(1000)));
    assert_eq!((spans[2].start, spans[2].end), (millis(1500), millis(2000)));
    assert!(editor.movement_of(clips[0]) == Some(Movement::ZoomIn));
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert!(editor.video_clip(clips[0]).is_none());
}

#[test]
fn more_photos_than_beats_stop_at_the_last_beat_plus_one() {
    let (mut editor, _events) = Editor::new_project("Beats");
    let pictures = photos(&mut editor, 5);
    editor
        .add_markers(&[millis(0), millis(400), millis(800)])
        .unwrap();
    let clips = editor.photos_to_beats(&pictures, Movement::None).unwrap();
    assert_eq!(clips.len(), 3);
    let last = editor.video_clip(clips[2]).unwrap().timeline;
    // One beat's length past the last marker.
    assert_eq!((last.start, last.end), (millis(800), millis(1200)));
}

#[test]
fn without_beats_or_photos_nothing_happens() {
    let (mut editor, _events) = Editor::new_project("Beats");
    let pictures = photos(&mut editor, 2);
    assert!(matches!(
        editor.photos_to_beats(&pictures, Movement::None),
        Err(EditorError::NotEnoughBeats)
    ));
    editor.add_markers(&[millis(0), millis(500)]).unwrap();
    assert!(matches!(
        editor.photos_to_beats(&[], Movement::None),
        Err(EditorError::NothingToCutToBeats)
    ));
}
