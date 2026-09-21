//! Marks that cover a passage rather than an instant
//! (`Editor::mark_range`, `Editor::set_marker_span`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{Marker, TimelineRange};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn range(from: i64, to: i64) -> TimelineRange {
    TimelineRange::new(seconds(from), seconds(to)).expect("non-empty")
}

/// A minute of pictures, so there is somewhere to mark.
fn editor() -> Editor {
    let (mut editor, _events) = Editor::new_project("Marks");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/long.mp4",
        MediaTime::from_seconds(60),
    ));
    editor.place_media(media).unwrap();
    editor
}

#[test]
fn a_plain_mark_is_an_instant() {
    let marker = Marker::at(seconds(4));
    assert!(!marker.is_ranged());
    assert_eq!(marker.end(), seconds(4));
    assert!(marker.covers(seconds(4)));
    assert!(!marker.covers(seconds(5)));
}

#[test]
fn a_ranged_mark_covers_its_stretch_and_stops() {
    let marker = Marker::over(seconds(10), seconds(5));
    assert!(marker.is_ranged());
    assert_eq!(marker.end(), seconds(15));
    assert!(marker.covers(seconds(10)));
    assert!(marker.covers(seconds(14)));
    assert!(
        !marker.covers(seconds(15)),
        "the end is where it stops, as every range here does"
    );
    assert!(!marker.covers(seconds(9)));
}

#[test]
fn a_range_can_be_marked_in_one_go() {
    let mut editor = editor();

    assert!(editor.mark_range(range(10, 20), "too long").expect("ok"));

    let markers = editor.markers().to_vec();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].time, seconds(10));
    assert_eq!(markers[0].span, seconds(10));
    assert_eq!(markers[0].label, "too long");
    assert!(markers[0].is_ranged());
}

/// Marking over a mark that is already there gives that one the stretch,
/// rather than leaving two marks at one instant.
#[test]
fn marking_over_an_existing_mark_gives_it_the_stretch() {
    let mut editor = editor();
    editor.toggle_marker(seconds(10)).expect("a mark");

    editor.mark_range(range(10, 20), "the middle").expect("ok");

    let markers = editor.markers().to_vec();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].span, seconds(10));
    assert_eq!(markers[0].label, "the middle");
}

/// A span can be given to a mark that is already there, and taken away again.
#[test]
fn a_span_can_be_set_and_cleared() {
    let mut editor = editor();
    editor.toggle_marker(seconds(5)).expect("a mark");

    assert!(editor.set_marker_span(seconds(5), seconds(3)).expect("ok"));
    assert_eq!(editor.markers()[0].span, seconds(3));
    assert!(
        !editor.set_marker_span(seconds(5), seconds(3)).expect("ok"),
        "setting the same span again is not an edit"
    );

    assert!(
        editor
            .set_marker_span(seconds(5), TimelineTime::ZERO)
            .expect("ok")
    );
    assert!(!editor.markers()[0].is_ranged());
    // And a mark that is not there takes nothing.
    assert!(!editor.set_marker_span(seconds(40), seconds(2)).expect("ok"));
}

/// A stretch the wrong way round is read as the stretch between the two
/// instants, which is plainly what was meant.
#[test]
fn a_backwards_span_is_read_as_a_stretch() {
    let mut editor = editor();
    editor.toggle_marker(seconds(5)).expect("a mark");

    editor
        .set_marker_span(seconds(5), TimelineTime::from_ticks(-seconds(2).ticks()))
        .expect("ok");

    assert_eq!(editor.markers()[0].span, seconds(2));
}

/// Asking what covers an instant: the passage wins over a plain mark inside
/// it, because it is the answer to "what is this bit".
#[test]
fn the_mark_covering_an_instant_is_the_ranged_one() {
    let mut editor = editor();
    editor.mark_range(range(10, 20), "music").expect("ok");
    editor.toggle_marker(seconds(15)).expect("a mark");

    let covering = editor.marker_covering(seconds(15)).expect("a mark");
    assert_eq!(covering.label, "music");
    // Outside it, the plain mark is found on its own instant and nowhere else.
    assert!(editor.marker_covering(seconds(25)).is_none());
    assert_eq!(
        editor.marker_covering(seconds(10)).map(|m| m.label.clone()),
        Some("music".to_owned())
    );
}

/// One undo step, and it puts the mark back as it was.
#[test]
fn marking_a_range_is_one_undo_step() {
    let mut editor = editor();
    editor.mark_range(range(10, 20), "note").expect("ok");

    editor.undo().expect("undo");

    assert!(editor.markers().is_empty());
}

/// Two marks at one instant are one mark, and the note about the passage is
/// the one that survives.
#[test]
fn a_merge_keeps_the_longer_reach() {
    let merged = bettercut_editor_core::timeline::marker::normalized(vec![
        Marker::at(seconds(4)),
        Marker::over(seconds(4), seconds(6)),
    ]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].span, seconds(6));
}

/// An empty stretch is not a passage.
#[test]
fn an_empty_range_marks_nothing() {
    let mut editor = editor();
    let sliver = TimelineRange {
        start: seconds(10),
        end: TimelineTime::from_ticks(seconds(10).ticks() + 1),
    };
    assert!(!editor.mark_range(sliver, "nothing").expect("ok"));
    assert!(editor.markers().is_empty());
}
