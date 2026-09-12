//! The whole video's volume (§20a.4's master gain).
//!
//! Project data, set through the same whole-video command as the picture
//! adjustments, because §20a.4 puts master gain inside the mix whose order must
//! match between preview and export (§46). The export test in the export crate
//! checks it reaches the file; these check the edit.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::{ClipProperty, Editor};

fn editor() -> Editor {
    let (editor, _rx) = Editor::new_project("Volume");
    editor
}

fn volume(editor: &Editor) -> f32 {
    editor.active_sequence().unwrap().master_volume
}

#[test]
fn a_new_project_is_at_full_volume() {
    assert_eq!(volume(&editor()), 1.0);
}

#[test]
fn the_whole_video_volume_is_set_and_undone() {
    let mut editor = editor();
    editor
        .set_sequence_value(ClipProperty::Gain(0.5), false)
        .unwrap();
    assert_eq!(volume(&editor), 0.5);

    editor.undo().unwrap();
    assert_eq!(volume(&editor), 1.0);
}

/// A drag across the slider is one undo step (§11).
#[test]
fn dragging_the_volume_is_one_undo_step() {
    let mut editor = editor();
    for step in 1..=10 {
        editor
            .set_sequence_value(ClipProperty::Gain(1.0 - step as f32 * 0.05), step > 1)
            .unwrap();
    }
    editor.undo().unwrap();
    assert_eq!(
        volume(&editor),
        1.0,
        "one undo did not take the whole drag back"
    );
}

/// Clamped in the model, for §38.2's reason: the journal replays commands, and
/// a limit only the slider knew about would come back unapplied.
#[test]
fn the_volume_is_clamped() {
    let mut editor = editor();
    editor
        .set_sequence_value(ClipProperty::Gain(50.0), false)
        .unwrap();
    assert_eq!(
        volume(&editor),
        bettercut_editor_core::timeline::sequence::MAX_MASTER_VOLUME
    );
    editor
        .set_sequence_value(ClipProperty::Gain(-1.0), false)
        .unwrap();
    assert_eq!(volume(&editor), 0.0);
}

/// The reset button asks whether the value is already the default. Gain is not
/// an animated parameter, and the generic check answered "yes" for every value
/// — so every volume reset button was greyed out whatever the volume was.
#[test]
fn a_changed_volume_is_not_reported_as_the_default() {
    assert!(ClipProperty::Gain(1.0).is_default());
    assert!(
        !ClipProperty::Gain(0.5).is_default(),
        "a halved volume claims to be the default, so its reset button is disabled"
    );
}

#[test]
fn resetting_the_volume_returns_it_to_full() {
    let mut editor = editor();
    editor
        .set_sequence_value(ClipProperty::Gain(0.3), false)
        .unwrap();
    editor
        .reset_sequence_parameter(ClipProperty::Gain(0.3))
        .unwrap();
    assert_eq!(volume(&editor), 1.0);
}

/// §38.2: survives a save, and a project from before the volume existed loads
/// at full volume rather than silent.
#[test]
fn the_volume_survives_a_save_and_old_projects_load_at_full() {
    let mut editor = editor();
    editor
        .set_sequence_value(ClipProperty::Gain(0.4), false)
        .unwrap();
    let json = serde_json::to_string(editor.project()).unwrap();
    let loaded: bettercut_editor_core::project_format::Project =
        serde_json::from_str(&json).unwrap();
    assert_eq!(loaded.sequences[0].master_volume, 0.4);

    let mut old: serde_json::Value = serde_json::to_value(editor.project()).unwrap();
    for sequence in old["sequences"].as_array_mut().unwrap() {
        sequence.as_object_mut().unwrap().remove("master_volume");
    }
    let loaded: bettercut_editor_core::project_format::Project =
        serde_json::from_value(old).unwrap();
    assert_eq!(
        loaded.sequences[0].master_volume, 1.0,
        "an old project loaded silent"
    );
}
