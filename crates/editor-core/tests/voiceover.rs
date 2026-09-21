//! Voiceovers (`Editor::add_voiceover`): a recorded WAV placed where the
//! playhead was, on a sound lane with room for it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::voiceover::{next_voiceover_file, voiceover_folder};

/// A second of quiet tone at 48 kHz, mono, as a 16-bit WAV.
fn write_tone(path: &Path, seconds: f32) {
    let rate = 48_000_u32;
    let frames = (rate as f32 * seconds) as usize;
    let mut out = Vec::new();
    let data_len = (frames * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

#[test]
fn a_take_lands_at_the_playhead_and_a_second_gets_its_own_lane() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.wav");
    let second = dir.path().join("two.wav");
    write_tone(&first, 1.0);
    write_tone(&second, 1.0);

    let (mut editor, _events) = Editor::new_project("Voice");
    let lanes = editor.active_sequence().unwrap().audio_tracks.len();
    let at = TimelineTime::from_seconds(2);

    let clip = editor.add_voiceover(&first, at).unwrap();
    let placed = editor.audio_clip(clip).unwrap();
    assert_eq!(placed.timeline.start, at);
    let length = placed.timeline.duration().ticks();
    assert!(
        (length - TimelineTime::from_seconds(1).ticks()).abs() < 20_000,
        "{length}"
    );
    assert_eq!(editor.active_sequence().unwrap().audio_tracks.len(), lanes);

    // Over the first take: a lane of its own, undone as one step.
    let depth = editor.undo_depth();
    let again = editor.add_voiceover(&second, at).unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.audio_tracks.len(), lanes + 1);
    assert_eq!(sequence.audio_tracks[lanes].name, "Voiceover");
    assert!(sequence.audio_tracks[lanes].get(again).is_some());
    assert_eq!(editor.undo_depth(), depth + 1);
    editor.undo().unwrap();
    assert_eq!(editor.active_sequence().unwrap().audio_tracks.len(), lanes);
}

#[test]
fn recordings_are_kept_beside_the_project_under_fresh_names() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("Trip.vproj");
    let folder = voiceover_folder(Some(&project));
    assert_eq!(folder, dir.path().join("Trip voiceovers"));

    std::fs::create_dir_all(&folder).unwrap();
    let first = next_voiceover_file(&folder);
    assert_eq!(first, folder.join("Voiceover 1.wav"));
    std::fs::write(&first, b"taken").unwrap();
    assert_eq!(next_voiceover_file(&folder), folder.join("Voiceover 2.wav"));

    assert!(voiceover_folder(None).ends_with("voiceovers"));
}
