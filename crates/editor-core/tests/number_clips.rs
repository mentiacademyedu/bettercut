//! Naming several clips at once, numbered (`Editor::number_clips`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_media::{MediaAsset, MediaKind};

#[test]
fn clips_are_numbered_in_timeline_order_and_a_shot_shares_with_its_sound() {
    let (mut editor, _events) = Editor::new_project("Numbers");
    let mut placed = Vec::new();
    for name in ["a", "b", "c"] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(2),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        placed.extend(editor.place_media(media).unwrap());
    }
    let depth = editor.undo_depth();
    // Handed over out of order, pictures and sounds mixed.
    let mut chosen = placed.clone();
    chosen.reverse();
    assert_eq!(editor.number_clips(&chosen, " Shot ").unwrap(), 6);
    assert_eq!(editor.undo_depth(), depth + 1, "one step");

    let first = placed[0];
    let sound = editor
        .linked_with(first)
        .into_iter()
        .find(|c| *c != first)
        .unwrap();
    assert_eq!(editor.clip_name(first).as_deref(), Some("Shot 01"));
    assert_eq!(
        editor.clip_name(sound).as_deref(),
        Some("Shot 01"),
        "its sound shares"
    );
    let last = placed[placed.len() - 2];
    assert_eq!(editor.clip_name(last).as_deref(), Some("Shot 03"));

    assert_eq!(
        editor.number_clips(&chosen, "Shot").unwrap(),
        0,
        "already so"
    );
}
