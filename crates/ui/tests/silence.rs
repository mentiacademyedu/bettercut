//! Remove Silences, end to end through the window (§78).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use bettercut_cache::{CacheLayout, CacheStore, Peak, Waveform};
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use bettercut_ui::silence_dialog::SilenceDialog;
use egui::{Pos2, RawInput, Rect, vec2};

/// A ten-second clip with sound: talk, a two-second pause, talk.
fn setup(dir: &std::path::Path) -> (Editor, UiState, ClipId) {
    let (mut editor, _events) = Editor::new_project("Silence");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/talk.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();

    let peaks = (0..2_000)
        .map(|i| {
            // Quiet from 4 s to 6 s.
            let level: i8 = if (800..1_200).contains(&i) { 1 } else { 90 };
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
    (editor, state, placed[1])
}

/// Draw the window and return what it said.
///
/// Two frames on one context: a window sizes itself on the first and has
/// nothing to show until the second.
fn frame(editor: &mut Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let mut words = String::new();
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::silence_dialog::show(ui.ctx(), editor, state);
        });
        output.textures_delta.clear();
        words.clear();
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                words.push_str(text.galley.text());
                words.push(' ');
            }
        }
    }
    words
}

#[test]
fn the_window_shows_the_pause_and_changes_nothing_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, sound) = setup(dir.path());

    let dialog = SilenceDialog::open(&editor, &mut state.waveforms, sound).expect("a waveform");
    assert_eq!(dialog.ranges().len(), 1);
    // Two seconds of pause, less 120 ms of padding either side.
    assert_eq!(dialog.ranges()[0].start, TimelineTime::from_millis(4_120));
    assert_eq!(dialog.ranges()[0].end, TimelineTime::from_millis(5_880));
    state.silence = Some(dialog);

    let before = editor.project().clone();
    let words = frame(&mut editor, &mut state);
    assert!(words.contains("1 silence"), "{words}");
    assert!(words.contains("Remove 1"), "{words}");
    assert_eq!(editor.project(), &before, "drawing cut something");
    assert!(state.silence.is_some(), "the window closed by itself");
}

#[test]
fn a_clip_with_no_waveform_yet_opens_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (editor, _, sound) = setup(dir.path());
    let mut empty = UiState::default(); // no cache attached
    assert!(SilenceDialog::open(&editor, &mut empty.waveforms, sound).is_none());
}

/// The ranges the window offers are what the editor cuts.
#[test]
fn confirming_cuts_the_pause_out_of_both_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, sound) = setup(dir.path());
    let dialog = SilenceDialog::open(&editor, &mut state.waveforms, sound).expect("a waveform");
    let ranges = dialog.ranges().to_vec();

    let removed = editor.remove_ranges(sound, &ranges).unwrap();

    assert_eq!(removed, 1);
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 2);
    assert_eq!(sequence.audio_tracks[0].len(), 2);
    // Ten seconds less the 1.76 s pause, snapped to frames.
    let end = sequence.duration();
    assert!(
        (end.ticks() - TimelineTime::from_millis(8_240).ticks()).abs() < 32_000,
        "left {end:?}"
    );
}
