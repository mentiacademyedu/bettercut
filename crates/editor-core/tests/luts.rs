//! Importing colour lookup tables and putting them on clips
//! (`Editor::import_lut`, `ClipProperty::Lut`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{ClipLut, CubeLut};
use bettercut_editor_core::{ClipProperty, Editor, EditorError};

/// A small valid `.cube` file on disk.
fn write_cube(dir: &Path, name: &str, title: Option<&str>) -> PathBuf {
    let lut = CubeLut::identity(2);
    let mut text = String::new();
    if let Some(title) = title {
        text.push_str(&format!("TITLE \"{title}\"\n"));
    }
    text.push_str("LUT_3D_SIZE 2\n");
    for [r, g, b] in &lut.table {
        text.push_str(&format!("{r} {g} {b}\n"));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// A video with sound placed on the timeline: picture and sound clips.
fn editor_with_clips() -> (
    Editor,
    bettercut_editor_core::foundation::ClipId,
    bettercut_editor_core::foundation::ClipId,
) {
    let (mut editor, _events) = Editor::new_project("LUTs");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0], placed[1])
}

/// An imported table is listed by the name its file gives it — the title if it
/// has one, the file name if not — and importing the same file again gives
/// back the same table rather than a second copy.
#[test]
fn a_table_is_imported_once_under_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let titled = write_cube(dir.path(), "teal.cube", Some("Teal and Orange"));
    let untitled = write_cube(dir.path(), "Faded Film.cube", None);
    let (mut editor, _, _) = editor_with_clips();

    let first = editor.import_lut(&titled).unwrap();
    let second = editor.import_lut(&untitled).unwrap();
    let again = editor.import_lut(&titled).unwrap();

    assert_eq!(first, again, "the same file was imported twice");
    assert_ne!(first, second);
    let names: Vec<&str> = editor
        .project()
        .luts
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(names, vec!["Teal and Orange", "Faded Film"]);
    assert!(editor.is_dirty());
}

/// A broken file is refused at import, with its reason, and nothing is added —
/// rather than accepted and silently drawing ungraded later.
#[test]
fn a_broken_file_is_refused_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.cube");
    std::fs::write(&path, "LUT_1D_SIZE 1024\n0 0 0\n").unwrap();
    let (mut editor, _, _) = editor_with_clips();

    let err = editor.import_lut(&path).unwrap_err();

    assert!(
        matches!(&err, EditorError::Lut(message) if message.contains("1D")),
        "{err}"
    );
    assert!(editor.project().luts.is_empty());

    let missing = editor
        .import_lut(dir.path().join("nowhere.cube"))
        .unwrap_err();
    assert!(missing.to_string().contains("nowhere.cube"), "{missing}");
}

/// On a picture: set, undone, strength kept in range. Refused by sound and by
/// the whole video. And a reset offered only once one is set.
#[test]
fn a_clip_takes_a_table_and_sound_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, picture, sound) = editor_with_clips();
    let id = editor
        .import_lut(write_cube(dir.path(), "look.cube", None))
        .unwrap();

    assert!(ClipProperty::Lut(None).is_default());
    assert!(!ClipProperty::Lut(Some(ClipLut::new(id))).is_default());

    editor
        .set_clip_property(
            picture,
            ClipProperty::Lut(Some(ClipLut {
                lut: id,
                strength: 7.0,
            })),
            false,
        )
        .unwrap();
    assert_eq!(
        editor.video_clip(picture).unwrap().lut,
        Some(ClipLut {
            lut: id,
            strength: 1.0
        }),
        "the strength was not brought into range"
    );
    editor.undo().unwrap();
    assert_eq!(editor.video_clip(picture).unwrap().lut, None);

    assert!(
        editor
            .set_clip_property(sound, ClipProperty::Lut(Some(ClipLut::new(id))), false)
            .is_err(),
        "a sound clip accepted a LUT"
    );
    assert!(
        editor
            .set_sequence_value(ClipProperty::Lut(Some(ClipLut::new(id))), false)
            .is_err(),
        "the whole video accepted a LUT"
    );
}

/// The table list and a clip's use of it survive a save and a load.
#[test]
fn tables_survive_a_save_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, picture, _) = editor_with_clips();
    let id = editor
        .import_lut(write_cube(dir.path(), "look.cube", None))
        .unwrap();
    editor
        .set_clip_property(
            picture,
            ClipProperty::Lut(Some(ClipLut {
                lut: id,
                strength: 0.4,
            })),
            false,
        )
        .unwrap();
    let file = dir.path().join("project.vproj");
    editor.save_as(&file).unwrap();

    let (opened, _events) = Editor::open(&file).unwrap();

    assert_eq!(opened.project().luts, editor.project().luts);
    assert_eq!(
        opened.video_clip(picture).unwrap().lut,
        Some(ClipLut {
            lut: id,
            strength: 0.4
        })
    );
}
