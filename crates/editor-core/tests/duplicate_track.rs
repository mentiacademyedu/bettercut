//! Duplicating a track with its clips.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Command, Editor, EditorError, TrackFlag};

/// Two four-second videos with sound, back to back.
fn edit() -> Editor {
    let (mut editor, _events) = Editor::new_project("Duplicate");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    editor.place_media(media).unwrap();
    editor
}

/// The copy sits straight after the original, holds the same clips at the
/// same places as new clips, tied to nothing; one undo takes it away and redo
/// brings back the very same track.
#[test]
fn a_picture_track_is_copied_after_itself_with_new_unlinked_clips() {
    let mut editor = edit();
    let original = editor.active_sequence().unwrap().video_tracks[0].clone();

    let copy_id = editor.duplicate_track(original.id).unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks.len(), 2);
    assert_eq!(
        sequence.video_tracks[0].id, original.id,
        "the original moved"
    );
    let copy = &sequence.video_tracks[1];
    assert_eq!(copy.id, copy_id);
    assert_eq!(copy.name, format!("{} copy", original.name));
    assert_eq!(copy.clips().len(), 2);
    for (made, source) in copy.clips().iter().zip(original.clips()) {
        assert_eq!(made.timeline, source.timeline);
        assert_eq!(made.source, source.source);
        assert_ne!(made.id, source.id, "a copy shares its original's id");
        assert!(
            made.link.is_none(),
            "a copy is tied to the original's sound"
        );
    }
    assert_eq!(
        sequence.video_tracks[0].clips()[0].link,
        original.clips()[0].link,
        "the original lost its link"
    );
    assert_eq!(editor.undo_label().as_deref(), Some("Duplicate Track"));

    editor.undo().unwrap();
    assert_eq!(editor.active_sequence().unwrap().video_tracks.len(), 1);
    editor.redo().unwrap();
    let again = &editor.active_sequence().unwrap().video_tracks[1];
    assert_eq!(again.id, copy_id);
    assert_eq!(again.clips().len(), 2);
}

/// A title lane copies its titles, timers and all.
#[test]
fn a_title_track_is_copied_too() {
    let (mut editor, _events) = Editor::new_project("Titles");
    editor.add_text("First").unwrap();
    editor
        .add_counter(bettercut_editor_core::timeline::CountDirection::Down)
        .unwrap();
    let lane = editor.active_sequence().unwrap().text_tracks[0].id;

    let copy = editor.duplicate_track(lane).unwrap();
    let sequence = editor.active_sequence().unwrap();
    let copied = sequence.text_track(copy).unwrap();
    assert_eq!(copied.clips().len(), 2);
    assert_eq!(copied.clips()[0].text, "First");
    assert!(
        copied.clips()[1].counter.is_some(),
        "the timer was not copied"
    );
}

/// A locked track's copy is unlocked; a hidden one's stays hidden.
#[test]
fn the_copy_is_unlocked_and_keeps_its_visibility() {
    let mut editor = edit();
    let sequence = editor.active_sequence().unwrap().id;
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    for (flag, value) in [(TrackFlag::Locked, true), (TrackFlag::Enabled, false)] {
        editor
            .dispatch(Command::SetTrackFlag {
                sequence,
                track,
                flag,
                value,
            })
            .unwrap();
    }
    let copy = editor.duplicate_track(track).unwrap();
    let copied = editor.active_sequence().unwrap().audio_track(copy).unwrap();
    assert!(!copied.locked, "the copy came out locked");
    assert!(!copied.enabled, "the copy of a muted track is playing");
}

#[test]
fn an_unknown_track_is_refused() {
    let mut editor = edit();
    assert!(matches!(
        editor.duplicate_track(TrackId::new()),
        Err(EditorError::TrackNotFound(_))
    ));
}

/// The copy is journalled like any edit, so a crash keeps it.
#[test]
fn a_duplicated_track_survives_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dup.vproj");
    let copy = {
        let mut editor = edit();
        editor.save_as(&path).unwrap();
        let track = editor.active_sequence().unwrap().video_tracks[0].id;
        let copy = editor.duplicate_track(track).unwrap();
        std::mem::forget(editor);
        copy
    };
    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).unwrap();
    let sequence = session.project.active().unwrap();
    let recovered = sequence.video_track(copy).expect("the copy was lost");
    assert_eq!(recovered.clips().len(), 2);
    assert_eq!(
        recovered.clips()[1].timeline.start,
        TimelineTime::from_seconds(4)
    );
}
