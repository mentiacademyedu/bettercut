//! The bookkeeping around a bake: recording one, replacing one, throwing them
//! away and dropping the ones whose files have gone
//! (`bettercut_editor_core::render_in_place`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::TimelineRange;

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn range(from: i64, to: i64) -> TimelineRange {
    TimelineRange::new(seconds(from), seconds(to)).expect("non-empty")
}

/// A file standing in for a finished bake, imported the way a real one is.
fn fake_bake(editor: &mut Editor, folder: &Path, name: &str) -> (MediaId, PathBuf) {
    let path = folder.join(name);
    std::fs::write(&path, b"not really a video").expect("write");
    let mut asset = MediaAsset::new(MediaKind::Video, path.clone(), MediaTime::from_seconds(4));
    asset.baked = true;
    (editor.import_media(asset), path)
}

#[test]
fn a_render_is_remembered_and_is_not_an_undo_step() {
    let folder = tempfile::tempdir().expect("temp");
    let (mut editor, _events) = Editor::new_project("Renders");
    let (media, _path) = fake_bake(&mut editor, folder.path(), "one.mp4");

    editor.record_render(range(0, 4), media, 7).expect("ok");

    assert_eq!(editor.renders().len(), 1);
    assert_eq!(editor.renders()[0].media, media);
    assert_eq!(editor.renders()[0].fingerprint, 7);
    assert!(editor.is_dirty(), "the project holds the render now");
    assert_eq!(
        editor.undo_label(),
        None,
        "a render is not an edit, so Ctrl+Z must not undo it"
    );
}

/// Re-baking a stretch replaces the bake of it, and the file the old one used
/// goes with it — it is ours, and nothing reads it any more.
#[test]
fn re_rendering_a_stretch_replaces_the_bake_and_its_file() {
    let folder = tempfile::tempdir().expect("temp");
    let (mut editor, _events) = Editor::new_project("Renders");
    let (first, first_path) = fake_bake(&mut editor, folder.path(), "one.mp4");
    let (second, second_path) = fake_bake(&mut editor, folder.path(), "two.mp4");

    editor.record_render(range(0, 4), first, 1).expect("ok");
    editor.record_render(range(0, 4), second, 2).expect("ok");

    assert_eq!(editor.renders().len(), 1, "one stretch, one bake");
    assert_eq!(editor.renders()[0].media, second);
    assert!(!first_path.exists(), "the replaced file should be gone");
    assert!(second_path.exists());
    assert!(
        editor.project().media_asset(first).is_none(),
        "and it should not still be in the project"
    );
}

#[test]
fn two_stretches_are_kept_apart() {
    let folder = tempfile::tempdir().expect("temp");
    let (mut editor, _events) = Editor::new_project("Renders");
    let (first, _) = fake_bake(&mut editor, folder.path(), "one.mp4");
    let (second, _) = fake_bake(&mut editor, folder.path(), "two.mp4");

    editor.record_render(range(0, 4), first, 1).expect("ok");
    editor.record_render(range(10, 14), second, 2).expect("ok");

    assert_eq!(editor.renders().len(), 2);
}

#[test]
fn throwing_the_renders_away_takes_the_files_with_them() {
    let folder = tempfile::tempdir().expect("temp");
    let (mut editor, _events) = Editor::new_project("Renders");
    let (first, first_path) = fake_bake(&mut editor, folder.path(), "one.mp4");
    let (second, second_path) = fake_bake(&mut editor, folder.path(), "two.mp4");
    editor.record_render(range(0, 4), first, 1).expect("ok");
    editor.record_render(range(10, 14), second, 2).expect("ok");

    assert_eq!(editor.clear_renders(), 2);
    assert!(editor.renders().is_empty());
    assert!(!first_path.exists());
    assert!(!second_path.exists());
    assert_eq!(editor.clear_renders(), 0, "and again is nothing to do");
}

/// A file the user imported is never deleted, whatever it is recorded as.
#[test]
fn only_files_this_program_wrote_are_deleted() {
    let folder = tempfile::tempdir().expect("temp");
    let (mut editor, _events) = Editor::new_project("Renders");
    let path = folder.path().join("the-users-own.mp4");
    std::fs::write(&path, b"footage").expect("write");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        path.clone(),
        MediaTime::from_seconds(4),
    ));
    editor.record_render(range(0, 4), media, 1).expect("ok");

    editor.clear_renders();
    assert!(path.exists(), "an imported file must survive");
    assert!(editor.project().media_asset(media).is_some());
}

#[test]
fn a_render_whose_file_has_gone_is_pruned() {
    let folder = tempfile::tempdir().expect("temp");
    let (mut editor, _events) = Editor::new_project("Renders");
    let (kept, _kept_path) = fake_bake(&mut editor, folder.path(), "one.mp4");
    let (lost, lost_path) = fake_bake(&mut editor, folder.path(), "two.mp4");
    editor.record_render(range(0, 4), kept, 1).expect("ok");
    editor.record_render(range(10, 14), lost, 2).expect("ok");

    std::fs::remove_file(&lost_path).expect("remove");
    assert_eq!(editor.prune_renders(), 1);
    assert_eq!(editor.renders().len(), 1);
    assert_eq!(editor.renders()[0].media, kept);
    assert_eq!(editor.prune_renders(), 0, "nothing left to prune");
}

/// The file a bake goes to names the stretch, so two of them cannot land on
/// one another.
#[test]
fn the_file_name_says_which_stretch_it_is() {
    let (editor, _events) = Editor::new_project("Renders");
    let first = editor.render_file(range(0, 4));
    let second = editor.render_file(range(10, 14));

    assert_ne!(first, second);
    assert_eq!(
        first.parent(),
        second.parent(),
        "both belong to this project"
    );
    let name = first.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.ends_with(".mp4"), "{name}");
    assert!(
        name.contains(&format!("{:012}", seconds(4).ticks())),
        "the name should carry the stretch: {name}"
    );
    // Asking twice gives the same file, so re-baking writes over the old one.
    assert_eq!(first, editor.render_file(range(0, 4)));
}

/// A project that is saved and opened again still knows what is baked.
#[test]
fn renders_survive_a_save_and_an_open() {
    let folder = tempfile::tempdir().expect("temp");
    let (mut editor, _events) = Editor::new_project("Renders");
    let (media, _path) = fake_bake(&mut editor, folder.path(), "one.mp4");
    editor.record_render(range(0, 4), media, 99).expect("ok");

    let project_path = folder.path().join("project.vproj");
    editor.save_as(&project_path).expect("saved");
    let (opened, _rx) = Editor::open(&project_path).expect("opened");

    assert_eq!(opened.renders().len(), 1);
    assert_eq!(opened.renders()[0].fingerprint, 99);
    assert_eq!(opened.renders()[0].media, media);
}
