//! Notes left on clips.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn one_clip() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Notes");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(8),
    ));
    let clip = editor.place_media(media).unwrap()[0];
    (editor, clip)
}

/// A note is left, changed and taken off, each one undo step; the same text
/// again is no step; a clip that is not there is refused.
#[test]
fn a_note_is_left_changed_and_removed() {
    let (mut editor, clip) = one_clip();
    assert_eq!(editor.clip_note(clip), None);

    assert!(editor.set_clip_note(clip, "  swap for take 3 ").unwrap());
    assert_eq!(editor.clip_note(clip), Some("swap for take 3"));
    assert_eq!(editor.undo_label().as_deref(), Some("Clip Note"));

    let depth = editor.undo_depth();
    assert!(!editor.set_clip_note(clip, "swap for take 3").unwrap());
    assert!(
        !editor
            .set_clip_note(ClipId::new(), "")
            .is_ok_and(|changed| changed)
    );
    assert!(matches!(
        editor.set_clip_note(ClipId::new(), "nowhere"),
        Err(EditorError::ClipNotFound(_))
    ));
    assert_eq!(editor.undo_depth(), depth);

    editor.set_clip_note(clip, "check the audio").unwrap();
    editor.set_clip_note(clip, "   ").unwrap();
    assert_eq!(editor.clip_note(clip), None);
    assert_eq!(editor.active_sequence().unwrap().notes.len(), 0);

    editor.undo().unwrap();
    assert_eq!(editor.clip_note(clip), Some("check the audio"));
    editor.undo().unwrap();
    assert_eq!(editor.clip_note(clip), Some("swap for take 3"));
    editor.undo().unwrap();
    assert_eq!(editor.clip_note(clip), None);

    let long = "n".repeat(Editor::MAX_CLIP_NOTE + 1);
    editor.set_clip_note(clip, &long).unwrap();
    assert_eq!(
        editor.clip_note(clip).unwrap().chars().count(),
        Editor::MAX_CLIP_NOTE
    );
}

/// Splitting a clip leaves its note on both halves; undoing the split gives
/// the whole clip its note back.
#[test]
fn a_split_clip_keeps_its_note_on_both_halves() {
    let (mut editor, clip) = one_clip();
    editor.set_clip_note(clip, "colour is off").unwrap();
    editor.set_playhead(TimelineTime::from_seconds(3));
    editor.split_at_playhead(&[clip]).unwrap();

    let halves: Vec<ClipId> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(halves.len(), 2);
    for half in &halves {
        assert_eq!(editor.clip_note(*half), Some("colour is off"));
    }

    editor.undo().unwrap();
    assert_eq!(editor.clip_note(clip), Some("colour is off"));
    assert_eq!(
        editor.active_sequence().unwrap().notes.len(),
        1,
        "a half's note was left behind"
    );
}

/// Notes are saved; a project from before them loads with none.
#[test]
fn notes_are_saved_and_old_projects_have_none() {
    let (mut editor, clip) = one_clip();
    editor.set_clip_note(clip, "keep").unwrap();
    let mut json = serde_json::to_value(editor.project()).unwrap();
    let back: bettercut_editor_core::project_format::Project =
        serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back.sequences[0].notes.len(), 1);
    json["sequences"][0]
        .as_object_mut()
        .unwrap()
        .remove("notes");
    let old: bettercut_editor_core::project_format::Project = serde_json::from_value(json).unwrap();
    assert!(old.sequences[0].notes.is_empty());
}
