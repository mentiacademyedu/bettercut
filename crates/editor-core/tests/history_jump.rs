//! Jumping through the history (the History window's click).
//!
//! A jump is undos or redos in a row, so the claim is that it lands exactly
//! where pressing the keys would have: the same project, the same steps left to
//! redo, and nothing lost on the way.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;

/// Three different edits, so every step has its own name and an ordering
/// mistake cannot hide behind three identical labels: a marker, a title, an
/// adjustment.
fn three_steps() -> Editor {
    let (mut editor, _events) = Editor::new_project("History");
    editor.toggle_marker(TimelineTime::from_seconds(1)).unwrap();
    editor.add_text("Title").unwrap();
    editor.add_adjustment().unwrap();
    let (done, _) = editor.history_steps();
    let mut unique = done.clone();
    unique.dedup();
    assert_eq!(
        unique.len(),
        3,
        "setup: the steps need distinct names: {done:?}"
    );
    editor
}

/// How many of the three edits are in the project: 0 to 3.
fn applied(editor: &Editor) -> usize {
    let sequence = editor.active_sequence().unwrap();
    sequence.markers.len()
        + sequence
            .text_tracks
            .iter()
            .map(|t| t.clips().len())
            .sum::<usize>()
        + sequence
            .adjustment_tracks
            .iter()
            .map(|t| t.clips().len())
            .sum::<usize>()
}

/// The list names every step, oldest first, and nothing is undone yet.
#[test]
fn the_steps_are_listed_oldest_first() {
    let editor = three_steps();
    let (done, undone) = editor.history_steps();
    assert_eq!(done.len(), 3);
    assert!(undone.is_empty());
    assert_eq!(Some(done.last().unwrap().clone()), editor.undo_label());
}

/// Back to the first step: the project is what one step made it, and the two
/// later steps wait to be redone, next one first.
#[test]
fn jumping_back_undoes_down_to_that_step() {
    let mut editor = three_steps();
    let (done, _) = editor.history_steps();

    assert_eq!(editor.jump_in_history(1).unwrap(), 2);

    assert_eq!(applied(&editor), 1);
    let (now_done, undone) = editor.history_steps();
    assert_eq!(now_done, done[..1]);
    assert_eq!(undone, vec![done[1].clone(), done[2].clone()]);
    assert_eq!(editor.redo_label(), Some(done[1].clone()));
}

/// And forward again to the end, through the steps that were undone.
#[test]
fn jumping_forward_redoes_up_to_that_step() {
    let mut editor = three_steps();
    editor.jump_in_history(0).unwrap();
    assert_eq!(applied(&editor), 0);

    assert_eq!(editor.jump_in_history(3).unwrap(), 3);

    assert_eq!(applied(&editor), 3);
    assert!(!editor.can_redo());
}

/// A jump is the same as pressing the keys: the project after jumping back two
/// equals the project after two undos.
#[test]
fn a_jump_matches_pressing_undo_that_many_times() {
    let mut jumped = three_steps();
    let mut pressed = three_steps();

    jumped.jump_in_history(1).unwrap();
    pressed.undo().unwrap();
    pressed.undo().unwrap();

    assert_eq!(applied(&jumped), applied(&pressed));
    assert_eq!(jumped.history_steps(), pressed.history_steps());
}

/// Clicking the current step, or past the end, moves nothing.
#[test]
fn a_jump_to_where_it_is_or_beyond_the_end_moves_nothing() {
    let mut editor = three_steps();
    assert_eq!(editor.jump_in_history(3).unwrap(), 0);
    assert_eq!(editor.jump_in_history(99).unwrap(), 0);
    assert_eq!(applied(&editor), 3);
}

/// A new edit after jumping back replaces the undone steps, as it does after
/// undo — so the list stops offering steps Redo can no longer reach.
#[test]
fn a_new_edit_after_a_jump_drops_the_undone_steps() {
    let mut editor = three_steps();
    editor.jump_in_history(1).unwrap();

    editor.toggle_marker(TimelineTime::from_seconds(9)).unwrap();
    assert_eq!(applied(&editor), 2);

    let (done, undone) = editor.history_steps();
    assert_eq!(done.len(), 2);
    assert!(undone.is_empty());
}
