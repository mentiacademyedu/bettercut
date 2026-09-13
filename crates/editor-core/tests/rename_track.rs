//! Renaming tracks.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::TrackId;
use bettercut_editor_core::{Editor, EditorError};

fn name_of(editor: &Editor, track: TrackId) -> String {
    editor
        .active_sequence()
        .unwrap()
        .track_name(track)
        .unwrap()
        .to_owned()
}

/// Any kind of track is renamed in one undo step, trimmed and held to a
/// length; the same name again is no step, and no name is refused.
#[test]
fn a_track_is_renamed_and_the_name_undoes() {
    let (mut editor, _events) = Editor::new_project("Rename");
    let sequence = editor.active_sequence().unwrap();
    let (video, title) = (sequence.video_tracks[0].id, sequence.text_tracks[0].id);
    let (old_video, old_title) = (name_of(&editor, video), name_of(&editor, title));

    assert!(editor.rename_track(video, "  B-roll ").unwrap());
    assert_eq!(name_of(&editor, video), "B-roll");
    assert!(editor.rename_track(title, "Lower thirds").unwrap());
    assert_eq!(name_of(&editor, title), "Lower thirds");

    let depth = editor.undo_depth();
    assert!(
        !editor.rename_track(video, "B-roll").unwrap(),
        "the same name made a step"
    );
    assert!(matches!(
        editor.rename_track(video, "   "),
        Err(EditorError::EmptyTrackName)
    ));
    assert_eq!(editor.undo_depth(), depth);

    let long = "x".repeat(Editor::MAX_TRACK_NAME + 10);
    editor.rename_track(video, &long).unwrap();
    assert_eq!(
        name_of(&editor, video).chars().count(),
        Editor::MAX_TRACK_NAME
    );

    editor.undo().unwrap();
    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(name_of(&editor, video), old_video);
    assert_eq!(name_of(&editor, title), old_title);
}

#[test]
fn an_unknown_track_is_refused() {
    let (mut editor, _events) = Editor::new_project("Rename");
    assert!(matches!(
        editor.rename_track(TrackId::new(), "Nowhere"),
        Err(EditorError::TrackNotFound(_))
    ));
}

/// A rename is journalled, so a crash keeps it.
#[test]
fn a_rename_survives_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rename.vproj");
    let track = {
        let (mut editor, _events) = Editor::new_project("Rename");
        editor.save_as(&path).unwrap();
        let track = editor.active_sequence().unwrap().audio_tracks[0].id;
        editor.rename_track(track, "Music").unwrap();
        std::mem::forget(editor);
        track
    };
    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).unwrap();
    assert_eq!(
        session.project.active().unwrap().track_name(track),
        Some("Music")
    );
}
