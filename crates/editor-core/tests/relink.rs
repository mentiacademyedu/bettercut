//! Media relink (§66).
//!
//! The failure this prevents: a user moves their footage folder, reopens the
//! project, and every clip is dead with no way back. Relinking has to restore
//! the project *without* redefining what the clips contain — a same-named file
//! of a different size is a different file, and swapping it in under existing
//! cuts would be worse than leaving the media missing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor};

/// Write a file of a given size, so the size check has something to check.
fn write(path: &Path, bytes: usize) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, vec![7_u8; bytes]).unwrap();
}

/// Move a file, creating the destination folder — `rename` will not.
fn move_to(from: &Path, to: &Path) {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::rename(from, to).unwrap();
}

/// An asset with a recorded size, as a real import produces (the prober fills
/// `file_size` from the file). Size is what makes the relink check meaningful.
fn sized(path: &Path, file_size: u64) -> MediaAsset {
    let mut asset = MediaAsset::new(MediaKind::Video, path, MediaTime::from_seconds(5));
    asset.file_size = file_size;
    asset
}

/// An editor holding one asset that points at `path`, with a clip using it.
fn editor_with(path: &Path) -> (Editor, bettercut_editor_core::foundation::MediaId) {
    let (mut editor, _rx) = Editor::new_project("Relink");
    let asset = MediaAsset::new(MediaKind::Video, path, MediaTime::from_seconds(10));
    let media = editor.import_media(asset);

    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();
    let clip = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    (editor, media)
}

#[test]
fn a_moved_file_is_detected_as_missing() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("shot.mp4");
    write(&original, 128);

    let (mut editor, media) = editor_with(&original);
    assert_eq!(editor.refresh_missing_media(), 0, "the file is right there");

    std::fs::remove_file(&original).unwrap();
    assert_eq!(editor.refresh_missing_media(), 1);
    assert!(editor.project().media_asset(media).unwrap().missing);
}

#[test]
fn relinking_restores_the_asset_and_keeps_the_clip() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("shot.mp4");
    let moved = dir.path().join("backup").join("shot.mp4");
    write(&original, 128);

    let (mut editor, media) = editor_with(&original);
    let clip_before = editor.active_sequence().unwrap().video_tracks[0].clips()[0].timeline;

    move_to(&original, &moved);
    editor.refresh_missing_media();

    editor.relink_media(media, &moved).expect("relink");

    let asset = editor.project().media_asset(media).unwrap();
    assert!(!asset.missing, "still marked missing after relink");
    assert_eq!(asset.path, moved);
    assert_eq!(asset.file_size, 128);

    // The edit must survive: relinking is about the file, not the cut.
    let clip_after = editor.active_sequence().unwrap().video_tracks[0].clips()[0].timeline;
    assert_eq!(clip_after, clip_before, "relinking moved the clip");
}

/// Relinking must not redefine the media. A file that decodes differently
/// should be imported, not relinked over the top of existing cuts.
#[test]
fn relinking_does_not_change_decoded_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("shot.mp4");
    let moved = dir.path().join("shot.mp4.moved");
    write(&original, 128);

    let (mut editor, media) = editor_with(&original);
    let before = editor.project().media_asset(media).unwrap().clone();

    move_to(&original, &moved);
    editor.refresh_missing_media();
    editor.relink_media(media, &moved).expect("relink");

    let after = editor.project().media_asset(media).unwrap();
    assert_eq!(after.duration, before.duration);
    assert_eq!((after.width, after.height), (before.width, before.height));
    assert_eq!(after.color, before.color);
    assert_eq!(after.kind, before.kind);
}

/// A rename should update the displayed name, or the browser keeps showing a
/// file name that no longer exists anywhere.
#[test]
fn a_renamed_file_updates_the_display_name() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("shot.mp4");
    let renamed = dir.path().join("shot-final.mp4");
    write(&original, 64);

    let (mut editor, media) = editor_with(&original);
    move_to(&original, &renamed);
    editor.refresh_missing_media();
    editor.relink_media(media, &renamed).expect("relink");

    assert_eq!(
        editor.project().media_asset(media).unwrap().file_name,
        "shot-final.mp4"
    );
}

#[test]
fn undo_restores_the_missing_state_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("shot.mp4");
    let moved = dir.path().join("elsewhere").join("shot.mp4");
    write(&original, 128);

    let (mut editor, media) = editor_with(&original);
    move_to(&original, &moved);
    editor.refresh_missing_media();
    editor.relink_media(media, &moved).expect("relink");

    editor.undo().expect("undo");

    let asset = editor.project().media_asset(media).unwrap();
    assert!(asset.missing, "undo left the asset looking present");
    assert_eq!(asset.path, original);
}

