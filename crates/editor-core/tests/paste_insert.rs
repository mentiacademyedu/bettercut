//! Paste insert: pasting makes room for itself.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Two four-second videos with sound, back to back: 0–4 and 4–8.
fn edit() -> (Editor, [ClipId; 2]) {
    let (mut editor, _events) = Editor::new_project("Paste");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let first = editor.place_media(media).unwrap();
    editor.place_media(media).unwrap();
    (editor, [first[0], first[1]])
}

type Spans = Vec<(i64, i64)>;

fn lanes(editor: &Editor) -> (Spans, Spans) {
    let sequence = editor.active_sequence().unwrap();
    let secs = |t: TimelineTime| t.ticks() / 960_000;
    (
        sequence.video_tracks[0]
            .clips()
            .iter()
            .map(|c| (secs(c.timeline.start), secs(c.timeline.end)))
            .collect(),
        sequence.audio_tracks[0]
            .clips()
            .iter()
            .map(|c| (secs(c.timeline.start), secs(c.timeline.end)))
            .collect(),
    )
}

/// On a cut: the copy goes in at the playhead, everything after moves right
/// by its length — sound and markers too — and one undo takes it all back.
#[test]
fn a_paste_on_a_cut_pushes_the_rest_along() {
    let (mut editor, first) = edit();
    editor.add_markers(&[seconds(6)]).unwrap();
    let before = lanes(&editor);
    editor.copy_clips(&first);
    editor.set_playhead(seconds(4));

    assert_eq!(editor.paste_insert().unwrap(), 2);
    let (pictures, sound) = lanes(&editor);
    assert_eq!(pictures, vec![(0, 4), (4, 8), (8, 12)]);
    assert_eq!(sound, pictures, "the sound fell out of step");
    assert_eq!(
        editor.markers()[0].time,
        seconds(10),
        "the marker was left behind"
    );
    assert_eq!(editor.undo_label().as_deref(), Some("Paste Insert 2 Clips"));

    editor.undo().unwrap();
    assert_eq!(lanes(&editor), before);
    assert_eq!(editor.markers()[0].time, seconds(6));
}

/// Inside a clip: it is split there, and the copy goes between the halves.
#[test]
fn a_paste_inside_a_clip_goes_between_its_halves() {
    let (mut editor, first) = edit();
    editor.copy_clips(&first);
    editor.set_playhead(seconds(2));

    editor.paste_insert().unwrap();
    let (pictures, sound) = lanes(&editor);
    assert_eq!(pictures, vec![(0, 2), (2, 6), (6, 8), (8, 12)]);
    assert_eq!(sound, pictures);
}

/// Nothing to paste is refused, and moves nothing.
#[test]
fn an_empty_clipboard_moves_nothing() {
    let (mut editor, _) = edit();
    let before = lanes(&editor);
    let depth = editor.undo_depth();
    assert!(matches!(
        editor.paste_insert(),
        Err(EditorError::Timeline(_))
    ));
    assert_eq!(lanes(&editor), before);
    assert_eq!(editor.undo_depth(), depth);
}

/// The room made is the copy's own length, not how far into the edit it came
/// from.
#[test]
fn the_room_made_is_the_clipboards_length() {
    let (mut editor, _) = edit();
    let later = editor.active_sequence().unwrap().video_tracks[0].clips()[1].id;
    editor.copy_clips(&[later]); // 4–8 s: four seconds long, ending at 8
    editor.set_playhead(TimelineTime::ZERO);

    editor.paste_insert().unwrap();
    assert_eq!(lanes(&editor).0, vec![(0, 4), (4, 8), (8, 12)]);
}
