//! "Mark Beats": markers on a music clip's beats, from its waveform.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use bettercut_cache::{CacheLayout, CacheStore, Peak, Waveform};
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;

/// A 20 s song at 120 bpm, placed from the start of the timeline, with its
/// waveform in a cache the store can read.
fn setup(dir: &std::path::Path) -> (Editor, UiState, bettercut_editor_core::foundation::ClipId) {
    let (mut editor, _events) = Editor::new_project("Beats");
    let mut asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.mp3",
        MediaTime::from_seconds(20),
    );
    asset.audio_codec = Some("mp3".to_owned());
    let media = editor.import_media(asset);
    let clip = editor.place_media(media).unwrap()[0];

    let peaks = (0..4_000)
        .map(|i| {
            let level: i8 = match i % 100 {
                0 => 120,
                1 => 80,
                2 => 50,
                _ => 12,
            };
            Peak {
                min: -level,
                max: level,
            }
        })
        .collect();
    let cache = Arc::new(CacheStore::new(CacheLayout::new(dir), u64::MAX));
    Waveform::new(200, peaks)
        .unwrap()
        .write(&cache.layout().waveform_file(media))
        .unwrap();

    let mut state = UiState::default();
    state.waveforms.attach(cache);
    (editor, state, clip)
}

#[test]
fn a_songs_beats_become_markers() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, clip) = setup(dir.path());

    bettercut_ui::context_menu::mark_beats(&mut editor, &mut state, clip);

    let markers = editor.markers();
    assert!(
        markers.len() >= 38,
        "{} markers for 40 beats",
        markers.len()
    );
    assert_eq!(markers[1].time, TimelineTime::from_millis(500));
    let status = state.status.as_ref().unwrap();
    assert!(status.text.contains("120 bpm"), "{}", status.text);

    // Once is enough: asking again adds nothing, and says so.
    bettercut_ui::context_menu::mark_beats(&mut editor, &mut state, clip);
    assert!(
        state
            .status
            .as_ref()
            .unwrap()
            .text
            .contains("already marked")
    );

    // And the whole set is one undo step.
    editor.undo().unwrap();
    assert!(editor.markers().is_empty());
}

#[test]
fn a_waveform_not_ready_yet_is_said_plainly() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _, clip) = setup(dir.path());
    let mut state = UiState::default(); // no cache attached: nothing analysed

    bettercut_ui::context_menu::mark_beats(&mut editor, &mut state, clip);

    assert!(editor.markers().is_empty());
    assert!(
        state
            .status
            .as_ref()
            .unwrap()
            .text
            .contains("still being analysed")
    );
}
