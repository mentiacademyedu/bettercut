//! Reading and writing caption files (Milestone 10).

use std::path::Path;

use crate::error::CaptionError;
use crate::segment::CaptionSegment;
use crate::srt::{self, Parsed};
use crate::vtt;

/// Which caption format a file is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    SubRip,
    WebVtt,
    /// Plain text, a line per caption with the time it is said: a
    /// transcript to read, not a subtitle file. Written only.
    Transcript,
}

impl Format {
    /// Guess from the extension, falling back to SubRip.
    ///
    /// The extension is a hint rather than the answer — the parsers overlap
    /// enough that a `.srt` containing WebVTT reads correctly anyway — so
    /// getting this wrong costs nothing. What it decides for certain is which
    /// format a *write* produces.
    pub fn of(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("vtt") => Self::WebVtt,
            Some("txt") => Self::Transcript,
            _ => Self::SubRip,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::SubRip => "srt",
            Self::WebVtt => "vtt",
            Self::Transcript => "txt",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SubRip => "SubRip (.srt)",
            Self::WebVtt => "WebVTT (.vtt)",
            Self::Transcript => "Transcript (.txt)",
        }
    }
}

/// Read a caption file, choosing the parser by what is inside it.
///
/// By content rather than by extension, because a `.srt` that is really WebVTT
/// is common enough — tools rename rather than convert — and the header says so
/// unambiguously.
pub fn read(path: &Path) -> Result<Parsed, CaptionError> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8(bytes).map_err(|_| CaptionError::NotText)?;

    let looks_like_vtt = text
        .strip_prefix('\u{feff}')
        .unwrap_or(&text)
        .trim_start()
        .starts_with("WEBVTT");

    if looks_like_vtt {
        vtt::parse(&text)
    } else {
        srt::parse(&text)
    }
}

/// Write captions in `format`.
pub fn write(path: &Path, segments: &[CaptionSegment], format: Format) -> Result<(), CaptionError> {
    let text = match format {
        Format::SubRip => srt::write(segments),
        Format::WebVtt => vtt::write(segments),
        Format::Transcript => transcript(segments),
    };
    std::fs::write(path, text)?;
    Ok(())
}

/// The captions as reading text: `[m:ss] words` a line, `[h:mm:ss]` past
/// the hour, line breaks inside a caption folded to spaces.
pub fn transcript(segments: &[CaptionSegment]) -> String {
    let mut out = String::new();
    for segment in segments {
        let words = segment
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if words.is_empty() {
            continue;
        }
        let seconds = segment.start.ticks().max(0) / bettercut_foundation::TICKS_PER_SECOND;
        let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
        if h > 0 {
            out.push_str(&format!("[{h}:{m:02}:{s:02}] {words}\n"));
        } else {
            out.push_str(&format!("[{m}:{s:02}] {words}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transcript_is_a_line_per_caption_with_its_time() {
        let segments = vec![
            CaptionSegment::new(ms(1_000), ms(2_000), "Hello\nthere"),
            CaptionSegment::new(ms(65_000), ms(66_000), "   "),
            CaptionSegment::new(ms(3_725_000), ms(3_726_000), "Late"),
        ];
        assert_eq!(
            transcript(&segments),
            "[0:01] Hello there\n[1:02:05] Late\n"
        );
        assert_eq!(Format::of(Path::new("notes.TXT")), Format::Transcript);
    }
    use bettercut_foundation::TimelineTime;

    fn ms(n: i64) -> TimelineTime {
        TimelineTime::from_millis(n)
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("bettercut-caption-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join(name)
    }

    #[test]
    fn the_format_comes_from_the_extension_when_writing() {
        assert_eq!(Format::of(Path::new("a.vtt")), Format::WebVtt);
        assert_eq!(Format::of(Path::new("a.VTT")), Format::WebVtt);
        assert_eq!(Format::of(Path::new("a.srt")), Format::SubRip);
        assert_eq!(Format::of(Path::new("a")), Format::SubRip);
    }

    /// Tools rename rather than convert, so a `.srt` holding WebVTT is common.
    /// Reading by content gets it right where the extension would not.
    #[test]
    fn a_mislabelled_file_is_read_by_its_contents() {
        let path = temp("mislabelled.srt");
        std::fs::write(
            &path,
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<v Bob>hello</v>\n",
        )
        .expect("write");

        let parsed = read(&path).expect("read");
        assert_eq!(parsed.segments.len(), 1);
        assert_eq!(
            parsed.segments[0].text, "hello",
            "read as SubRip, the voice tag would still be in the text"
        );
    }

    #[test]
    fn a_file_round_trips_through_disk() {
        for (name, format) in [("round.srt", Format::SubRip), ("round.vtt", Format::WebVtt)] {
            let path = temp(name);
            let segments = vec![
                CaptionSegment::new(ms(1000), ms(2000), "first"),
                CaptionSegment::new(ms(3000), ms(4000), "second"),
            ];

            write(&path, &segments, format).expect("write");
            assert_eq!(read(&path).expect("read").segments, segments, "{name}");
        }
    }

    #[test]
    fn a_binary_file_is_refused_rather_than_mangled() {
        let path = temp("binary.srt");
        std::fs::write(&path, [0xff_u8, 0xfe, 0x00, 0x01]).expect("write");
        assert!(matches!(read(&path), Err(CaptionError::NotText)));
    }

    #[test]
    fn a_missing_file_reports_io_rather_than_no_captions() {
        let missing = temp("does-not-exist.srt");
        let _ = std::fs::remove_file(&missing);
        assert!(matches!(read(&missing), Err(CaptionError::Io(_))));
    }
}
