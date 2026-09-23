//! Writing markers to a file (`Editor::export_markers`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;

#[test]
fn the_markers_are_written_in_the_format_the_extension_asks_for() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Trip");
    editor
        .add_markers(&[TimelineTime::from_seconds(1), TimelineTime::from_seconds(4)])
        .unwrap();
    editor
        .set_marker_label(TimelineTime::from_seconds(4), "Drone")
        .unwrap();

    let csv = dir.path().join("markers.csv");
    assert_eq!(editor.export_markers(&csv).unwrap(), 2);
    let text = std::fs::read_to_string(&csv).unwrap();
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.contains(",Drone"), "{text}");

    let edl = dir.path().join("markers.edl");
    assert_eq!(editor.export_markers(&edl).unwrap(), 2);
    let text = std::fs::read_to_string(&edl).unwrap();
    // The title is the sequence's name, not the project's.
    assert!(text.starts_with("TITLE: Sequence"), "{text}");
    assert!(text.contains("|M:Drone |D:1"), "{text}");
}

/// An imported CSV adds its markers as one step, keeps what was there, and
/// merges onto an existing instant rather than doubling it.
#[test]
fn a_csv_is_imported_as_one_step_onto_the_markers_there() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Import");
    editor
        .add_markers(&[TimelineTime::from_seconds(1)])
        .unwrap();
    let path = dir.path().join("notes.csv");
    std::fs::write(
        &path,
        "name,seconds,colour\nAlready there,1,Red\nNew one,4,Green\nAnother,7.5,\n",
    )
    .unwrap();
    let depth = editor.undo_depth();

    assert_eq!(editor.import_markers(&path).unwrap(), 2);
    let markers = editor.markers().to_vec();
    assert_eq!(markers.len(), 3, "{markers:?}");
    assert_eq!(
        markers[0].label, "Already there",
        "the existing mark took the name"
    );
    assert_eq!(markers[1].time, TimelineTime::from_seconds(4));
    assert_eq!(markers[2].time, TimelineTime::from_millis(7_500));
    assert_eq!(editor.undo_depth(), depth + 1);
    editor.undo().unwrap();
    assert_eq!(editor.markers().len(), 1);

    let empty = dir.path().join("nothing.csv");
    std::fs::write(&empty, "just,words\n").unwrap();
    assert_eq!(editor.import_markers(&empty).unwrap(), 0);
    assert!(
        editor
            .import_markers(&dir.path().join("missing.csv"))
            .is_err()
    );
}

#[test]
fn nothing_to_write_is_refused_and_leaves_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let (editor, _events) = Editor::new_project("Empty");
    let path = dir.path().join("markers.csv");
    assert!(editor.export_markers(&path).is_err());
    assert!(!path.exists());
}
