//! Taking media out of the project (§12, §2).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor};

fn editor_with_media() -> (Editor, bettercut_editor_core::foundation::MediaId) {
    let (mut editor, _rx) = Editor::new_project("Library");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    (editor, media)
}

#[test]
fn an_unused_asset_can_be_removed() {
    let (mut editor, media) = editor_with_media();
    assert_eq!(editor.project().media.len(), 1);

    editor.remove_media(media).unwrap();
    assert!(editor.project().media.is_empty());
}

/// Removing media a clip still uses would leave that cut pointing at nothing.
/// The refusal is the point: throwing the clip away too would discard an edit
/// the user never asked to lose.
#[test]
fn an_asset_in_use_is_refused() {
    let (mut editor, media) = editor_with_media();
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();
    let clip = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();

    assert!(editor.media_is_used(media));
    assert!(editor.remove_media(media).is_err());
    assert_eq!(
        editor.project().media.len(),
        1,
        "the asset was removed anyway"
    );
}

/// And once the clip is gone, it can be removed — so the message the interface
/// shows ("delete those clips first") is actually actionable.
#[test]
fn removing_the_clip_makes_the_asset_removable() {
    let (mut editor, media) = editor_with_media();
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();
    let clip = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let clip_id = clip.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();

    editor.remove_clip(track, clip_id).unwrap();
    assert!(!editor.media_is_used(media));
    editor.remove_media(media).unwrap();
    assert!(editor.project().media.is_empty());
}

/// §11: it is an edit, so it undoes — with the asset intact, not a fresh import
/// that would carry a new id and break anything referring to the old one.
#[test]
fn removing_media_undoes() {
    let (mut editor, media) = editor_with_media();
    let before = editor.project().media_asset(media).unwrap().clone();

    editor.remove_media(media).unwrap();
    editor.undo().unwrap();

    let after = editor.project().media_asset(media).expect("asset is back");
    assert_eq!(after.id, before.id, "the identity has to survive");
    assert_eq!(after.path, before.path);
    assert_eq!(after.duration, before.duration);
}

#[test]
fn removing_an_asset_that_is_not_there_fails_cleanly() {
    let (mut editor, media) = editor_with_media();
    editor.remove_media(media).unwrap();
    assert!(editor.remove_media(media).is_err());
}
