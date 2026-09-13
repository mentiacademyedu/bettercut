//! Reversing clips through the editor (`Editor::set_reversed`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{ClipProperty, Editor, EditorError};

/// A video with sound, placed: picture and sound, linked.
fn linked_pair() -> (
    Editor,
    bettercut_editor_core::foundation::ClipId,
    bettercut_editor_core::foundation::ClipId,
) {
    let (mut editor, _events) = Editor::new_project("Reverse");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(6),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0], placed[1])
}

/// §12: the sound runs backwards under its picture — from either side — and
/// one undo puts both forwards again.
#[test]
fn a_linked_pair_reverses_together_in_one_step() {
    let (mut editor, picture, sound) = linked_pair();
    let depth = editor.undo_depth();

    editor.set_reversed(sound, true).unwrap();

    assert!(
        editor.video_clip(picture).unwrap().reversed,
        "the picture was left forwards"
    );
    assert!(editor.audio_clip(sound).unwrap().reversed);
    assert!(editor.is_reversed(picture));
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(editor.undo_label().as_deref(), Some("Reverse"));

    editor.undo().unwrap();
    assert!(!editor.is_reversed(picture) && !editor.is_reversed(sound));
}

/// Reversing leaves the clip where it is, the same length, playing the same
/// material — only the direction changes.
#[test]
fn reversing_changes_the_direction_and_nothing_else() {
    let (mut editor, picture, _) = linked_pair();
    let before = editor.video_clip(picture).unwrap().clone();

    editor.set_reversed(picture, true).unwrap();
    editor.set_reversed(picture, false).unwrap();

    assert_eq!(editor.video_clip(picture).unwrap(), &before);
}

/// A photo has no motion to reverse; refused, and nothing in the history.
#[test]
fn a_photo_is_not_reversed() {
    let (mut editor, _events) = Editor::new_project("Reverse");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/p.png",
        MediaTime::ZERO,
    ));
    let photo = editor.place_media(media).unwrap()[0];
    let label = editor.undo_label();

    let result = editor.set_reversed(photo, true);

    assert!(
        matches!(result, Err(EditorError::NoMotionToRetime)),
        "{result:?}"
    );
    assert_eq!(editor.undo_label(), label);
}

/// A keyframe placed on a reversed clip is evaluated where it was placed, and
/// the timeline shows it at the playhead — keys ride on the frames, backwards
/// with the picture.
#[test]
fn a_key_on_a_reversed_clip_lands_at_the_playhead() {
    let (mut editor, picture, _) = linked_pair();
    editor.set_reversed(picture, true).unwrap();
    let playhead = TimelineTime::from_seconds(1);
    editor.set_playhead(playhead);

    let at = editor.source_time_at_playhead(picture).unwrap();

    let clip = editor.video_clip(picture).unwrap();
    assert_eq!(
        at,
        clip.progress_time_at(playhead),
        "placed where it is not evaluated"
    );
    assert_eq!(
        clip.timeline_time_of(at),
        playhead,
        "shown away from the playhead"
    );
}

/// The whole video has no direction to reverse.
#[test]
fn the_whole_video_is_not_reversed() {
    let (mut editor, _, _) = linked_pair();
    assert!(
        editor
            .set_sequence_value(ClipProperty::Reverse(true), false)
            .is_err()
    );
    assert!(ClipProperty::Reverse(false).is_default());
    assert!(!ClipProperty::Reverse(true).is_default());
}