// ---- locate folder -------------------------------------------------------

/// §66's "locate folder": one action fixes every file that moved together.
#[test]
fn locating_a_folder_relinks_everything_that_matches() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.mp4"), dir.path().join("b.mp4"));
    write(&a, 100);
    write(&b, 200);

    let (mut editor, _rx) = Editor::new_project("Relink");
    let id_a = editor.import_media(sized(&a, 100));
    let id_b = editor.import_media(sized(&b, 200));

    let backup = dir.path().join("backup");
    std::fs::create_dir_all(&backup).unwrap();
    move_to(&a, &backup.join("a.mp4"));
    move_to(&b, &backup.join("b.mp4"));
    assert_eq!(editor.refresh_missing_media(), 2);

    assert_eq!(editor.relink_from_folder(&backup).expect("scan"), 2);
    assert!(!editor.project().media_asset(id_a).unwrap().missing);
    assert!(!editor.project().media_asset(id_b).unwrap().missing);
}

/// §79: one user action, one undo step, however many files it touched.
#[test]
fn a_folder_relink_undoes_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.mp4"), dir.path().join("b.mp4"));
    write(&a, 100);
    write(&b, 200);

    let (mut editor, _rx) = Editor::new_project("Relink");
    editor.import_media(sized(&a, 100));
    editor.import_media(sized(&b, 200));

    let backup = dir.path().join("backup");
    std::fs::create_dir_all(&backup).unwrap();
    move_to(&a, &backup.join("a.mp4"));
    move_to(&b, &backup.join("b.mp4"));
    editor.refresh_missing_media();
    editor.relink_from_folder(&backup).expect("scan");

    editor.undo().expect("one undo");

    assert!(
        editor.project().media.iter().all(|m| m.missing),
        "one undo did not restore both assets"
    );
}

/// The conservative half: a same-named file of a different size is a different
/// file, and must not be swapped in under existing cuts.
#[test]
fn a_same_name_different_size_file_is_not_relinked() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("shot.mp4");
    write(&original, 500);

    let (mut editor, _rx) = Editor::new_project("Relink");
    let media = editor.import_media(sized(&original, 500));

    // The real file is gone; a same-named file of a different size sits in the
    // folder the user points at.
    std::fs::remove_file(&original).unwrap();
    let decoy_dir = dir.path().join("decoy");
    write(&decoy_dir.join("shot.mp4"), 501);

    assert_eq!(editor.refresh_missing_media(), 1);

    let relinked = editor.relink_from_folder(&decoy_dir).expect("scan");
    assert_eq!(relinked, 0, "an impostor file was silently relinked");
    assert!(
        editor.project().media_asset(media).unwrap().missing,
        "the asset was quietly repointed at a different file"
    );
}

/// The same file, same size, in a new folder — the case relink exists for.
#[test]
fn a_matching_size_in_a_new_folder_is_relinked() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("shot.mp4");
    write(&original, 500);

    let (mut editor, _rx) = Editor::new_project("Relink");
    let media = editor.import_media(sized(&original, 500));

    std::fs::remove_file(&original).unwrap();
    let moved_dir = dir.path().join("moved");
    write(&moved_dir.join("shot.mp4"), 500);
    editor.refresh_missing_media();

    assert_eq!(editor.relink_from_folder(&moved_dir).expect("scan"), 1);
    assert!(!editor.project().media_asset(media).unwrap().missing);
}

#[test]
fn scanning_a_folder_with_nothing_matching_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("shot.mp4");
    write(&original, 10);
    let (mut editor, _) = editor_with(&original);
    std::fs::remove_file(&original).unwrap();
    editor.refresh_missing_media();

    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(editor.relink_from_folder(&empty).expect("scan"), 0);
}

#[test]
fn scanning_a_folder_that_does_not_exist_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _) = editor_with(&dir.path().join("shot.mp4"));
    assert_eq!(
        editor
            .relink_from_folder(dir.path().join("nope"))
            .expect("scan"),
        0
    );
}

/// §38.2 replays commands after a crash, so this one must survive JSON — and
/// must not re-stat the filesystem when it does.
#[test]
fn the_command_round_trips_through_json() {
    let command = bettercut_editor_core::Command::RelinkMedia {
        media: bettercut_editor_core::foundation::MediaId::new(),
        path: std::path::PathBuf::from("D:/footage/shot.mp4"),
        file_size: 4096,
    };

    let json = serde_json::to_string(&command).expect("serialize");
    assert!(json.contains("relink_media"), "unexpected tag: {json}");
    assert_eq!(
        serde_json::from_str::<bettercut_editor_core::Command>(&json).expect("deserialize"),
        command
    );
}
