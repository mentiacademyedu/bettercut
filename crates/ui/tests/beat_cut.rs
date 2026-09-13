//! Cutting a clip on the beat, end to end.
//!
//! Where the beats are is tested in `playback`, and cutting at a list of
//! instants in `editor-core`. This is the joining of the two, and the one piece
//! of judgement that lives only here: how many cuts is too many for one click.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use bettercut_cache::{CacheLayout, CacheStore, Peak, Waveform};
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;

/// A clip of `seconds` with a hit every `beat_ms`, at 200 peaks a second.
fn setup(dir: &std::path::Path, seconds: i64, beat_ms: usize) -> (Editor, UiState, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Beats");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/music.mp4",
        MediaTime::from_seconds(seconds),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();

    let buckets = (seconds as usize) * 200;
    let every = beat_ms / 5;
    let peaks = (0..buckets)
        .map(|i| {
            // A short loud hit on the beat, quiet between.
            let level: i8 = if i % every < 3 { 100 } else { 4 };
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
    (editor, state, placed[0], placed[1])
}

fn clip_count(editor: &Editor) -> (usize, usize) {
    let sequence = editor.active_sequence().unwrap();
    (
        sequence.video_tracks[0].clips().len(),
        sequence.audio_tracks[0].clips().len(),
    )
}

/// Eight seconds at two beats a second: the clip comes back in pieces, and the
/// sound is cut with the picture (§12).
#[test]
fn a_clip_is_cut_on_every_beat() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, picture, sound) = setup(dir.path(), 8, 500);

    bettercut_ui::context_menu::cut_on_beats(&mut editor, &mut state, picture, sound);

    let (video, audio) = clip_count(&editor);
    assert!(video > 8, "only {video} pieces from sixteen beats");
    assert_eq!(video, audio, "§12: the sound was not cut with the picture");
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("Cut into")),
        "{:?}",
        state.status
    );
}

/// §11: one intention, one step. Sixteen cuts is one decision.
#[test]
fn cutting_on_beats_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, picture, sound) = setup(dir.path(), 8, 500);
    let before = editor.undo_depth();

    bettercut_ui::context_menu::cut_on_beats(&mut editor, &mut state, picture, sound);
    assert_eq!(editor.undo_depth(), before + 1);

    editor.undo().unwrap();
    assert_eq!(clip_count(&editor), (1, 1), "undo left it in pieces");
}

/// A whole song of beats is a thousand clips and an undo step nobody can see
/// the end of. The count is offered as a refusal rather than performed as a
/// surprise.
#[test]
fn too_many_beats_is_refused_with_the_count() {
    let dir = tempfile::tempdir().unwrap();
    // Four minutes at four beats a second: far past what one click should do.
    let (mut editor, mut state, picture, sound) = setup(dir.path(), 240, 250);

    bettercut_ui::context_menu::cut_on_beats(&mut editor, &mut state, picture, sound);

    assert_eq!(clip_count(&editor), (1, 1), "it cut anyway");
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("too many to cut")),
        "{:?}",
        state.status
    );
}

/// Sound with no beat in it says so rather than cutting at nothing.
#[test]
fn a_clip_with_no_beat_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _state, picture, sound) = setup(dir.path(), 8, 500);

    // A waveform with no hits at all.
    let cache = Arc::new(CacheStore::new(CacheLayout::new(dir.path()), u64::MAX));
    let media = editor.audio_clip(sound).unwrap().media_id;
    Waveform::new(200, vec![Peak { min: -4, max: 4 }; 1_600])
        .unwrap()
        .write(&cache.layout().waveform_file(media))
        .unwrap();
    let mut state = UiState::default();
    state.waveforms.attach(cache);

    bettercut_ui::context_menu::cut_on_beats(&mut editor, &mut state, picture, sound);

    assert_eq!(clip_count(&editor), (1, 1));
    assert!(
        state
            .status
            .as_ref()
            .is_some_and(|s| s.text.contains("No steady beat")),
        "{:?}",
        state.status
    );
}

/// The beats are found in the music, but the cut lands on the clip the user
/// asked about — a trimmed picture is cut only where it actually is.
#[test]
fn only_the_beats_inside_the_clip_cut_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, mut state, picture, sound) = setup(dir.path(), 8, 500);

    // Trim the pair to its first five seconds. Not shorter: finding a tempo
    // takes about four seconds of sound, so a two-second clip has no beat to
    // detect at all — which is a fact about the detector, not about cutting.
    let track = editor.track_of(picture).unwrap();
    editor
        .trim_clip(
            track,
            picture,
            bettercut_editor_core::TrimEdge::End,
            TimelineTime::from_seconds(5),
        )
        .unwrap();

    bettercut_ui::context_menu::cut_on_beats(&mut editor, &mut state, picture, sound);

    let sequence = editor.active_sequence().unwrap();
    let clips = sequence.video_tracks[0].clips();
    assert!(
        clips.len() > 4,
        "five seconds at two beats a second should be several pieces, got {}",
        clips.len()
    );
    assert!(
        clips
            .iter()
            .all(|clip| clip.timeline.start < TimelineTime::from_seconds(5)),
        "a cut landed past the end of the trimmed clip"
    );
}
