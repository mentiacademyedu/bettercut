//! Picking a cut and moving it a frame at a time
//! (`bettercut_editor_core::trim_window`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, ticks_per_frame};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Two eight-second shots butted together, each taken from the middle of a
/// long file so the cut has room to move either way.
fn two_shots() -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Trim");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/scene.mp4",
        MediaTime::from_seconds(60),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let clips = (0..2)
        .map(|_| {
            editor
                .place_media_range(
                    media,
                    Some((MediaTime::from_seconds(20), MediaTime::from_seconds(28))),
                )
                .unwrap()[0]
        })
        .collect();
    (editor, clips)
}

fn frame(editor: &Editor) -> i64 {
    ticks_per_frame(editor.active_sequence().unwrap().frame_rate).unwrap()
}

fn span(editor: &Editor, clip: ClipId) -> (i64, i64) {
    let range = editor
        .active_sequence()
        .unwrap()
        .clip_span(clip)
        .unwrap()
        .timeline;
    (range.start.ticks(), range.end.ticks())
}

#[test]
fn the_cut_nearest_the_playhead_is_the_one_found() {
    let (editor, clips) = two_shots();
    let cut = editor.cut_near(seconds(7)).expect("a cut");

    assert_eq!(cut.outgoing, clips[0]);
    assert_eq!(cut.incoming, clips[1]);
    assert_eq!(cut.at, seconds(8));
    assert!(cut.can_move(), "both shots have footage either side");
    assert!(cut.earliest < cut.at && cut.at < cut.latest);
}

/// Asked from the far end of the cut, it is still the same cut: there is only
/// one.
#[test]
fn the_same_cut_is_found_from_either_side() {
    let (editor, _clips) = two_shots();
    assert_eq!(
        editor.cut_near(seconds(1)).map(|cut| cut.at),
        Some(seconds(8))
    );
    assert_eq!(
        editor.cut_near(seconds(15)).map(|cut| cut.at),
        Some(seconds(8))
    );
}

#[test]
fn a_gap_is_not_a_cut() {
    let (mut editor, clips) = two_shots();
    let track = editor.track_of(clips[1]).unwrap();
    editor
        .move_clip(track, track, clips[1], seconds(10))
        .expect("moved");

    assert!(editor.cut_near(seconds(8)).is_none());
    assert!(editor.cut_after(clips[0]).is_none());
}

/// A frame later: the first shot grows by one and the second gives one up,
/// and nothing after the pair moves.
#[test]
fn the_cut_moves_a_frame_at_a_time() {
    let (mut editor, clips) = two_shots();
    let frame = frame(&editor);
    let before = span(&editor, clips[1]).1;

    let at = editor.trim_cut_by(clips[0], 1).expect("trimmed");

    assert_eq!(at.ticks(), seconds(8).ticks() + frame);
    assert_eq!(span(&editor, clips[0]).1, seconds(8).ticks() + frame);
    assert_eq!(span(&editor, clips[1]).0, seconds(8).ticks() + frame);
    assert_eq!(
        span(&editor, clips[1]).1,
        before,
        "the end of the pair moved"
    );
}

#[test]
fn it_moves_the_other_way_too() {
    let (mut editor, clips) = two_shots();
    let frame = frame(&editor);

    editor.trim_cut_by(clips[0], -5).expect("trimmed");

    assert_eq!(span(&editor, clips[0]).1, seconds(8).ticks() - 5 * frame);
    assert_eq!(editor.undo_label().as_deref(), Some("Roll Edit"));
}

/// A key held down should run out of room quietly rather than refusing: the
/// cut stops where the footage stops.
#[test]
fn trimming_past_the_footage_stops_at_the_limit() {
    let (mut editor, clips) = two_shots();
    let room = editor.roll_room(clips[0]).expect("room");

    let at = editor.trim_cut_by(clips[0], 10_000).expect("trimmed");

    assert_eq!(at, room.1, "it should have stopped at the far limit");
    assert_eq!(span(&editor, clips[0]).1, room.1.ticks());
    // And again from there is nothing at all.
    let steps = editor.undo_label();
    assert_eq!(editor.trim_cut_by(clips[0], 10_000).expect("ok"), room.1);
    assert_eq!(editor.undo_label(), steps, "an empty trim made a step");
}

#[test]
fn a_clip_with_nothing_after_it_has_no_cut_to_trim() {
    let (mut editor, clips) = two_shots();
    let err = editor.trim_cut_by(clips[1], 1).unwrap_err();
    assert!(matches!(err, EditorError::NoNeighbour), "{err}");
}

/// The sound goes with the picture, as it does for any roll (§12).
#[test]
fn the_sound_moves_with_the_cut() {
    let (mut editor, clips) = two_shots();
    let frame = frame(&editor);
    let sound = editor.sound_of(clips[0]).expect("linked sound");

    editor.trim_cut_by(clips[0], 2).expect("trimmed");

    assert_eq!(span(&editor, sound).1, seconds(8).ticks() + 2 * frame);
}
