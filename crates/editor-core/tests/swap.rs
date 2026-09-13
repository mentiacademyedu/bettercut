//! Swapping a clip with its neighbour (`bettercut_editor_core::swap`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError, Neighbour};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A four-second and a six-second video with sound, placed back to back.
fn two_takes() -> (Editor, [ClipId; 2], [ClipId; 2]) {
    let (mut editor, _events) = Editor::new_project("Swap");
    let mut place = |name: &str, length: i64| {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(length),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        let placed = editor.place_media(media).unwrap();
        [placed[0], placed[1]]
    };
    let short = place("short", 4);
    let long = place("long", 6);
    (editor, short, long)
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

fn start_of(editor: &Editor, clip: ClipId) -> i64 {
    editor.clip_payload(clip).unwrap().start().ticks() / 960_000
}

/// The later clip takes the earlier one's place, the earlier follows it, the
/// gap between them stays, and the sound comes too. One undo puts them back.
#[test]
fn two_clips_trade_places_and_keep_their_gap() {
    let (mut editor, short, long) = two_takes();
    let track = editor.track_of(long[0]).unwrap();
    editor.move_clip(track, track, long[0], seconds(5)).unwrap(); // a 1 s gap
    let before = lanes(&editor);
    assert_eq!(before.0, vec![(0, 4), (5, 11)]);

    let other = editor
        .swap_with_neighbour(short[0], Neighbour::Next)
        .unwrap();
    assert_eq!(other, long[0]);
    assert_eq!(start_of(&editor, long[0]), 0);
    assert_eq!(start_of(&editor, short[0]), 7);
    let (pictures, sound) = lanes(&editor);
    assert_eq!(
        pictures,
        vec![(0, 6), (7, 11)],
        "the pair should still cover 0–11"
    );
    assert_eq!(sound, pictures, "the sound did not come along");
    assert_eq!(start_of(&editor, long[1]), 0);
    assert_eq!(editor.undo_label().as_deref(), Some("Swap Clips"));

    editor.undo().unwrap();
    assert_eq!(lanes(&editor), before);
}

/// Swapping backwards from the second clip is the same edit.
#[test]
fn previous_swaps_the_same_pair() {
    let (mut editor, short, long) = two_takes();
    assert_eq!(
        editor.neighbour_of(long[0], Neighbour::Previous),
        Some(short[0])
    );
    editor
        .swap_with_neighbour(long[0], Neighbour::Previous)
        .unwrap();
    assert_eq!(lanes(&editor).0, vec![(0, 6), (6, 10)]);
    assert_eq!(start_of(&editor, short[0]), 6);
}

/// Nothing on that side: refused, and no step is left behind.
#[test]
fn an_end_clip_has_no_neighbour_that_way() {
    let (mut editor, short, long) = two_takes();
    let depth = editor.undo_depth();
    assert_eq!(editor.neighbour_of(long[0], Neighbour::Next), None);
    assert_eq!(editor.neighbour_of(short[0], Neighbour::Previous), None);
    assert!(matches!(
        editor.swap_with_neighbour(long[0], Neighbour::Next),
        Err(EditorError::NoNeighbour)
    ));
    assert_eq!(editor.undo_depth(), depth);
}

/// When a picture's sound would land on a sound that is not part of the swap,
/// nothing moves.
#[test]
fn a_swap_with_no_room_for_the_sound_changes_nothing() {
    let (mut editor, short, long) = two_takes();
    // The long clip's sound stays where it is, loose: the short clip's sound
    // would have to land on top of it.
    editor.unlink(long[0]).unwrap();
    let before = lanes(&editor);
    let depth = editor.undo_depth();

    assert!(matches!(
        editor.swap_with_neighbour(short[0], Neighbour::Next),
        Err(EditorError::NoRoomToSwap)
    ));
    assert_eq!(lanes(&editor), before);
    assert_eq!(editor.undo_depth(), depth);
}

/// Titles trade places on their lane like anything else.
#[test]
fn titles_swap_too() {
    let (mut editor, _events) = Editor::new_project("Titles");
    let first = editor.add_text("First").unwrap();
    let second = editor.add_text("Second").unwrap();
    let start = |editor: &Editor, clip| {
        editor
            .active_sequence()
            .unwrap()
            .text_clip(clip)
            .unwrap()
            .timeline
            .start
    };
    let (a, b) = (start(&editor, first), start(&editor, second));
    assert!(a < b);

    editor.swap_with_neighbour(first, Neighbour::Next).unwrap();
    assert_eq!(start(&editor, second), a);
    assert!(start(&editor, first) > a);
}

/// A locked track says it is locked, not that there was no room.
#[test]
fn a_locked_track_is_reported_as_locked() {
    let (mut editor, short, _) = two_takes();
    let sequence = editor.active_sequence().unwrap().id;
    let track = editor.track_of(short[0]).unwrap();
    editor
        .dispatch(bettercut_editor_core::Command::SetTrackFlag {
            sequence,
            track,
            flag: bettercut_editor_core::TrackFlag::Locked,
            value: true,
        })
        .unwrap();
    let before = lanes(&editor);
    match editor.swap_with_neighbour(short[0], Neighbour::Next) {
        Err(EditorError::Timeline(
            bettercut_editor_core::timeline::TimelineError::TrackLocked(_),
        )) => {}
        other => panic!("expected a locked track, got {other:?}"),
    }
    assert_eq!(lanes(&editor), before);
}
