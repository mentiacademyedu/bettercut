//! Timing captions from speech, end to end (§28).
//!
//! Half of automatic captions with no model in sight: the waveform says *when*
//! someone is talking, and the user types *what* they said. This is the whole
//! path — a clip's sound, through the phrase finder, onto the caption lane.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use bettercut_cache::{CacheLayout, CacheStore, Peak, Waveform};
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;

/// A six-second clip: two seconds of talk, a second of pause, two more of
/// talk, a second of quiet at the end.
fn setup(dir: &std::path::Path) -> (Editor, UiState, ClipId) {
    let (mut editor, _events) = Editor::new_project("Captions");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/talk.mp4",
        MediaTime::from_seconds(6),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();

    // 200 peaks a second, six seconds.
    let peaks = (0..1_200)
        .map(|i| {
            let loud = i < 400 || (600..1_000).contains(&i);
            let level: i8 = if loud { 90 } else { 1 };
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

fn captions(editor: &Editor) -> Vec<(i64, i64, String)> {
    editor
        .active_sequence()
        .unwrap()
        .text_tracks
        .iter()
        .flat_map(|track| track.clips())
        .map(|clip| {
            (
                clip.timeline.start.ticks() / 960,
                clip.timeline.end.ticks() / 960,
                clip.text.clone(),
            )
        })
        .collect()
}

#[test]
fn each_phrase_becomes_an_empty_caption() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, sound) = setup(dir.path());

    bettercut_ui::context_menu::time_captions(&mut editor, &mut state, sound);

    assert_eq!(
        captions(&editor),
        vec![(0, 2_000, String::new()), (3_000, 5_000, String::new())],
        "expected one empty caption per phrase"
    );
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("2 empty captions")),
        "the user was not told what happened: {:?}",
        state.status
    );
}

/// Timing replaces whatever is on the lane rather than piling a second set on
/// top of it. Captions from an earlier pass — or from a subtitle file — are
/// cleared, and are one undo away.
#[test]
fn timing_replaces_what_was_on_the_lane() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, sound) = setup(dir.path());

    // A caption from somewhere else, at a time no phrase is at.
    editor
        .replace_captions(
            vec![bettercut_editor_core::captions::CaptionSegment::new(
                TimelineTime::from_millis(5_200),
                TimelineTime::from_millis(5_800),
                "from a subtitle file",
            )],
            "Import",
        )
        .unwrap();

    bettercut_ui::context_menu::time_captions(&mut editor, &mut state, sound);

    assert_eq!(
        captions(&editor),
        vec![(0, 2_000, String::new()), (3_000, 5_000, String::new())],
        "the old caption was left behind"
    );
}

/// A blank caption is still a caption on the timeline — but it says nothing, so
/// exporting a subtitle file from it has nothing to write.
#[test]
fn empty_captions_export_as_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, sound) = setup(dir.path());
    bettercut_ui::context_menu::time_captions(&mut editor, &mut state, sound);

    assert!(
        editor.caption_segments().is_empty(),
        "captions with no words were written out as blank subtitles"
    );
}

#[test]
fn a_clip_with_no_speech_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _state, sound) = setup(dir.path());

    // A waveform store holding a clip that never gets above a whisper.
    let quiet = Arc::new(CacheStore::new(CacheLayout::new(dir.path()), u64::MAX));
    let media = editor.audio_clip(sound).unwrap().media_id;
    Waveform::new(200, vec![Peak { min: -1, max: 1 }; 1_200])
        .unwrap()
        .write(&quiet.layout().waveform_file(media))
        .unwrap();
    let mut state = UiState::default();
    state.waveforms.attach(quiet);

    bettercut_ui::context_menu::time_captions(&mut editor, &mut state, sound);

    assert!(captions(&editor).is_empty());
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("No speech")),
        "{:?}",
        state.status
    );
}
