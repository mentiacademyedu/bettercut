//! Save a Copy: the project written somewhere else, the editor left where it
//! was.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::{Editor, EditorError};

/// The copy holds the project as it stands, unsaved change included; the
/// editor keeps its own file, stays unsaved, and keeps its recovery data.
#[test]
fn a_copy_leaves_the_editor_on_its_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("cut.vproj");
    let (mut editor, _events) = Editor::new_project("Cut");
    editor.save_as(&original).unwrap();

    editor.toggle_marker(TimelineTime::from_seconds(2)).unwrap();
    assert!(editor.is_dirty());
    let journal = editor.recovery_paths().clone();

    let written = editor
        .save_copy(dir.path().join("cut before the music"))
        .unwrap();
    assert_eq!(
        written.extension().unwrap(),
        "vproj",
        "the extension was not added"
    );

    assert_eq!(
        editor.path(),
        Some(original.as_path()),
        "the editor moved to the copy"
    );
    assert!(editor.is_dirty(), "a copy marked the original as saved");
    assert_eq!(
        editor.recovery_paths().dir,
        journal.dir,
        "the recovery data moved"
    );
    // And it still holds the unsaved marker: a crash now loses nothing.
    let recovered = bettercut_editor_core::recover(journal).unwrap();
    assert_eq!(
        recovered.project.active().unwrap().markers.len(),
        1,
        "the copy threw away the recovery data"
    );

    let copy = bettercut_editor_core::project_format::load(&written).unwrap();
    assert_eq!(
        copy.active().unwrap().markers.len(),
        1,
        "the unsaved marker is not in the copy"
    );
    let on_disk = bettercut_editor_core::project_format::load(&original).unwrap();
    assert!(
        on_disk.active().unwrap().markers.is_empty(),
        "the original file was written too"
    );
}

/// A copy over the project's own file is refused, however the path is spelt.
#[test]
fn a_copy_over_the_original_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("cut.vproj");
    let (mut editor, _events) = Editor::new_project("Cut");
    editor.save_as(&original).unwrap();
    editor.toggle_marker(TimelineTime::from_seconds(2)).unwrap();

    assert!(matches!(
        editor.save_copy(&original),
        Err(EditorError::CopyOverOriginal)
    ));
    // The same file, without its extension and by way of a folder and back.
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    assert!(matches!(
        editor.save_copy(dir.path().join("sub").join("..").join("cut")),
        Err(EditorError::CopyOverOriginal)
    ));
    let on_disk = bettercut_editor_core::project_format::load(&original).unwrap();
    assert!(on_disk.active().unwrap().markers.is_empty());
}

/// A project never saved can still be copied; it stays unsaved.
#[test]
fn an_unsaved_project_can_be_copied() {
    let dir = tempfile::tempdir().unwrap();
    let (editor, _events) = Editor::new_project("Draft");
    let written = editor.save_copy(dir.path().join("draft.vproj")).unwrap();
    assert!(written.exists());
    assert_eq!(editor.path(), None);
}
