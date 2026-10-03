//! Crash recovery after an undo or a redo. Neither is a command, so neither
//! reached the journal: replay after a crash brought back every edit the
//! person had undone, and lost the ones they redid.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::{Editor, recover};

/// The project as recovery would bring it back, as JSON.
fn recovered(editor: &Editor) -> serde_json::Value {
    let session = recover(editor.recovery_paths().clone()).expect("recovery data exists");
    assert_eq!(session.failed, 0, "every journalled edit replays");
    serde_json::to_value(&session.project).unwrap()
}

#[test]
fn recovery_after_undo_and_redo_is_the_project_as_it_is() {
    let (mut editor, _events) = Editor::new_project("Undo and crash");
    editor.add_text("Kept").unwrap();
    editor.add_text("Undone").unwrap();

    editor.undo().unwrap();
    let now = serde_json::to_value(editor.project()).unwrap();
    assert_eq!(recovered(&editor), now, "the undone title stays undone");
    assert!(!now.to_string().contains("Undone"));

    editor.redo().unwrap();
    let now = serde_json::to_value(editor.project()).unwrap();
    assert_eq!(recovered(&editor), now, "the redone title comes back");

    // And edits after an undo journal on top of it as usual.
    editor.undo().unwrap();
    editor.add_text("After").unwrap();
    let now = serde_json::to_value(editor.project()).unwrap();
    assert_eq!(recovered(&editor), now);

    editor.shutdown();
}
