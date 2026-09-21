//! Text to speech (`bettercut_ui::speech`), with a real Windows voice.
//!
//! Skips itself where there are no voices — a machine without Windows' speech
//! engine is not a failure of this code.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_ui::speech::{synthesize, voices};

#[test]
fn a_title_is_spoken_into_a_voiceover() {
    let Ok(installed) = voices() else {
        eprintln!("no speech engine; skipping");
        return;
    };
    if installed.is_empty() {
        eprintln!("no voices installed; skipping");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("spoken.wav");
    // Quotes, a dollar sign and an accent: none of them may reach the script.
    synthesize(
        "Café's \"grand\" opening costs $5; `echo` nothing.",
        Some(&installed[0]),
        2,
        &path,
    )
    .unwrap();
    assert!(
        std::fs::metadata(&path).unwrap().len() > 10_000,
        "a near-empty file"
    );

    let (mut editor, _events) = Editor::new_project("Spoken");
    let clip = editor
        .add_voiceover(&path, TimelineTime::from_seconds(1))
        .unwrap();
    let placed = editor.audio_clip(clip).unwrap();
    assert_eq!(placed.timeline.start, TimelineTime::from_seconds(1));
    assert!(placed.timeline.duration() > TimelineTime::from_millis(800));
}

#[test]
fn nothing_to_say_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    assert!(synthesize("   ", None, 0, &dir.path().join("x.wav")).is_err());
}
