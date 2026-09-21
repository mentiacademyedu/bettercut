//! Split a clip into equal parts or every N seconds
//! (`Editor::split_into_parts`, `Editor::split_every`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A twelve-second shot with its sound, from zero.
fn shot() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Pieces");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/take.mp4",
        MediaTime::from_seconds(12),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let clip = editor.place_media(media).unwrap()[0];
    (editor, clip)
}

fn picture_starts(editor: &Editor) -> Vec<TimelineTime> {
    editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.timeline.start)
        .collect()
}

#[test]
fn equal_parts_are_equal_and_one_step() {
    let (mut editor, clip) = shot();
    let depth = editor.undo_depth();
    assert_eq!(editor.split_into_parts(clip, 4).unwrap(), 3);
    assert_eq!(
        picture_starts(&editor),
        vec![seconds(0), seconds(3), seconds(6), seconds(9)]
    );
    // The sound is in the same four pieces.
    assert_eq!(
        editor.active_sequence().unwrap().audio_tracks[0]
            .clips()
            .len(),
        4
    );
    assert_eq!(editor.undo_depth(), depth + 1);
    editor.undo().unwrap();
    assert_eq!(picture_starts(&editor), vec![seconds(0)]);
}

#[test]
fn every_n_seconds_leaves_the_remainder_last() {
    let (mut editor, clip) = shot();
    assert_eq!(editor.split_every(clip, seconds(5)).unwrap(), 2);
    assert_eq!(
        picture_starts(&editor),
        vec![seconds(0), seconds(5), seconds(10)]
    );

    // An exact multiple makes no empty sliver at the end.
    let (mut editor, clip) = shot();
    assert_eq!(editor.split_every(clip, seconds(4)).unwrap(), 2);
    assert_eq!(
        picture_starts(&editor),
        vec![seconds(0), seconds(4), seconds(8)]
    );
}

#[test]
fn nonsense_splits_are_refused() {
    let (mut editor, clip) = shot();
    for parts in [0, 1, 101] {
        assert!(matches!(
            editor.split_into_parts(clip, parts),
            Err(EditorError::SplitTooFine)
        ));
    }
    assert!(matches!(
        editor.split_every(clip, seconds(12)),
        Err(EditorError::SplitTooFine)
    ));
    assert!(matches!(
        editor.split_every(clip, TimelineTime::from_millis(10)),
        Err(EditorError::SplitTooFine)
    ));
    assert!(matches!(
        editor.split_every(clip, TimelineTime::ZERO),
        Err(EditorError::SplitTooFine)
    ));
    assert_eq!(picture_starts(&editor), vec![seconds(0)]);
}
