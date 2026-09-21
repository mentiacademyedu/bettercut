//! Roll edit (`Editor::roll_edit`): the cut between two touching clips moves,
//! one clip growing as the other shrinks.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TICKS_PER_SECOND, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError, TrimEdge};

fn secs(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn seconds_of(t: i64) -> f64 {
    t as f64 / TICKS_PER_SECOND as f64
}

/// Timeline and source seconds of a picture or sound clip.
fn spans(editor: &Editor, clip: ClipId) -> ((f64, f64), (f64, f64)) {
    let (timeline, source) = editor
        .video_clip(clip)
        .map(|c| (c.timeline, c.source))
        .or_else(|| editor.audio_clip(clip).map(|c| (c.timeline, c.source)))
        .unwrap();
    (
        (
            seconds_of(timeline.start.ticks()),
            seconds_of(timeline.end.ticks()),
        ),
        (
            seconds_of(source.start.ticks()),
            seconds_of(source.end.ticks()),
        ),
    )
}

fn sound_of(editor: &Editor, clip: ClipId) -> ClipId {
    editor
        .linked_with(clip)
        .into_iter()
        .find(|c| *c != clip)
        .unwrap()
}

/// Two 10-second shots with sound, cut together at 6 s: the first plays its
/// file's 0–6 s, the second its 2–10 s (so it runs 6–14 s on the timeline).
fn two_shots() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Roll");
    let place = |editor: &mut Editor, name: &str| {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(10),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        editor.place_media(media).unwrap()[0]
    };
    let first = place(&mut editor, "first");
    let track = editor.track_of(first).unwrap();
    editor
        .trim_clip(track, first, TrimEdge::End, secs(6))
        .unwrap();
    let second = place(&mut editor, "second");
    assert_eq!(spans(&editor, second).0, (6.0, 16.0));
    editor
        .trim_clip(track, second, TrimEdge::Start, secs(8))
        .unwrap();
    editor.move_clip(track, track, second, secs(6)).unwrap();
    assert_eq!(spans(&editor, second), ((6.0, 14.0), (2.0, 10.0)));
    (editor, first, second)
}

#[test]
fn rolling_the_cut_later_grows_the_first_and_shrinks_the_second() {
    let (mut editor, first, second) = two_shots();
    let depth = editor.undo_depth();

    editor.roll_edit(first, secs(8)).unwrap();

    assert_eq!(spans(&editor, first), ((0.0, 8.0), (0.0, 8.0)));
    assert_eq!(spans(&editor, second), ((8.0, 14.0), (4.0, 10.0)));
    // Their sound rolls with them.
    assert_eq!(spans(&editor, sound_of(&editor, first)).0, (0.0, 8.0));
    assert_eq!(spans(&editor, sound_of(&editor, second)).0, (8.0, 14.0));
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(spans(&editor, first), ((0.0, 6.0), (0.0, 6.0)));
    assert_eq!(spans(&editor, second), ((6.0, 14.0), (2.0, 10.0)));
}

#[test]
fn rolling_the_cut_earlier_works_the_other_way() {
    let (mut editor, first, second) = two_shots();
    editor.roll_edit(first, secs(5)).unwrap();
    assert_eq!(spans(&editor, first), ((0.0, 5.0), (0.0, 5.0)));
    assert_eq!(spans(&editor, second), ((5.0, 14.0), (1.0, 10.0)));
}

#[test]
fn a_roll_stops_where_a_file_runs_out() {
    let (mut editor, first, second) = two_shots();
    // The second shot has 2 s of file before its in-point; the first has 4 s
    // after its out-point.
    assert_eq!(editor.roll_room(first), Some((secs(4), secs(10))));

    editor.roll_edit(first, secs(1)).unwrap();
    assert_eq!(spans(&editor, second), ((4.0, 14.0), (0.0, 10.0)));

    editor.roll_edit(first, secs(13)).unwrap();
    assert_eq!(spans(&editor, first), ((0.0, 10.0), (0.0, 10.0)));
    assert_eq!(spans(&editor, second), ((10.0, 14.0), (6.0, 10.0)));

    // Already at the end of the first file: nothing to do, no undo step.
    let depth = editor.undo_depth();
    editor.roll_edit(first, secs(12)).unwrap();
    assert_eq!(editor.undo_depth(), depth);
}

#[test]
fn a_clip_with_nothing_touching_it_has_no_cut_to_roll() {
    let (mut editor, _, second) = two_shots();
    assert_eq!(editor.roll_partner(second, TrimEdge::End), None);
    assert!(matches!(
        editor.roll_edit(second, secs(12)),
        Err(EditorError::NoNeighbour)
    ));
}
