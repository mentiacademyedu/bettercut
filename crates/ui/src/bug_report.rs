//! Details for a bug report, put on the clipboard: which build, which
//! system, and the shape of the project — lanes, clips, sizes — so a report
//! from a tester says what it needs to without a round of questions.
//!
//! Nothing is sent anywhere, and nothing about the footage is included: no
//! file names, no paths, no words from titles or captions.

use bettercut_editor_core::Editor;

/// The report text for the open project.
pub fn details(editor: &Editor) -> String {
    let mut lines = vec![
        format!("bettercut {}", env!("CARGO_PKG_VERSION")),
        format!(
            "system: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    ];
    let project = editor.project();
    lines.push(format!(
        "project: {} sequence(s), {} media file(s)",
        project.sequences.len(),
        project.media.len()
    ));
    if let Some(sequence) = editor.active_sequence() {
        let rate = sequence.frame_rate.as_rational();
        lines.push(format!(
            "sequence: {}x{} at {}/{} fps, {} long",
            sequence.resolution.width,
            sequence.resolution.height,
            rate.num(),
            rate.den(),
            sequence.duration().format_timecode()
        ));
        lines.push(format!(
            "lanes: {} picture, {} sound, {} title; {} clip(s)",
            sequence.video_tracks.len(),
            sequence.audio_tracks.len(),
            sequence.text_tracks.len(),
            sequence.clip_count()
        ));
    }
    lines.push(format!("undo steps: {}", editor.undo_depth()));
    lines.push(String::new());
    lines.push("What happened:".to_owned());
    lines.push("What I expected:".to_owned());
    lines.join("\n")
}
