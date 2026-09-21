//! Collect: the project and its files copied into one folder.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use bettercut_editor_core::Editor;
use bettercut_editor_core::collect::MEDIA_FOLDER;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};

/// A file of `size` bytes at `path`.
fn file(path: &Path, size: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![7_u8; size]).unwrap();
}

/// A project using two shots from different folders, both called clip.mp4,
/// plus a colour clip that has no file at all.
fn project(root: &Path) -> Editor {
    let (mut editor, _events) = Editor::new_project("Handover");
    for card in ["card-a", "card-b"] {
        let path = root.join(card).join("clip.mp4");
        file(&path, 32);
        editor.import_media(MediaAsset::new(
            MediaKind::Video,
            path,
            MediaTime::from_seconds(5),
        ));
    }
    editor
        .add_colour_clip(bettercut_editor_core::media::Generated::solid([0, 0, 0]))
        .unwrap();
    editor
}

#[test]
fn every_file_is_copied_in_and_the_project_points_at_the_copies() {
    let temp = tempfile::tempdir().unwrap();
    let editor = project(temp.path());
    let folder = temp.path().join("handover");

    let (files, bytes) = editor.collect_size();
    assert_eq!((files, bytes), (2, 64));

    let done = editor.collect_into(&folder).unwrap();
    assert_eq!(done.copied, 2);
    assert_eq!(done.bytes, 64);
    assert!(done.missing.is_empty(), "{:?}", done.missing);
    assert!(done.project.exists());

    // Both shots came across, and the second did not land on the first.
    let media = folder.join(MEDIA_FOLDER);
    assert!(media.join("clip.mp4").exists());
    assert!(media.join("clip (2).mp4").exists());

    // The collected project opens with its files found.
    let (copy, _events) = Editor::open(&done.project).unwrap();
    for asset in &copy.project().media {
        if asset.generated.is_some() {
            continue;
        }
        assert!(
            asset.path.starts_with(&media),
            "{} still points outside the folder",
            asset.path.display()
        );
        assert!(
            asset.path.exists(),
            "{} was not copied",
            asset.path.display()
        );
    }

    // And the project on screen still points at the originals: collecting is
    // a copy, not a move.
    for asset in &editor.project().media {
        assert!(!asset.path.starts_with(&media));
    }
}

#[test]
fn collecting_twice_over_the_same_folder_copies_nothing_again() {
    let temp = tempfile::tempdir().unwrap();
    let editor = project(temp.path());
    let folder = temp.path().join("handover");

    editor.collect_into(&folder).unwrap();
    let again = editor.collect_into(&folder).unwrap();
    assert_eq!(again.copied, 2, "still collected");
    assert_eq!(again.bytes, 0, "but nothing was copied a second time");
}

#[test]
fn a_file_that_is_not_there_is_reported_and_the_rest_still_collect() {
    let temp = tempfile::tempdir().unwrap();
    let mut editor = project(temp.path());
    editor.import_media(MediaAsset::new(
        MediaKind::Video,
        temp.path().join("gone").join("missing.mp4"),
        MediaTime::from_seconds(5),
    ));
    let folder = temp.path().join("handover");

    let done = editor.collect_into(&folder).unwrap();
    assert_eq!(done.copied, 2);
    assert_eq!(done.missing.len(), 1);
    assert!(
        done.missing[0].contains("missing.mp4"),
        "{:?}",
        done.missing
    );
    assert!(done.project.exists());
}
