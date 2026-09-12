//! Normalising a clip's level, end to end (§20a.4).
//!
//! The arithmetic is tested in `playback`; this is the part that reads the
//! clip's cached waveform, writes the gain, and says what it did.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use bettercut_cache::{CacheLayout, CacheStore, Peak, Waveform};
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;

/// A ten-second clip whose sound peaks at `level` out of 127.
fn setup(dir: &std::path::Path, level: i8) -> (Editor, UiState, ClipId) {
    let (mut editor, _events) = Editor::new_project("Normalise");
    let mut asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/quiet.wav",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("pcm".to_owned());
    let media = editor.import_media(asset);
    let sound = editor.place_media(media).unwrap()[0];

    let cache = Arc::new(CacheStore::new(CacheLayout::new(dir), u64::MAX));
    Waveform::new(
        200,
        vec![
            Peak {
                min: -level,
                max: level
            };
            2_000
        ],
    )
    .unwrap()
    .write(&cache.layout().waveform_file(media))
    .unwrap();

    let mut state = UiState::default();
    state.waveforms.attach(cache);
    (editor, state, sound)
}

fn gain(editor: &Editor, clip: ClipId) -> f32 {
    editor.audio_clip(clip).unwrap().gain
}

#[test]
fn a_quiet_clip_is_raised_and_the_change_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, sound) = setup(dir.path(), 38);
    assert_eq!(gain(&editor, sound), 1.0);

    bettercut_ui::context_menu::normalise_volume(&mut editor, &mut state, sound);

    let after = gain(&editor, sound);
    assert!((after - 3.0).abs() < 0.2, "expected about 3x, got {after}");
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("+9.5 dB")),
        "{:?}",
        state.status
    );
}

/// Normalising replaces the gain rather than multiplying it: the peak is
/// measured from the source, so the answer does not depend on what the user
/// had already set — and normalising twice does not raise it twice.
#[test]
fn normalising_twice_lands_in_the_same_place() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, sound) = setup(dir.path(), 38);

    bettercut_ui::context_menu::normalise_volume(&mut editor, &mut state, sound);
    let once = gain(&editor, sound);
    bettercut_ui::context_menu::normalise_volume(&mut editor, &mut state, sound);

    assert_eq!(
        gain(&editor, sound),
        once,
        "the second pass raised it again"
    );
}

#[test]
fn a_clip_already_at_the_target_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, sound) = setup(dir.path(), 113);
    let before = editor.undo_depth();

    bettercut_ui::context_menu::normalise_volume(&mut editor, &mut state, sound);

    assert_eq!(gain(&editor, sound), 1.0);
    assert_eq!(
        editor.undo_depth(),
        before,
        "an inaudible step went into the history"
    );
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("already at a good level")),
        "{:?}",
        state.status
    );
}

#[test]
fn a_clip_still_being_analysed_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _state, sound) = setup(dir.path(), 38);
    let mut state = UiState::default(); // no waveform cache attached

    bettercut_ui::context_menu::normalise_volume(&mut editor, &mut state, sound);

    assert_eq!(gain(&editor, sound), 1.0);
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("still being analysed")),
        "{:?}",
        state.status
    );
}
