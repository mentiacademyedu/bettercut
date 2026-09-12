//! Markers on the timeline.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;

fn editor() -> Editor {
    Editor::new_project("Markers").0
}

fn times(editor: &Editor) -> Vec<TimelineTime> {
    editor.markers().iter().map(|m| m.time).collect()
}

#[test]
fn a_marker_toggles_on_and_off_and_undoes() {
    let mut editor = editor();
    let at = TimelineTime::from_seconds(2);

    assert!(editor.toggle_marker(at).unwrap(), "added");
    assert_eq!(times(&editor), [at]);
    assert!(!editor.toggle_marker(at).unwrap(), "removed");
    assert!(editor.markers().is_empty());

    editor.undo().unwrap();
    assert_eq!(times(&editor), [at], "undo brings it back");
}

/// A mark lands on a frame, where a cut could.
#[test]
fn a_marker_is_snapped_to_the_frame_grid() {
    let mut editor = editor();
    // 30 fps: 32,000 ticks a frame. 40,000 is a quarter of the way into frame 1.
    editor
        .toggle_marker(TimelineTime::from_ticks(40_000))
        .unwrap();
    assert_eq!(times(&editor), [TimelineTime::from_ticks(32_000)]);
}

#[test]
fn many_markers_are_one_undo_step_and_keep_the_old_ones() {
    let mut editor = editor();
    editor.toggle_marker(TimelineTime::from_seconds(1)).unwrap();
    let depth = editor.undo_depth();

    let beats: Vec<_> = (1..=4).map(TimelineTime::from_seconds).collect();
    let added = editor.add_markers(&beats).unwrap();

    assert_eq!(added, 3, "the mark at 1 s was already there");
    assert_eq!(times(&editor), beats);
    assert_eq!(editor.undo_depth(), depth + 1);
    editor.undo().unwrap();
    assert_eq!(times(&editor), [TimelineTime::from_seconds(1)]);
}

#[test]
fn clearing_takes_them_all_and_undoes() {
    let mut editor = editor();
    let beats: Vec<_> = (1..=3).map(TimelineTime::from_seconds).collect();
    editor.add_markers(&beats).unwrap();
    editor.clear_markers().unwrap();
    assert!(editor.markers().is_empty());
    editor.undo().unwrap();
    assert_eq!(times(&editor), beats);
}

#[test]
fn markers_survive_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("markers.vproj");
    {
        let mut editor = editor();
        editor.save_as(&path).unwrap();
        editor
            .add_markers(&[TimelineTime::from_seconds(3), TimelineTime::from_seconds(7)])
            .unwrap();
        std::mem::forget(editor);
    }
    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).unwrap();
    let recovered: Vec<_> = session
        .project
        .active()
        .unwrap()
        .markers
        .iter()
        .map(|m| m.time)
        .collect();
    assert_eq!(
        recovered,
        [TimelineTime::from_seconds(3), TimelineTime::from_seconds(7)]
    );
}

/// A clip dragged near a marker catches on it — the marker outranks a clip
/// edge the same distance away, because it was put there on purpose.
#[test]
fn a_marker_is_a_snap_target_that_beats_a_clip_edge() {
    use bettercut_editor_core::timeline::snap::{SnapKind, collect_targets, nearest};

    let mut editor = editor();
    editor.toggle_marker(TimelineTime::from_seconds(5)).unwrap();
    let sequence = editor.active_sequence().unwrap();
    let targets = collect_targets(sequence, TimelineTime::ZERO, &[]);

    let hit = nearest(
        TimelineTime::from_seconds(5) + TimelineTime::from_ticks(100),
        &targets,
        TimelineTime::from_ticks(1_000),
    )
    .expect("caught on the marker");
    assert_eq!(hit.kind, SnapKind::Marker);
    assert_eq!(hit.time, TimelineTime::from_seconds(5));
}
