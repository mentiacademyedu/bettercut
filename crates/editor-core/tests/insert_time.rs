//! Opening a gap across every track (§10's insert).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A ten-second video with sound at zero, and a title from 6 to 9 seconds.
fn editor_with_edit() -> Editor {
    let (mut editor, _events) = Editor::new_project("Insert");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();

    editor.set_playhead(seconds(6));
    let title = editor.add_text("Later").unwrap();
    let track = editor.track_of(title).unwrap();
    editor
        .trim_clip(
            track,
            title,
            bettercut_editor_core::TrimEdge::End,
            seconds(9),
        )
        .unwrap();
    editor
}

fn spans(editor: &Editor) -> Vec<(i64, i64)> {
    let sequence = editor.active_sequence().unwrap();
    sequence.video_tracks[0]
        .clips()
        .iter()
        .map(|c| {
            (
                c.timeline.start.ticks() / 960_000,
                c.timeline.end.ticks() / 960_000,
            )
        })
        .collect()
}

#[test]
fn a_gap_splits_what_it_opens_inside_and_moves_the_rest() {
    let mut editor = editor_with_edit();
    let depth = editor.undo_depth();

    // Two seconds at 4 s: the clip is cut there and its second half moves.
    editor.insert_time(seconds(4), seconds(2)).unwrap();

    assert_eq!(spans(&editor), [(0, 4), (6, 12)]);
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.audio_tracks[0]
            .clips()
            .iter()
            .map(|c| (
                c.timeline.start.ticks() / 960_000,
                c.timeline.end.ticks() / 960_000
            ))
            .collect::<Vec<_>>(),
        [(0, 4), (6, 12)],
        "the sound was not cut and moved with the picture"
    );
    // The title started after the gap, so it moved whole.
    let title = &sequence.text_tracks[0].clips()[0];
    assert_eq!(title.timeline.start, seconds(8));
    assert_eq!(title.timeline.end, seconds(11));

    assert_eq!(editor.undo_depth(), depth + 1, "one undo step");
    editor.undo().unwrap();
    assert_eq!(spans(&editor), [(0, 10)]);
    assert_eq!(
        editor.active_sequence().unwrap().text_tracks[0].clips()[0]
            .timeline
            .start,
        seconds(6)
    );
}

/// The halves either side of the gap stay linked to their own sound (§12).
#[test]
fn the_halves_stay_linked_to_their_sound() {
    let mut editor = editor_with_edit();
    editor.insert_time(seconds(4), seconds(2)).unwrap();

    let sequence = editor.active_sequence().unwrap();
    for clip in sequence.video_tracks[0].clips() {
        let link = clip.link.expect("a half lost its link");
        let partner = sequence.audio_tracks[0]
            .clips()
            .iter()
            .find(|c| c.link == Some(link))
            .expect("no sound linked to this half");
        assert_eq!(partner.timeline, clip.timeline);
    }
}

#[test]
fn markers_after_the_gap_move_with_it() {
    let mut editor = editor_with_edit();
    editor
        .add_markers(&[seconds(2), seconds(4), seconds(8)])
        .unwrap();

    editor.insert_time(seconds(4), seconds(2)).unwrap();

    let times: Vec<_> = editor.markers().iter().map(|m| m.time).collect();
    assert_eq!(
        times,
        [seconds(2), seconds(6), seconds(10)],
        "a marker before the gap moved, or one after it did not"
    );
}

#[test]
fn a_gap_at_the_end_moves_nothing_and_a_zero_gap_does_nothing() {
    let mut editor = editor_with_edit();
    let before = editor.project().clone();

    assert_eq!(editor.insert_time(seconds(30), seconds(2)).unwrap(), 0);
    assert_eq!(
        editor.insert_time(seconds(4), TimelineTime::ZERO).unwrap(),
        0
    );
    assert_eq!(
        editor.project(),
        &before,
        "an empty insert changed the edit"
    );
}

/// A gap opened exactly on a cut does not split anything: both clips already
/// have an edge there.
#[test]
fn a_gap_on_a_cut_just_moves_what_follows() {
    let mut editor = editor_with_edit();
    editor.set_playhead(seconds(5));
    editor.split_at_playhead(&[]).unwrap();
    assert_eq!(spans(&editor), [(0, 5), (5, 10)]);

    editor.insert_time(seconds(5), seconds(3)).unwrap();

    assert_eq!(spans(&editor), [(0, 5), (8, 13)]);
}
