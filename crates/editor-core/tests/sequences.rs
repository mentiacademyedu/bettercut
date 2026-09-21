//! More than one sequence in a project: add, switch, duplicate, remove, rename.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn with_a_clip() -> Editor {
    let (mut editor, _events) = Editor::new_project("Sequences");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(8),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    editor
}

/// A new sequence comes after the others in the same format, with a track of
/// each everyday kind, and is shown; undo takes it away and shows the first.
#[test]
fn a_new_sequence_is_added_and_shown() {
    let mut editor = with_a_clip();
    let first = editor.active_sequence().unwrap().clone();

    let second = editor.add_sequence().unwrap();
    let list = editor.sequence_list();
    assert_eq!(list.len(), 2);
    assert_eq!(list[1].0, second);
    let shown = editor.active_sequence().unwrap();
    assert_eq!(shown.id, second, "the new sequence is not the one shown");
    assert_eq!(
        (shown.resolution, shown.frame_rate),
        (first.resolution, first.frame_rate)
    );
    assert_eq!(
        (
            shown.video_tracks.len(),
            shown.audio_tracks.len(),
            shown.text_tracks.len()
        ),
        (1, 1, 1)
    );
    assert_ne!(shown.name, first.name);

    editor.undo().unwrap();
    assert_eq!(editor.sequence_list().len(), 1);
    assert_eq!(editor.active_sequence().unwrap().id, first.id);
}

/// Switching is not an edit, and each sequence keeps its playhead.
#[test]
fn each_sequence_keeps_its_own_playhead() {
    let mut editor = with_a_clip();
    let first = editor.active_sequence().unwrap().id;
    editor.set_playhead(TimelineTime::from_seconds(5));
    let second = editor.add_sequence().unwrap();
    assert_eq!(editor.playhead(), TimelineTime::ZERO);
    editor.set_playhead(TimelineTime::from_seconds(2));
    let depth = editor.undo_depth();

    assert!(editor.switch_sequence(first));
    assert_eq!(editor.playhead(), TimelineTime::from_seconds(5));
    assert!(editor.switch_sequence(second));
    assert_eq!(editor.playhead(), TimelineTime::from_seconds(2));
    assert!(
        !editor.switch_sequence(second),
        "switching to the one shown did something"
    );
    assert_eq!(editor.undo_depth(), depth, "switching made an undo step");
}

/// A copy has every clip, all under new ids, with its picture and sound still
/// tied to each other — and an edit to the copy leaves the original alone.
#[test]
fn a_duplicate_is_independent_of_the_original() {
    let mut editor = with_a_clip();
    let picture = editor.active_sequence().unwrap().video_tracks[0].clips()[0].clone();
    editor.set_clip_note(picture.id, "keep").unwrap();
    let second = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/b.mp4",
        MediaTime::from_seconds(4),
    ));
    editor.place_media(second).unwrap();
    let other = editor.active_sequence().unwrap().video_tracks[0].clips()[1].id;
    editor.group_clips(&[picture.id, other]).unwrap();
    let original = editor.active_sequence().unwrap().clone();
    let copy_id = editor.duplicate_sequence(original.id).unwrap();

    let copy = editor.active_sequence().unwrap().clone();
    assert_eq!(copy.id, copy_id);
    assert_eq!(copy.name, format!("{} copy", original.name));
    let (copied_picture, copied_sound) = (
        &copy.video_tracks[0].clips()[0],
        &copy.audio_tracks[0].clips()[0],
    );
    assert_ne!(copied_picture.id, picture.id, "the copy shares a clip id");
    assert_ne!(
        copy.video_tracks[0].id, original.video_tracks[0].id,
        "the copy shares a track id"
    );
    assert!(copied_picture.link.is_some());
    assert_eq!(
        copied_picture.link, copied_sound.link,
        "the copy's picture and sound came apart"
    );
    assert_ne!(
        copied_picture.link, picture.link,
        "the copy is tied to the original's sound"
    );
    assert_eq!(
        editor.clip_note(copied_picture.id),
        Some("keep"),
        "the note did not follow"
    );
    let mut group = editor
        .group_of(copied_picture.id)
        .expect("the group did not follow");
    group.sort();
    let mut expected = vec![copied_picture.id, copy.video_tracks[0].clips()[1].id];
    expected.sort();
    assert_eq!(
        group, expected,
        "the copy's group holds the original's clips"
    );

    // Move the copy's picture; the original's stays put.
    let track = copy.video_tracks[0].id;
    editor
        .move_clip(
            track,
            track,
            copied_picture.id,
            TimelineTime::from_seconds(3),
        )
        .unwrap();
    let untouched =
        editor.project().sequence(original.id).unwrap().video_tracks[0].clips()[0].timeline;
    assert_eq!(untouched, picture.timeline);
}

/// The last sequence cannot go; removing the one shown shows its neighbour;
/// undo brings it back and shows it again.
#[test]
fn sequences_are_removed_but_never_the_last() {
    let mut editor = with_a_clip();
    let first = editor.active_sequence().unwrap().id;
    assert!(matches!(
        editor.remove_sequence(first),
        Err(EditorError::LastSequence)
    ));

    let second = editor.add_sequence().unwrap();
    editor.remove_sequence(second).unwrap();
    assert_eq!(editor.active_sequence().unwrap().id, first);
    editor.undo().unwrap();
    assert_eq!(editor.active_sequence().unwrap().id, second);
    assert_eq!(editor.sequence_list().len(), 2);
}

#[test]
fn a_sequence_is_renamed() {
    let mut editor = with_a_clip();
    let id = editor.active_sequence().unwrap().id;
    assert!(editor.rename_sequence(id, "  Vertical cut ").unwrap());
    assert_eq!(editor.active_sequence().unwrap().name, "Vertical cut");
    assert!(!editor.rename_sequence(id, "Vertical cut").unwrap());
    assert!(editor.rename_sequence(id, "  ").is_err());
    editor.undo().unwrap();
    assert_ne!(editor.active_sequence().unwrap().name, "Vertical cut");
}

/// Sequences added and edited are journalled, so a crash keeps them.
#[test]
fn a_new_sequence_survives_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sequences.vproj");
    let second = {
        let mut editor = with_a_clip();
        editor.save_as(&path).unwrap();
        let first = editor.active_sequence().unwrap().id;
        let copy = editor.duplicate_sequence(first).unwrap();
        editor.rename_sequence(copy, "Square").unwrap();
        std::mem::forget(editor);
        copy
    };
    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).unwrap();
    let recovered = session.project.sequence(second).expect("the copy was lost");
    assert_eq!(recovered.name, "Square");
    assert_eq!(recovered.video_tracks[0].clips().len(), 1);
}
