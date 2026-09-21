//! Setting the sequence's audio visualizer (`Editor::set_visualizer`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::timeline::visualizer::Visualizer;

#[test]
fn a_visualizer_is_set_kept_in_range_undone_and_saved() {
    let (mut editor, _events) = Editor::new_project("Bars");
    let wild = Visualizer {
        bars: 500,
        height: 9.0,
        ..Visualizer::new(vec![0.5, 2.0, f32::NAN], TimelineTime::from_seconds(1))
    };
    editor.set_visualizer(Some(wild), false).unwrap();
    let kept = editor
        .active_sequence()
        .unwrap()
        .visualizer
        .clone()
        .unwrap();
    assert_eq!(kept.bars, 64);
    assert_eq!(kept.height, 0.5);
    assert_eq!(kept.levels, vec![0.5, 1.0, 0.0]);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bars.vproj");
    editor.save_as(&path).unwrap();
    let (opened, _) = Editor::open(&path).unwrap();
    assert_eq!(opened.active_sequence().unwrap().visualizer, Some(kept));

    editor.undo().unwrap();
    assert!(editor.active_sequence().unwrap().visualizer.is_none());
}
