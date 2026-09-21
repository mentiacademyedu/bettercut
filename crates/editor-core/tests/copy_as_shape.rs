//! Copy a sequence at another shape (`Editor::copy_sequence_as`): a vertical or
//! square cut of a landscape edit, kept in the project to keep working on.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{FrameRate, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{Resolution, crop_to_aspect};
use bettercut_editor_core::{ClipProperty, Editor, SHAPES, Shape};

fn shape(label: &str) -> Shape {
    SHAPES.into_iter().find(|s| s.label == label).unwrap()
}

/// A 1920×1080 sequence with two landscape clips, the second shrunk by hand.
fn landscape_edit() -> Editor {
    let (mut editor, _events) = Editor::new_project("Shapes");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/wide.mp4",
        MediaTime::from_seconds(10),
    );
    asset.width = 1920;
    asset.height = 1080;
    let media = editor.import_media(asset);
    editor
        .set_sequence_format(Resolution::HD_1080, FrameRate::FPS_30)
        .unwrap();
    editor.place_media(media).unwrap();
    let framed = editor.place_media(media).unwrap()[0];
    editor
        .set_clip_property(framed, ClipProperty::Scale { x: 0.5, y: 0.5 }, false)
        .unwrap();
    editor
}

#[test]
fn the_copy_is_the_new_shape_with_its_shots_filling_it() {
    let mut editor = landscape_edit();
    let original = editor.active_sequence().unwrap().clone();
    let depth = editor.undo_depth();

    let copy = editor.copy_sequence_as(original.id, shape("9:16")).unwrap();

    // Switched to, named for its shape, one step, right after the original.
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.id, copy);
    assert_eq!(sequence.name, format!("{} 9:16", original.name));
    assert_eq!(sequence.resolution, Resolution::new(1080, 1920));
    assert_eq!(editor.undo_depth(), depth + 1);
    let index = editor
        .project()
        .sequences
        .iter()
        .position(|s| s.id == copy)
        .unwrap();
    assert_eq!(editor.project().sequences[index - 1].id, original.id);

    // Fresh clips; the untouched shot is cropped to fill, the shrunk one kept.
    let clips = sequence.video_tracks[0].clips();
    assert_eq!(clips.len(), 2);
    assert!(clips.iter().all(|c| original.clip_span(c.id).is_none()));
    assert_eq!(
        clips[0].crop,
        crop_to_aspect(1920.0 / 1080.0, 1080.0 / 1920.0)
    );
    assert!(clips[1].crop.is_none(), "hand framing was changed");

    // The original is exactly as it was.
    let now = editor.project().sequence(original.id).unwrap();
    assert_eq!(now.resolution, Resolution::HD_1080);
    assert!(now.video_tracks[0].clips().iter().all(|c| c.crop.is_none()));

    editor.undo().unwrap();
    assert!(editor.project().sequence(copy).is_none());
}

#[test]
fn a_square_copy_keeps_the_short_edge() {
    let mut editor = landscape_edit();
    let id = editor.active_sequence().unwrap().id;
    editor.copy_sequence_as(id, shape("1:1")).unwrap();
    assert_eq!(
        editor.active_sequence().unwrap().resolution,
        Resolution::new(1080, 1080)
    );
}
