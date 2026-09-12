//! Cutting stretches out of a clip and closing the gaps (§78).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::TimelineRange;

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn range(from: i64, to: i64) -> TimelineRange {
    TimelineRange::new(seconds(from), seconds(to)).unwrap()
}

/// A ten-second video with sound, placed at zero.
fn editor_with_clip() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Silence");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/talk.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0], placed[1])
}

/// Clip spans in whole seconds, picture then sound.
type Spans = (Vec<(i64, i64)>, Vec<(i64, i64)>);

fn spans(editor: &Editor) -> Spans {
    let sequence = editor.active_sequence().unwrap();
    let of = |ranges: Vec<TimelineRange>| {
        ranges
            .into_iter()
            .map(|r| (r.start.ticks() / 960_000, r.end.ticks() / 960_000))
            .collect::<Vec<_>>()
    };
    (
        of(sequence.video_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline)
            .collect()),
        of(sequence.audio_tracks[0]
            .clips()
            .iter()
            .map(|c| c.timeline)
            .collect()),
    )
}

#[test]
fn two_stretches_come_out_and_the_gaps_close() {
    let (mut editor, picture, _) = editor_with_clip();

    let removed = editor
        .remove_ranges(picture, &[range(2, 3), range(5, 6)])
        .unwrap();

    assert_eq!(removed, 2);
    // 0–2, then what was 3–5, then what was 6–10: eight seconds in three parts.
    let (video, audio) = spans(&editor);
    assert_eq!(video, [(0, 2), (2, 4), (4, 8)]);
    assert_eq!(
        audio, video,
        "the sound was cut differently from the picture"
    );
}

#[test]
fn it_is_one_undo_step() {
    let (mut editor, picture, _) = editor_with_clip();
    let depth = editor.undo_depth();

    editor
        .remove_ranges(picture, &[range(2, 3), range(5, 6), range(8, 9)])
        .unwrap();
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    let (video, audio) = spans(&editor);
    assert_eq!(video, [(0, 10)]);
    assert_eq!(audio, [(0, 10)]);
}

/// Each remaining piece of picture keeps a link to the piece of sound beside
/// it, so moving one still brings the other (§12).
#[test]
fn the_pieces_stay_linked_in_pairs() {
    let (mut editor, picture, _) = editor_with_clip();
    editor.remove_ranges(picture, &[range(4, 6)]).unwrap();

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 2);
    for clip in sequence.video_tracks[0].clips() {
        let link = clip.link.expect("a piece of picture lost its link");
        let partner = sequence.audio_tracks[0]
            .clips()
            .iter()
            .find(|c| c.link == Some(link))
            .expect("no sound linked to this piece");
        assert_eq!(partner.timeline, clip.timeline);
    }
}

/// Silence at the very start and the very end has nothing to split against on
/// one side.
#[test]
fn stretches_at_the_edges_come_out() {
    let (mut editor, picture, _) = editor_with_clip();
    editor
        .remove_ranges(picture, &[range(0, 2), range(8, 10)])
        .unwrap();
    let (video, _) = spans(&editor);
    assert_eq!(video, [(0, 6)]);
}

#[test]
fn overlapping_ranges_are_taken_out_once() {
    let (mut editor, picture, _) = editor_with_clip();
    let removed = editor
        .remove_ranges(picture, &[range(2, 5), range(4, 6)])
        .unwrap();
    assert_eq!(removed, 1, "the two overlap and are one cut");
    let (video, _) = spans(&editor);
    assert_eq!(video, [(0, 2), (2, 6)]);
}

/// A range covering the whole clip removes it, rather than leaving a
/// zero-length piece behind.
#[test]
fn a_range_over_the_whole_clip_removes_it() {
    let (mut editor, picture, _) = editor_with_clip();
    editor.remove_ranges(picture, &[range(0, 10)]).unwrap();
    assert_eq!(editor.active_sequence().unwrap().clip_count(), 0);
    editor.undo().unwrap();
    assert_eq!(editor.active_sequence().unwrap().clip_count(), 2);
}

#[test]
fn nothing_to_cut_changes_nothing() {
    let (mut editor, picture, _) = editor_with_clip();
    let before = editor.project().clone();
    let removed = editor.remove_ranges(picture, &[range(20, 22)]).unwrap();
    assert_eq!(removed, 0);
    assert_eq!(
        editor.project(),
        &before,
        "a cut outside the clip changed it"
    );
}
