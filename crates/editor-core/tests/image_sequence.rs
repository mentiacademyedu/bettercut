//! Importing numbered stills as one clip (`Editor::import_image_sequence`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{FrameRate, MediaTime};

fn write_frame(dir: &Path, number: u32) {
    let mut ppm = b"P6\n2 2\n255\n".to_vec();
    ppm.extend_from_slice(&[90, 40, 20].repeat(4));
    std::fs::write(dir.join(format!("shot.{number:03}.ppm")), ppm).unwrap();
}

#[test]
fn the_run_imports_as_one_video_and_places_at_its_length() {
    let dir = tempfile::tempdir().unwrap();
    for number in 1..=3 {
        write_frame(dir.path(), number);
    }
    let (mut editor, _events) = Editor::new_project("Stills");
    let id = editor
        .import_image_sequence(&dir.path().join("shot.002.ppm"), FrameRate::PAL_25)
        .unwrap();
    let asset = editor.project().media_asset(id).unwrap();
    assert!(!asset.is_still());
    assert_eq!(asset.duration, MediaTime::from_millis(120));
    assert_eq!(asset.sequence.unwrap().count, 3);

    let clips = editor.place_media(id).unwrap();
    assert_eq!(clips.len(), 1, "one picture clip, no sound");
    let sequence = editor.active_sequence().unwrap();
    let clip = &sequence.video_tracks[0].clips()[0];
    assert_eq!(clip.source.duration(), MediaTime::from_millis(120));
}

#[test]
fn a_lone_still_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_frame(dir.path(), 7);
    let (mut editor, _events) = Editor::new_project("Stills");
    assert!(
        editor
            .import_image_sequence(&dir.path().join("shot.007.ppm"), FrameRate::PAL_25)
            .is_err()
    );
    assert!(editor.project().media.is_empty());
}
