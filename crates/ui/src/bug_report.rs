//! Details for a bug report, put on the clipboard: which build, which
//! system, and the shape of the project — lanes, clips, sizes — so a report
//! from a tester says what it needs to without a round of questions.
//!
//! Nothing is sent anywhere, and nothing about the footage is included: no
//! file names, no paths, no words from titles or captions.

use bettercut_editor_core::Editor;

/// Where bugs are reported: the project's public issue list.
pub const ISSUES_URL: &str = "https://github.com/mentiacademyedu/bettercut/issues";

/// A link that opens a new issue with `title` and `body` already typed in.
/// Nothing is sent until the person reads it and presses Submit themselves.
pub fn new_issue_url(title: &str, body: &str) -> String {
    format!(
        "{ISSUES_URL}/new?title={}&body={}",
        percent_encode(title),
        percent_encode(body)
    )
}

/// The body of a bug report: the questions worth answering, then the
/// details — version, system, project shape, and nothing about the footage.
pub fn issue_body(details: &str) -> String {
    format!(
        "**What happened**\n\n\n**What you expected**\n\n\n**Steps to make it happen again**\n1. \n\n\
         **Details** (from bettercut)\n```\n{details}\n```\n"
    )
}

/// Everything but the characters a URL leaves alone, as `%XX`.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

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
    lines.push("log: attach bettercut.log from %APPDATA%\\bettercut\\logs".to_owned());
    lines.push(String::new());
    lines.push("What happened:".to_owned());
    lines.push("What I expected:".to_owned());
    lines.join("\n")
}

#[cfg(test)]
mod issue_tests {
    use super::*;

    #[test]
    fn a_new_issue_link_carries_its_text_encoded() {
        let url = new_issue_url("Crash: export", "a b\n&c");
        assert!(url.starts_with("https://github.com/mentiacademyedu/bettercut/issues/new?"));
        assert!(url.contains("title=Crash%3A%20export"));
        assert!(url.ends_with("body=a%20b%0A%26c"));
    }
}
