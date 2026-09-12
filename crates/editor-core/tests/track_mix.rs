//! An audio track's volume and pan (§20a.4's track stage).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::{Editor, EditorError};

fn editor() -> Editor {
    Editor::new_project("Mix").0
}

fn audio_track(editor: &Editor) -> bettercut_editor_core::foundation::TrackId {
    editor.active_sequence().unwrap().audio_tracks[0].id
}

fn mix(editor: &Editor) -> (f32, f32) {
    let track = &editor.active_sequence().unwrap().audio_tracks[0];
    (track.gain, track.pan)
}

#[test]
fn a_new_track_is_at_unity_and_centred() {
    assert_eq!(mix(&editor()), (1.0, 0.0));
}

#[test]
fn a_tracks_mix_is_set_and_undone() {
    let mut editor = editor();
    let track = audio_track(&editor);
    editor.set_track_mix(track, 0.5, -0.4, false).unwrap();
    assert_eq!(mix(&editor), (0.5, -0.4));
    editor.undo().unwrap();
    assert_eq!(mix(&editor), (1.0, 0.0));
}

#[test]
fn a_tracks_mix_is_kept_in_range() {
    let mut editor = editor();
    let track = audio_track(&editor);
    editor.set_track_mix(track, 99.0, -7.0, false).unwrap();
    assert_eq!(
        mix(&editor),
        (bettercut_editor_core::timeline::MAX_TRACK_GAIN, -1.0)
    );
    editor
        .set_track_mix(track, f32::NAN, f32::NAN, false)
        .unwrap();
    assert_eq!(
        mix(&editor),
        (1.0, 0.0),
        "a non-number is neutral, not silence"
    );
}

#[test]
fn a_picture_track_has_no_volume() {
    let mut editor = editor();
    let video = editor.active_sequence().unwrap().video_tracks[0].id;
    let err = editor.set_track_mix(video, 0.5, 0.0, false).unwrap_err();
    assert!(matches!(err, EditorError::ClipKindMismatch), "{err}");
}

#[test]
fn dragging_a_tracks_volume_is_one_undo_step() {
    let mut editor = editor();
    let track = audio_track(&editor);
    let depth = editor.undo_depth();
    for (i, gain) in [0.9, 0.7, 0.5].into_iter().enumerate() {
        editor.set_track_mix(track, gain, 0.0, i > 0).unwrap();
    }
    assert_eq!(editor.undo_depth(), depth + 1);
}

/// A project saved before tracks had a volume loads at unity (serde default).
#[test]
fn an_older_track_loads_at_unity() {
    let mut editor = editor();
    let track = audio_track(&editor);
    editor.set_track_mix(track, 0.25, 0.5, false).unwrap();

    let mut json =
        serde_json::to_value(&editor.active_sequence().unwrap().audio_tracks[0]).unwrap();
    let fields = json.as_object_mut().unwrap();
    fields.remove("gain");
    fields.remove("pan");
    let old: bettercut_editor_core::timeline::AudioTrack = serde_json::from_value(json).unwrap();
    assert_eq!((old.gain, old.pan), (1.0, 0.0));
}

#[test]
fn a_tracks_mix_survives_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mix.vproj");
    {
        let mut editor = editor();
        editor.save_as(&path).unwrap();
        let track = audio_track(&editor);
        editor.set_track_mix(track, 0.3, 0.6, false).unwrap();
        std::mem::forget(editor);
    }
    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).unwrap();
    let track = &session.project.active().unwrap().audio_tracks[0];
    assert_eq!((track.gain, track.pan), (0.3, 0.6));
}
