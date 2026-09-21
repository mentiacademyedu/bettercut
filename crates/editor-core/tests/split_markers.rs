//! Split every clip at the markers (`Editor::split_at_markers`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn starts(editor: &Editor) -> (Vec<TimelineTime>, Vec<TimelineTime>) {
    let sequence = editor.active_sequence().unwrap();
    let of = |clips: Vec<TimelineTime>| {
        let mut clips = clips;
        clips.sort();
        clips
    };
    (
        of(sequence.video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline.start)
            .collect()),
        of(sequence.audio_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline.start)
            .collect()),
    )
}

/// A 10 s shot with sound and markers at 3 s and 7 s: three pieces of
/// picture and three of sound, in one undo step.
#[test]
fn every_clip_is_cut_at_every_marker() {
    let (mut editor, _events) = Editor::new_project("Beats");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/dance.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    editor
        .add_markers(&[seconds(3), seconds(7), seconds(12)])
        .unwrap();
    let depth = editor.undo_depth();

    let splits = editor.split_at_markers(None).unwrap();
    assert_eq!(splits, 4, "two cuts in the picture and two in the sound");
    assert_eq!(editor.undo_depth(), depth + 1);
    let (picture, sound) = starts(&editor);
    assert_eq!(picture, vec![seconds(0), seconds(3), seconds(7)]);
    assert_eq!(sound, picture, "the sound was not cut with the picture");

    editor.undo().unwrap();
    assert_eq!(starts(&editor).0, vec![seconds(0)]);
}

#[test]
fn no_markers_is_refused() {
    let (mut editor, _events) = Editor::new_project("Beats");
    assert!(matches!(
        editor.split_at_markers(None),
        Err(EditorError::NoMarkers)
    ));
}
