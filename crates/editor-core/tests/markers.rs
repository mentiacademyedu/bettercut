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

/// A marker is named, renamed and un-named, each one undo step; naming a
/// place with no marker, or giving the same name again, changes nothing.
#[test]
fn a_marker_can_be_named_and_the_name_undoes() {
    let mut editor = editor();
    let at = TimelineTime::from_seconds(4);
    editor.toggle_marker(at).unwrap();
    let depth = editor.undo_depth();

    assert!(editor.set_marker_label(at, "  drop  ").unwrap());
    assert_eq!(editor.markers()[0].label, "drop", "spaces were kept");
    assert_eq!(editor.undo_depth(), depth + 1);

    assert!(
        !editor.set_marker_label(at, "drop").unwrap(),
        "same name again"
    );
    assert!(
        !editor
            .set_marker_label(TimelineTime::from_seconds(9), "nowhere")
            .unwrap(),
        "no marker there"
    );
    assert_eq!(editor.undo_depth(), depth + 1, "a no-op left an undo step");

    let long = "x".repeat(Editor::MAX_MARKER_LABEL + 20);
    editor.set_marker_label(at, &long).unwrap();
    assert_eq!(
        editor.markers()[0].label.chars().count(),
        Editor::MAX_MARKER_LABEL
    );

    editor.undo().unwrap();
    assert_eq!(editor.markers()[0].label, "drop");
    editor.set_marker_label(at, "").unwrap();
    assert!(editor.markers()[0].label.is_empty());
}

/// One marker goes; the rest stay; undo brings it back with its name.
#[test]
fn one_marker_can_be_removed() {
    let mut editor = editor();
    let [a, b, c] = [2, 5, 8].map(TimelineTime::from_seconds);
    editor.add_markers(&[a, b, c]).unwrap();
    editor.set_marker_label(b, "chorus").unwrap();

    assert!(editor.remove_marker(b).unwrap());
    assert_eq!(times(&editor), [a, c]);
    assert!(!editor.remove_marker(b).unwrap(), "already gone");

    editor.undo().unwrap();
    assert_eq!(times(&editor), [a, b, c]);
    assert_eq!(editor.markers()[1].label, "chorus");
}

/// A marker takes a colour, one undo step; the same colour again, or a place
/// with no marker, changes nothing; a project from before colours loads plain.
#[test]
fn a_marker_can_be_coloured() {
    use bettercut_editor_core::timeline::ColorLabel;

    let mut editor = editor();
    let at = TimelineTime::from_seconds(3);
    editor.toggle_marker(at).unwrap();
    let depth = editor.undo_depth();

    assert!(editor.set_marker_color(at, ColorLabel::Red).unwrap());
    assert_eq!(editor.markers()[0].color, ColorLabel::Red);
    assert!(!editor.set_marker_color(at, ColorLabel::Red).unwrap());
    assert!(
        !editor
            .set_marker_color(TimelineTime::from_seconds(9), ColorLabel::Blue)
            .unwrap()
    );
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(editor.markers()[0].color, ColorLabel::None);

    editor.set_marker_color(at, ColorLabel::Blue).unwrap();
    let mut json = serde_json::to_value(editor.project()).unwrap();
    json["sequences"][0]["markers"][0]
        .as_object_mut()
        .unwrap()
        .remove("color");
    let old: bettercut_editor_core::project_format::Project = serde_json::from_value(json).unwrap();
    assert_eq!(old.sequences[0].markers[0].color, ColorLabel::None);
}
