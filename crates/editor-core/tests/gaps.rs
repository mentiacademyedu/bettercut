//! Closing gaps on a track (`bettercut_editor_core::gaps`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Three four-second videos with sound, butted together from zero: pictures
/// on the first video track, sound on the first audio track, each pair linked.
/// Returns the editor and the picture/sound ids, earliest first.
fn three_clips() -> (Editor, Vec<[ClipId; 2]>) {
    let (mut editor, _events) = Editor::new_project("Gaps");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let pairs = (0..3)
        .map(|_| {
            let placed = editor.place_media(media).unwrap();
            [placed[0], placed[1]]
        })
        .collect();
    (editor, pairs)
}

fn picture_track(editor: &Editor) -> TrackId {
    editor.active_sequence().unwrap().video_tracks[0].id
}

type Spans = Vec<(i64, i64)>;

/// Each lane's clips as whole seconds.
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

fn move_to(editor: &mut Editor, clip: ClipId, at: i64) {
    let track = editor.track_of(clip).unwrap();
    editor.move_clip(track, track, clip, seconds(at)).unwrap();
}

/// A gap between two pictures closes, the sound comes with them, and one undo
/// puts it all back.
#[test]
fn closing_a_gap_pulls_the_clip_and_its_sound_left() {
    let (mut editor, pairs) = three_clips();
    move_to(&mut editor, pairs[2][0], 14);
    move_to(&mut editor, pairs[1][0], 6);
    let before = lanes(&editor);
    assert_eq!(before.0, vec![(0, 4), (6, 10), (14, 18)]);

    let track = picture_track(&editor);
    let moved = editor.close_gap(track, seconds(5)).unwrap();
    assert_eq!(moved, 4, "two pictures and their two sounds");
    let after = lanes(&editor);
    assert_eq!(after.0, vec![(0, 4), (4, 8), (12, 16)]);
    assert_eq!(after.1, after.0, "the sound fell out of step");
    assert_eq!(editor.undo_label().as_deref(), Some("Close Gap"));

    editor.undo().unwrap();
    assert_eq!(lanes(&editor), before);
}

/// The space before the first clip is a gap; the space after the last is not.
#[test]
fn the_start_is_a_gap_and_the_end_is_not() {
    let (mut editor, pairs) = three_clips();
    for (pair, at) in pairs.iter().rev().zip([11, 7, 3]) {
        move_to(&mut editor, pair[0], at);
    }
    let track = picture_track(&editor);
    assert_eq!(editor.gaps_on(track).len(), 1);
    assert!(editor.gap_at(track, seconds(30)).is_none());
    assert!(matches!(
        editor.close_gap(track, seconds(30)),
        Err(EditorError::NoGapThere)
    ));

    editor.close_gap(track, seconds(1)).unwrap();
    assert_eq!(lanes(&editor).0, vec![(0, 4), (4, 8), (8, 12)]);
    assert!(editor.gaps_on(track).is_empty());
}

/// Every gap on the track at once, in one undo step.
#[test]
fn close_all_gaps_closes_every_one() {
    let (mut editor, pairs) = three_clips();
    move_to(&mut editor, pairs[2][0], 15);
    move_to(&mut editor, pairs[1][0], 9);
    move_to(&mut editor, pairs[0][0], 2);
    let before = lanes(&editor);
    let track = picture_track(&editor);
    assert_eq!(editor.gaps_on(track).len(), 3);

    assert_eq!(editor.close_all_gaps(track).unwrap(), 3);
    let after = lanes(&editor);
    assert_eq!(after.0, vec![(0, 4), (4, 8), (8, 12)]);
    assert_eq!(after.1, after.0);

    editor.undo().unwrap();
    assert_eq!(lanes(&editor), before, "one undo did not reopen every gap");
}

