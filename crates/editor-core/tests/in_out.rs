//! In and out marks (`Editor::set_mark_in`, `set_mark_out`, `clear_marks`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::timeline::TimelineRange;

fn marks(editor: &Editor) -> (Option<TimelineTime>, Option<TimelineTime>) {
    let sequence = editor.active_sequence().unwrap();
    (sequence.mark_in, sequence.mark_out)
}

/// Both marks make a range; each is its own undo step.
#[test]
fn in_and_out_make_a_range() {
    let (mut editor, _events) = Editor::new_project("Marks");
    editor.set_mark_in(TimelineTime::from_seconds(2)).unwrap();
    assert_eq!(
        editor.active_sequence().unwrap().marked_range(),
        None,
        "one mark is not a range"
    );
    editor.set_mark_out(TimelineTime::from_seconds(5)).unwrap();

    assert_eq!(
        editor.active_sequence().unwrap().marked_range(),
        Some(
            TimelineRange::new(TimelineTime::from_seconds(2), TimelineTime::from_seconds(5))
                .unwrap()
        )
    );
    editor.undo().unwrap();
    assert_eq!(marks(&editor), (Some(TimelineTime::from_seconds(2)), None));
}

/// A mark placed past the other one clears it, rather than leaving a range
/// that runs backwards.
#[test]
fn a_mark_past_the_other_clears_it() {
    let (mut editor, _events) = Editor::new_project("Marks");
    editor.set_mark_in(TimelineTime::from_seconds(2)).unwrap();
    editor.set_mark_out(TimelineTime::from_seconds(5)).unwrap();

    editor.set_mark_in(TimelineTime::from_seconds(7)).unwrap();
    assert_eq!(marks(&editor), (Some(TimelineTime::from_seconds(7)), None));

    editor.set_mark_out(TimelineTime::from_seconds(3)).unwrap();
    assert_eq!(marks(&editor), (None, Some(TimelineTime::from_seconds(3))));
}

/// Marks land on the frame grid, as every edit instant does (§76).
#[test]
fn marks_snap_to_frames() {
    let (mut editor, _events) = Editor::new_project("Marks");
    let frame = editor.active_sequence().unwrap().ticks_per_frame();
    editor
        .set_mark_in(TimelineTime::from_ticks(frame * 10 + frame / 3))
        .unwrap();
    assert_eq!(marks(&editor).0, Some(TimelineTime::from_ticks(frame * 10)));
}

/// Clearing removes both in one step; clearing nothing is no step.
#[test]
fn clearing_marks() {
    let (mut editor, _events) = Editor::new_project("Marks");
    let depth = editor.undo_depth();
    editor.clear_marks().unwrap();
    assert_eq!(editor.undo_depth(), depth, "clearing nothing left a step");

    editor.set_mark_in(TimelineTime::from_seconds(1)).unwrap();
    editor.set_mark_out(TimelineTime::from_seconds(4)).unwrap();
    editor.clear_marks().unwrap();
    assert_eq!(marks(&editor), (None, None));
    editor.undo().unwrap();
    assert!(editor.active_sequence().unwrap().marked_range().is_some());
}

/// Marks are part of the project: saved and loaded.
#[test]
fn marks_survive_a_save_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Marks");
    editor.set_mark_in(TimelineTime::from_seconds(1)).unwrap();
    editor.set_mark_out(TimelineTime::from_seconds(3)).unwrap();
    let file = dir.path().join("marks.vproj");
    editor.save_as(&file).unwrap();

    let (opened, _events) = Editor::open(&file).unwrap();
    assert_eq!(marks(&opened), marks(&editor));
}
