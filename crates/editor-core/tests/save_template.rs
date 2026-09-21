//! Saving the edit as a template (`Editor::save_as_template`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::templates::SlotKind;
use bettercut_editor_core::timeline::Movement;
use bettercut_editor_core::{Editor, EditorError};

#[test]
fn an_edit_becomes_a_template_that_loads() {
    let (mut editor, _events) = Editor::new_project("Trip");
    let shot = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/beach.mp4",
        MediaTime::from_seconds(4),
    ));
    let photo = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/sunset.jpg",
        MediaTime::ZERO,
    ));
    let mut song = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.mp3",
        MediaTime::from_seconds(20),
    );
    song.audio_codec = Some("mp3".to_owned());
    let song = editor.import_media(song);

    let beach = editor.place_media(shot).unwrap()[0];
    editor.place_media(photo).unwrap();
    // The same shot again: one slot, used twice.
    editor.place_media(shot).unwrap();
    editor.place_media(song).unwrap();
    editor.set_movement(beach, Movement::ZoomIn).unwrap();
    editor.set_playhead(TimelineTime::from_seconds(1));
    editor.add_text("Summer").unwrap();
    editor
        .add_shape(bettercut_editor_core::text::ShapeKind::Star)
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let saved = editor.save_as_template("Summer Trip!", dir.path()).unwrap();
    assert_eq!(saved.path, dir.path().join("summer-trip.json"));
    assert_eq!(saved.slots, 3, "a shot, a photo and the music");
    assert_eq!(saved.left_out, 1, "the star has no place in a template");

    let template = bettercut_editor_core::templates::library::load_file(&saved.path).unwrap();
    assert_eq!(template.name, "Summer Trip!");
    let kinds: Vec<SlotKind> = template.slots.iter().map(|s| s.kind).collect();
    assert!(kinds.contains(&SlotKind::Video));
    assert!(kinds.contains(&SlotKind::Image));
    assert!(kinds.contains(&SlotKind::Audio));
    // Clips, the title and the music: the shot twice, the photo, the title, the song.
    assert_eq!(template.elements.len(), 5);

    // Saving again keeps the first.
    let again = editor.save_as_template("Summer Trip!", dir.path()).unwrap();
    assert_eq!(again.path, dir.path().join("summer-trip-2.json"));
}

#[test]
fn an_empty_edit_is_not_a_template() {
    let (editor, _events) = Editor::new_project("Nothing");
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        editor.save_as_template("Empty", dir.path()),
        Err(EditorError::TemplateNotSaved(_))
    ));
}
