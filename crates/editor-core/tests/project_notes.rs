//! Project notes (`Editor::set_project_notes`): one undo step per edit, no
//! step for no change, and kept in the project file.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_project_format::Project;

#[test]
fn notes_are_one_step_and_no_step_when_unchanged() {
    let (mut editor, _events) = Editor::new_project("Notes");
    assert_eq!(editor.project_notes(), "");
    let depth = editor.undo_depth();

    editor.set_project_notes("fix shot 3").unwrap();
    assert_eq!(editor.project_notes(), "fix shot 3");
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.set_project_notes("fix shot 3").unwrap();
    assert_eq!(editor.undo_depth(), depth + 1, "no change, no step");

    editor
        .set_project_notes("fix shot 3\nask about music")
        .unwrap();
    assert_eq!(editor.undo_depth(), depth + 2);
    editor.undo().unwrap();
    assert_eq!(editor.project_notes(), "fix shot 3");
    editor.undo().unwrap();
    assert_eq!(editor.project_notes(), "");
}

#[test]
fn notes_travel_in_the_project_and_an_older_file_has_none() {
    let (mut editor, _events) = Editor::new_project("Notes");
    editor.set_project_notes("ask about music").unwrap();
    let text = serde_json::to_string(editor.project()).unwrap();
    let back: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(back.notes, "ask about music");

    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json.as_object_mut().unwrap().remove("notes");
    let older: Project = serde_json::from_value(json).unwrap();
    assert_eq!(older.notes, "");
}
