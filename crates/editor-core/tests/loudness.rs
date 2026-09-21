//! Matching the whole mix to a loudness target (`Editor::match_loudness`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn editor_with_sound() -> Editor {
    let (mut editor, _events) = Editor::new_project("Levels");
    let mut asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/mix.wav",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("pcm".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    editor
}

fn volume(editor: &Editor) -> f32 {
    editor.active_sequence().unwrap().master_volume
}

#[test]
fn a_quiet_mix_is_turned_up_to_its_target() {
    let mut editor = editor_with_sound();
    assert_eq!(volume(&editor), 1.0);
    let depth = editor.undo_depth();

    // Six dB under: a doubling of the master volume.
    let applied = editor.match_loudness(-20.0, -14.0).unwrap();
    assert!((applied - 2.0).abs() < 0.01, "{applied}");
    assert!((volume(&editor) - applied).abs() < 1e-5);
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(volume(&editor), 1.0);
}

#[test]
fn a_loud_mix_is_turned_down() {
    let mut editor = editor_with_sound();
    let applied = editor.match_loudness(-8.0, -14.0).unwrap();
    assert!(applied < 1.0 && applied > 0.4, "{applied}");
}

/// A mix already on target is left exactly as it is — and makes no undo step.
#[test]
fn a_mix_on_target_is_left_alone() {
    let mut editor = editor_with_sound();
    let depth = editor.undo_depth();
    let applied = editor.match_loudness(-14.0, -14.0).unwrap();
    assert!((applied - 1.0).abs() < 1e-5);
    assert_eq!(editor.undo_depth(), depth);
}

/// The master volume has a ceiling, so a nearly silent mix is brought up as
/// far as it goes and no further.
#[test]
fn the_volume_is_held_to_what_the_master_allows() {
    let mut editor = editor_with_sound();
    let applied = editor.match_loudness(-60.0, -14.0).unwrap();
    assert!(applied <= 4.0, "{applied}");
    assert!(applied > 1.0);
}

#[test]
fn a_measurement_that_is_not_a_number_is_refused() {
    let mut editor = editor_with_sound();
    assert!(matches!(
        editor.match_loudness(f32::NAN, -14.0),
        Err(EditorError::NothingToHear)
    ));
}
