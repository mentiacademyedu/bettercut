//! Taking a stretch out of every lane and closing the gap
//! (`Editor::remove_time`) — the mirror of `insert_time`, and what cutting a
//! line out of the caption list does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::TimelineRange;

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn range(from: i64, to: i64) -> TimelineRange {
    TimelineRange::new(seconds(from), seconds(to)).expect("non-empty")
}

/// Three four-second shots with their own sound, butted together from zero.
fn three_shots() -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Ripple");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let clips = (0..3)
        .map(|_| editor.place_media(media).unwrap()[0])
        .collect();
    (editor, clips)
}

/// A lane's clips as whole seconds.
type Spans = Vec<(i64, i64)>;

/// The picture lane and the sound lane, to compare against each other.
fn lanes(editor: &Editor) -> (Spans, Spans) {
    let sequence = editor.active_sequence().unwrap();
    let secs = |t: TimelineTime| t.ticks() / 960_000;
    let spans = |track: &[(i64, i64)]| track.to_vec();
    let video = sequence.video_tracks[0]
        .clips()
        .iter()
        .map(|c| (secs(c.timeline.start), secs(c.timeline.end)))
        .collect::<Vec<_>>();
    let audio = sequence.audio_tracks[0]
        .clips()
        .iter()
        .map(|c| (secs(c.timeline.start), secs(c.timeline.end)))
        .collect::<Vec<_>>();
    (spans(&video), spans(&audio))
}

/// A whole clip out of the middle: the rest closes up, on every lane.
#[test]
fn a_stretch_covering_a_clip_takes_it_out() {
    let (mut editor, _clips) = three_shots();

    let removed = editor.remove_time(range(4, 8)).expect("removed");

    assert_eq!(removed, 2, "the picture and its sound");
    let (video, audio) = lanes(&editor);
    assert_eq!(video, vec![(0, 4), (4, 8)]);
    assert_eq!(audio, video, "the sound fell out of step");
    assert_eq!(editor.undo_label().as_deref(), Some("Ripple Delete Range"));
}

/// Part of a clip: it is cut at both ends, the middle goes, and what is left
/// closes up.
#[test]
fn a_stretch_inside_one_clip_cuts_it_at_both_ends() {
    let (mut editor, _clips) = three_shots();

    editor.remove_time(range(5, 7)).expect("removed");

    // Twelve seconds less two: the piece after the cut closes up behind it.
    let (video, audio) = lanes(&editor);
    assert_eq!(video, vec![(0, 4), (4, 5), (5, 6), (6, 10)]);
    assert_eq!(audio, video);
}

/// One undo puts the whole thing back — clips, splits and all.
#[test]
fn one_undo_puts_the_stretch_back() {
    let (mut editor, _clips) = three_shots();
    let before = lanes(&editor);

    editor.remove_time(range(3, 9)).expect("removed");
    assert_ne!(lanes(&editor), before);

    editor.undo().expect("undo");
    assert_eq!(lanes(&editor), before);
}

/// Markers inside the stretch mark something that is no longer there;
/// markers after it move with the picture they mark.
#[test]
fn markers_inside_go_and_markers_after_move() {
    let (mut editor, _clips) = three_shots();
    for at in [2, 5, 10] {
        editor.set_playhead(seconds(at));
        editor.toggle_marker(seconds(at)).expect("marker");
    }
    assert_eq!(editor.markers().len(), 3);

    editor.remove_time(range(4, 8)).expect("removed");

    let at: Vec<i64> = editor
        .markers()
        .iter()
        .map(|marker| marker.time.ticks() / 960_000)
        .collect();
    assert_eq!(at, vec![2, 6], "the one at five went, the one at ten moved");
}

/// A stretch shorter than a frame snaps to nothing, and nothing is what it
/// does — rather than leaving an undo step that undoes no change.
#[test]
fn a_stretch_shorter_than_a_frame_is_nothing_to_do() {
    let (mut editor, _clips) = three_shots();
    let before = lanes(&editor);
    let steps = editor.undo_label();
    let sliver = TimelineRange {
        start: seconds(4),
        end: TimelineTime::from_ticks(seconds(4).ticks() + 1),
    };

    assert_eq!(editor.remove_time(sliver).expect("ok"), 0);

    assert_eq!(lanes(&editor), before);
    assert_eq!(editor.undo_label(), steps, "an empty edit made a step");
}

/// A stretch past the end of the edit takes what it covers and nothing else.
#[test]
fn a_stretch_past_the_end_is_held_to_what_is_there() {
    let (mut editor, _clips) = three_shots();

    editor.remove_time(range(10, 30)).expect("removed");

    let (video, _audio) = lanes(&editor);
    assert_eq!(video, vec![(0, 4), (4, 8), (8, 10)]);
}

/// Titles ride along too: a caption after the stretch keeps its place against
/// the picture it belongs to.
#[test]
fn a_title_after_the_stretch_moves_with_the_picture() {
    let (mut editor, _clips) = three_shots();
    editor.set_playhead(seconds(9));
    let title = editor.add_text("after").expect("a title");
    let start_of = |editor: &Editor, clip: ClipId| {
        editor
            .active_sequence()
            .unwrap()
            .clip_span(clip)
            .unwrap()
            .timeline
            .start
    };
    assert_eq!(start_of(&editor, title), seconds(9));

    editor.remove_time(range(4, 8)).expect("removed");

    assert_eq!(start_of(&editor, title), seconds(5));
}
