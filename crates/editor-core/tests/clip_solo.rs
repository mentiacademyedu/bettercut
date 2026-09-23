//! Clip solo (`Editor::set_clip_solo`): one clip alone on screen and in the
//! mix, its linked sound with it, undone as one step.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_media::{MediaAsset, MediaKind};

fn two_shots() -> (Editor, Vec<ClipId>, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Solo");
    let mut placed = Vec::new();
    for name in ["a", "b"] {
        // A sound track, as `probe` would report one: placing it adds the
        // linked sound clip.
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(4),
        )
        .with_audio(48_000, 2);
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        placed.push(editor.place_media(media).unwrap());
    }
    let (first, second) = (placed.remove(0), placed.remove(0));
    assert_eq!(first.len(), 2, "picture and sound");
    (editor, first, second)
}

#[test]
fn soloing_a_shot_takes_its_sound_and_undoes_as_one_step() {
    let (mut editor, first, second) = two_shots();
    assert!(!editor.any_clip_soloed());
    let depth = editor.undo_depth();

    assert_eq!(editor.set_clip_solo(&[first[0]], true).unwrap(), 2);
    assert!(editor.clip_soloed(first[0]));
    assert!(
        editor.clip_soloed(first[1]),
        "the linked sound did not come along"
    );
    assert!(!editor.clip_soloed(second[0]));
    assert!(editor.any_clip_soloed());
    assert_eq!(editor.undo_depth(), depth + 1);
    let sequence = editor.active_sequence().unwrap();
    assert!(sequence.picture_clip_soloed() && sequence.sound_clip_soloed());
    assert!(sequence.clip_plays(first[0], true, true));
    assert!(!sequence.clip_plays(second[0], true, true));

    // Asking for what is already so changes nothing and adds no step.
    assert_eq!(editor.set_clip_solo(&[first[0]], true).unwrap(), 0);
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert!(!editor.any_clip_soloed());
    editor.redo().unwrap();
    assert!(editor.clip_soloed(first[1]));

    assert_eq!(editor.clear_clip_solos().unwrap(), 2);
    assert!(!editor.any_clip_soloed());
    assert_eq!(editor.clear_clip_solos().unwrap(), 0);
}

#[test]
fn a_clip_that_is_not_there_cannot_be_soloed() {
    let (mut editor, _, _) = two_shots();
    assert!(editor.set_clip_solo(&[ClipId::new()], true).is_err());
    assert!(!editor.any_clip_soloed());
}

#[test]
fn a_deleted_clip_leaves_no_solo_behind() {
    let (mut editor, first, second) = two_shots();
    editor.set_clip_solo(&[second[0]], true).unwrap();
    let sequence = editor.active_sequence().unwrap();
    let on: Vec<_> = second
        .iter()
        .map(|clip| (sequence.clip_span(*clip).unwrap().track, *clip))
        .collect();
    for (track, clip) in on {
        editor.remove_clip(track, clip).unwrap();
    }
    // Its id is still listed, but it counts for nothing: the other shot plays.
    assert!(!editor.any_clip_soloed());
    let sequence = editor.active_sequence().unwrap();
    assert!(sequence.clip_plays(first[0], true, sequence.picture_clip_soloed()));
}