/// Sound that is no longer tied to its picture stays where it is.
#[test]
fn unlinked_sound_stays_put() {
    let (mut editor, pairs) = three_clips();
    move_to(&mut editor, pairs[2][0], 14);
    editor.unlink(pairs[2][0]).unwrap();

    editor
        .close_gap(picture_track(&editor), seconds(9))
        .unwrap();
    let (pictures, sound) = lanes(&editor);
    assert_eq!(pictures, vec![(0, 4), (4, 8), (8, 12)]);
    assert_eq!(sound, vec![(0, 4), (4, 8), (14, 18)]);
}

/// When the sound tied to a later picture has no room to move — another clip
/// sits where it would land — the gap stays open and nothing changes.
#[test]
fn a_gap_whose_sound_has_no_room_stays_open() {
    let (mut editor, pairs) = three_clips();
    // The middle picture goes, its sound stays: a gap in the pictures with
    // sound under it.
    editor.unlink(pairs[1][0]).unwrap();
    let track = picture_track(&editor);
    editor.remove_clip(track, pairs[1][0]).unwrap();
    let before = lanes(&editor);
    let label = editor.undo_label();

    assert!(matches!(
        editor.close_gap(track, seconds(5)),
        Err(EditorError::GapBlocked)
    ));
    assert!(matches!(
        editor.close_all_gaps(track),
        Err(EditorError::GapBlocked)
    ));
    assert_eq!(lanes(&editor), before);
    assert_eq!(
        editor.undo_label(),
        label,
        "a refused close left an undo step"
    );
}

/// A blocked gap is skipped by "close all", and the others still close.
#[test]
fn close_all_skips_a_blocked_gap() {
    let (mut editor, pairs) = three_clips();
    move_to(&mut editor, pairs[2][0], 13);
    move_to(&mut editor, pairs[1][0], 8);
    move_to(&mut editor, pairs[0][0], 1);
    editor.unlink(pairs[1][0]).unwrap();
    let track = picture_track(&editor);
    editor.remove_clip(track, pairs[1][0]).unwrap();
    // Pictures 1–5 and 13–17; sound 1–5, 8–12 (loose) and 13–17. The gap from
    // 5 to 13 would push the last sound into the loose one; the one before 1
    // is free to close.

    assert_eq!(editor.gaps_on(track).len(), 2);
    assert_eq!(editor.close_all_gaps(track).unwrap(), 1);
    let (pictures, sound) = lanes(&editor);
    assert_eq!(pictures, vec![(0, 4), (12, 16)]);
    assert_eq!(sound, vec![(0, 4), (8, 12), (12, 16)]);
}

/// "Everything after here": clips starting at or after an instant, on one
/// track or all, leaving out locked tracks.
#[test]
fn clips_starting_from_an_instant() {
    let (mut editor, pairs) = three_clips(); // 0–4, 4–8, 8–12, with sound
    let sequence = editor.active_sequence().unwrap();
    let (pictures, sounds) = (sequence.video_tracks[0].id, sequence.audio_tracks[0].id);

    let all = editor.clips_starting_from(seconds(4), None);
    assert_eq!(all.len(), 4, "two pictures and their sounds");
    assert_eq!(
        editor.clips_starting_from(seconds(4), Some(pictures)),
        vec![pairs[1][0], pairs[2][0]],
        "in time order"
    );
    assert_eq!(
        editor.clips_starting_from(seconds(5), None),
        vec![pairs[2][0], pairs[2][1]],
        "a clip the instant falls inside is not after it"
    );
    assert_eq!(
        editor.clips_starting_from(TimelineTime::ZERO, None).len(),
        6
    );

    let sequence_id = editor.active_sequence().unwrap().id;
    editor
        .dispatch(bettercut_editor_core::Command::SetTrackFlag {
            sequence: sequence_id,
            track: sounds,
            flag: bettercut_editor_core::TrackFlag::Locked,
            value: true,
        })
        .unwrap();
    assert_eq!(
        editor.clips_starting_from(seconds(4), None),
        vec![pairs[1][0], pairs[2][0]],
        "a locked track's clips were selected"
    );
}
