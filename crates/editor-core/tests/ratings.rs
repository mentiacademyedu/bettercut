//! Rating files in the media browser (`Editor::set_media_rating`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn library() -> (Editor, Vec<bettercut_editor_core::foundation::MediaId>) {
    let (mut editor, _events) = Editor::new_project("Takes");
    let ids = ["take 1", "take 2", "take 3"]
        .into_iter()
        .map(|name| {
            editor.import_media(MediaAsset::new(
                MediaKind::Video,
                format!("C:/media/{name}.mp4"),
                MediaTime::from_seconds(4),
            ))
        })
        .collect();
    (editor, ids)
}

#[test]
fn a_file_is_rated_cleared_and_undone() {
    let (mut editor, media) = library();
    assert_eq!(editor.media_rating(media[0]), 0);
    let depth = editor.undo_depth();

    assert!(editor.set_media_rating(media[0], 4).unwrap());
    assert_eq!(editor.media_rating(media[0]), 4);
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(editor.undo_label().as_deref(), Some("Rate 4 Stars"));

    // The same rating again is not a step.
    assert!(!editor.set_media_rating(media[0], 4).unwrap());
    assert_eq!(editor.undo_depth(), depth + 1);

    // Nothing rates zero — that is how a rating comes off.
    assert!(editor.set_media_rating(media[0], 0).unwrap());
    assert_eq!(editor.media_rating(media[0]), 0);

    editor.undo().unwrap();
    assert_eq!(editor.media_rating(media[0]), 4);
    editor.undo().unwrap();
    assert_eq!(editor.media_rating(media[0]), 0);
}

#[test]
fn a_rating_is_never_more_than_five_stars() {
    let (mut editor, media) = library();
    editor.set_media_rating(media[1], 99).unwrap();
    assert_eq!(editor.media_rating(media[1]), 5);
}

/// Ratings are part of the project, so they survive a save and a load.
#[test]
fn ratings_are_saved_with_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("takes.vproj");
    let (mut editor, media) = library();
    editor.set_media_rating(media[2], 5).unwrap();
    editor.save_as(&path).unwrap();

    let (opened, _events) = Editor::open(&path).unwrap();
    assert_eq!(opened.media_rating(media[2]), 5);
    assert_eq!(opened.media_rating(media[0]), 0);
}
