//! Earlier versions kept on save, and restoring one (`bettercut_editor_core::versions`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::versions::{
    KEPT_VERSIONS, keep_version, list_versions, versions_folder,
};

/// Saving over the project keeps what was there; the first save has nothing
/// to keep.
#[test]
fn saving_over_a_project_keeps_the_previous_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trip.vproj");
    let (mut editor, _events) = Editor::new_project("Trip");

    editor.save_as(&path).unwrap();
    assert!(
        editor.versions().is_empty(),
        "a first save kept a version of nothing"
    );

    editor.toggle_marker(TimelineTime::from_seconds(2)).unwrap();
    editor.save().unwrap();
    let versions = editor.versions();
    assert_eq!(versions.len(), 1);
    assert!(versions[0].path.starts_with(versions_folder(&path)));

    // The kept version is the project before the marker.
    let (old, _) = Editor::open(&versions[0].path).unwrap();
    assert!(
        old.markers().is_empty(),
        "the version is not the earlier save"
    );
}

/// Restoring brings an earlier project back unsaved, keeps the current one as
/// a version first, and starts the history afresh.
#[test]
fn restoring_keeps_the_current_project_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trip.vproj");
    let (mut editor, _events) = Editor::new_project("Trip");
    editor.save_as(&path).unwrap();
    editor.toggle_marker(TimelineTime::from_seconds(2)).unwrap();
    editor.save().unwrap();
    editor.toggle_marker(TimelineTime::from_seconds(5)).unwrap();

    let earliest = editor.versions().pop().unwrap();
    editor.restore_version(&earliest.path).unwrap();
    assert!(
        editor.markers().is_empty(),
        "the earlier project did not come back"
    );
    assert!(editor.is_dirty(), "a restore counts as saved");
    assert_eq!(
        editor.path(),
        Some(path.as_path()),
        "the project moved to the version's file"
    );
    assert!(editor.undo().is_err(), "the history carried over");

    let kept = editor.versions();
    let before = kept
        .iter()
        .find(|v| v.label.ends_with("before restore"))
        .expect("the current project was not kept");
    let (was, _) = Editor::open(&before.path).unwrap();
    assert_eq!(was.markers().len(), 2, "the unsaved work was not kept");
}

/// Only the newest versions are kept.
#[test]
fn old_versions_are_trimmed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trip.vproj");
    std::fs::write(&path, b"{}").unwrap();
    for second in 0..(KEPT_VERSIONS as u64 + 5) {
        keep_version(&path, 1_000_000 + second).unwrap();
    }
    let versions = list_versions(&path);
    assert_eq!(versions.len(), KEPT_VERSIONS);
    // Newest first, and the oldest five gone.
    assert!(versions[0].path > versions[1].path);
    let names: Vec<String> = versions
        .iter()
        .map(|v| v.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(!names.iter().any(|n| n.contains("13-46-40")), "{names:?}");
}
