//! Ducking music under a voice, end to end (§20a.4, §24).
//!
//! The shape of the dip is tested in `playback`; this is the part that decides
//! *what it ducks under* — every other clip playing over it — and that the
//! result lands on the clip as an undoable envelope.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use bettercut_cache::{CacheLayout, CacheStore, Peak, Waveform};
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AnimatedParameter, AudioClip, SourceRange};
use bettercut_ui::UiState;

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Twenty seconds of peaks: loud where `speaking` says so.
fn waveform(speaking: impl Fn(i64) -> bool) -> Waveform {
    let peaks = (0..4_000)
        .map(|i| {
            let level: i8 = if speaking(i * 5) { 90 } else { 1 };
            Peak {
                min: -level,
                max: level,
            }
        })
        .collect();
    Waveform::new(200, peaks).unwrap()
}

/// Music on A1 from zero, a voice on A2 talking from 5 s to 9 s.
///
/// Returns the editor, the state with both waveforms cached, and the music
/// clip's id.
fn setup(dir: &std::path::Path) -> (Editor, UiState, ClipId) {
    let (mut editor, _events) = Editor::new_project("Ducking");
    let cache = Arc::new(CacheStore::new(CacheLayout::new(dir), u64::MAX));

    let mut music_asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/bed.wav",
        MediaTime::from_seconds(20),
    );
    music_asset.audio_codec = Some("pcm".to_owned());
    let music_media = editor.import_media(music_asset);
    // Music plays throughout, never silent.
    waveform(|_| true)
        .write(&cache.layout().waveform_file(music_media))
        .unwrap();

    let mut voice_asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/voice.wav",
        MediaTime::from_seconds(20),
    );
    voice_asset.audio_codec = Some("pcm".to_owned());
    let voice_media = editor.import_media(voice_asset);
    // Talking from five seconds to nine.
    waveform(|ms| (5_000..9_000).contains(&ms))
        .write(&cache.layout().waveform_file(voice_media))
        .unwrap();

    let music = editor.place_media(music_media).unwrap()[0];

    // The voice on a second track, so both play at once.
    editor.add_audio_track("A2").unwrap();
    let sequence = editor.active_sequence().unwrap();
    let second = sequence.audio_tracks[1].id;
    let voice = AudioClip::new(
        voice_media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(20)).unwrap(),
    )
    .unwrap();
    editor.add_clip(second, voice.into()).unwrap();

    let mut state = UiState::default();
    state.waveforms.attach(cache);
    (editor, state, music)
}

fn gain_at(editor: &Editor, clip: ClipId, at: TimelineTime) -> f32 {
    editor.audio_clip(clip).unwrap().gain_at(at)
}

#[test]
fn the_music_dips_where_the_voice_is() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, music) = setup(dir.path());

    bettercut_ui::context_menu::duck_under_voice(&mut editor, &mut state, music);

    assert_eq!(gain_at(&editor, music, seconds(2)), 1.0, "before the voice");
    assert_eq!(gain_at(&editor, music, seconds(7)), 0.25, "under the voice");
    assert_eq!(gain_at(&editor, music, seconds(12)), 1.0, "after the voice");
    // The drop is a ramp, not a step: halfway through the 150 ms attack.
    let mid = gain_at(&editor, music, TimelineTime::from_millis(4_925));
    assert!(
        mid > 0.25 && mid < 1.0,
        "the music slammed down instead of ramping: {mid}"
    );
}

/// One shape, one step — and clearing it puts the clip back.
#[test]
fn ducking_is_one_undo_step_and_can_be_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, music) = setup(dir.path());
    let before = editor.undo_depth();

    bettercut_ui::context_menu::duck_under_voice(&mut editor, &mut state, music);
    assert_eq!(editor.undo_depth(), before + 1);

    editor.undo().unwrap();
    assert!(
        !editor
            .audio_clip(music)
            .unwrap()
            .keyframes
            .is_animated(AnimatedParameter::Gain),
        "undo left part of the envelope behind"
    );
}

/// A clip with nothing playing over it has nothing to duck under, and says so
/// rather than writing a flat envelope that does nothing.
#[test]
fn nothing_playing_over_it_is_said_plainly() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, music) = setup(dir.path());

    // Move the voice out from under the music entirely.
    let sequence = editor.active_sequence().unwrap();
    let voice = sequence.audio_tracks[1].clips()[0].id;
    let track = sequence.audio_tracks[1].id;
    editor.move_clip(track, track, voice, seconds(40)).unwrap();

    bettercut_ui::context_menu::duck_under_voice(&mut editor, &mut state, music);

    assert!(
        !editor
            .audio_clip(music)
            .unwrap()
            .keyframes
            .is_animated(AnimatedParameter::Gain),
        "an envelope was written with nothing to duck under"
    );
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("Nothing is speaking")),
        "{:?}",
        state.status
    );
}

/// A clip does not duck under itself: its own sound is not the voice.
#[test]
fn a_clip_does_not_duck_under_itself() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, music) = setup(dir.path());

    // Remove the voice track's clip, leaving only the music — which is loud
    // throughout, so ducking under itself would silence it completely.
    let sequence = editor.active_sequence().unwrap();
    let voice = sequence.audio_tracks[1].clips()[0].id;
    let track = sequence.audio_tracks[1].id;
    editor.remove_clip(track, voice).unwrap();

    bettercut_ui::context_menu::duck_under_voice(&mut editor, &mut state, music);

    assert_eq!(
        gain_at(&editor, music, seconds(7)),
        1.0,
        "the music ducked under its own sound"
    );
}
