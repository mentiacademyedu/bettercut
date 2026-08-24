//! WebVTT (`.vtt`) — read and written (§27, Milestone 10).
//!
//! The format most transcription tools emit, and the one browsers accept:
//!
//! ```text
//! WEBVTT
//!
//! 00:00:01.000 --> 00:00:04.000
//! The first line
//! ```
//!
//! Structurally close enough to SubRip that the cue parsing is *shared* rather
//! than written twice — the timecodes differ only in using `.` before the
//! milliseconds, which [`crate::srt`] already accepts. What differs is the
//! header, the comment and style blocks, and the inline markup.

use crate::error::CaptionError;
use crate::segment::CaptionSegment;
use crate::srt::{self, Parsed};

/// Read a WebVTT file.
pub fn parse(text: &str) -> Result<Parsed, CaptionError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    // `WEBVTT`, optionally followed by a title on the same line. Not required:
    // a file missing it is still readable, and refusing would help nobody.
    let body = text
        .strip_prefix("WEBVTT")
        .map_or(text, |rest| rest.split_once('\n').map_or("", |(_, r)| r));

    // NOTE, STYLE and REGION blocks are metadata, not captions. Left in, the
    // NOTE text would be parsed as a cue with no timing and counted as a
    // skipped block — a warning about a file that is perfectly fine.
    let mut kept = String::with_capacity(body.len());
    let mut in_metadata = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            in_metadata = false;
            kept.push('\n');
            continue;
        }
        if !in_metadata
            && (trimmed.starts_with("NOTE")
                || trimmed.starts_with("STYLE")
                || trimmed.starts_with("REGION"))
        {
            in_metadata = true;
        }
        if !in_metadata {
            kept.push_str(line);
            kept.push('\n');
        }
    }

    let mut parsed = srt::parse(&kept)?;
    for segment in &mut parsed.segments {
        segment.text = strip_markup(&segment.text);
    }
    // Stripping markup can empty a cue that was nothing but a tag.
    parsed.segments.retain(|s| !s.is_blank());
    Ok(parsed)
}

/// Remove WebVTT's inline tags, keeping the words between them.
///
/// `<v Alice>`, `<c.loud>`, `<i>`, `<00:00:01.000>` — voice spans, classes,
/// styling and the karaoke timestamps that word-level tools emit. None of them
/// are rendered yet (§26 draws one style per clip), and leaving them in would
/// put literal angle brackets on screen.
fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0_u32;
    for ch in text.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Write a WebVTT file.
pub fn write(segments: &[CaptionSegment]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for segment in segments {
        out.push_str(&format!(
            "{} --> {}\n",
            timecode(segment.start),
            timecode(segment.end)
        ));
        out.push_str(segment.text.trim());
        out.push_str("\n\n");
    }
    out
}

/// The same clock as SubRip with a `.` in place of the `,`.
fn timecode(time: bettercut_foundation::TimelineTime) -> String {
    srt::write(&[CaptionSegment::new(time, time, "x")])
        .lines()
        .nth(1)
        .and_then(|l| l.split(" --> ").next())
        .unwrap_or("00:00:00,000")
        .replace(',', ".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::TimelineTime;

    fn ms(n: i64) -> TimelineTime {
        TimelineTime::from_millis(n)
    }

    #[test]
    fn a_plain_file_parses() {
        let parsed = parse(
            "WEBVTT\n\n\
             00:00:01.000 --> 00:00:04.000\nThe first line\n\n\
             00:00:05.000 --> 00:00:06.000\nThe second\n",
        )
        .expect("parse");

        assert_eq!(parsed.skipped, 0);
        assert_eq!(parsed.segments.len(), 2);
        assert_eq!(parsed.segments[0].start, ms(1000));
        assert_eq!(parsed.segments[0].text, "The first line");
    }

    /// A file missing the header is still readable. Refusing would help nobody.
    #[test]
    fn a_missing_header_is_tolerated() {
        let parsed = parse("00:00:01.000 --> 00:00:02.000\nwords\n").expect("parse");
        assert_eq!(parsed.segments.len(), 1);
    }

    /// NOTE and STYLE blocks are metadata. Parsed as cues they would be counted
    /// as unreadable and produce a warning about a perfectly good file.
    #[test]
    fn metadata_blocks_are_not_mistaken_for_captions() {
        let parsed = parse(
            "WEBVTT\n\n\
             NOTE\nThis file was generated automatically\nand has two lines of note\n\n\
             STYLE\n::cue { color: yellow }\n\n\
             00:00:01.000 --> 00:00:02.000\nactual words\n",
        )
        .expect("parse");

        assert_eq!(parsed.skipped, 0, "metadata was counted as a broken cue");
        assert_eq!(parsed.segments.len(), 1);
        assert_eq!(parsed.segments[0].text, "actual words");
    }

    /// Inline tags must not reach the screen as angle brackets.
    #[test]
    fn inline_markup_is_stripped() {
        let parsed = parse(
            "WEBVTT\n\n\
             00:00:01.000 --> 00:00:02.000\n\
             <v Alice><c.loud>Hello</c> there</v>\n",
        )
        .expect("parse");

        assert_eq!(parsed.segments[0].text, "Hello there");
    }

    /// Word-level timestamps ride inside the cue text in tools that produce
    /// them. Stripped for now; §27's word timing will read them properly.
    #[test]
    fn karaoke_timestamps_are_stripped() {
        let parsed = parse(
            "WEBVTT\n\n\
             00:00:01.000 --> 00:00:03.000\n\
             <00:00:01.000>One <00:00:02.000>two\n",
        )
        .expect("parse");

        assert_eq!(parsed.segments[0].text, "One two");
    }

    /// A cue that was nothing but a tag has nothing left to show.
    #[test]
    fn a_cue_of_pure_markup_is_dropped() {
        let parsed = parse(
            "WEBVTT\n\n\
             00:00:01.000 --> 00:00:02.000\n<v Alice></v>\n\n\
             00:00:03.000 --> 00:00:04.000\nreal\n",
        )
        .expect("parse");

        assert_eq!(parsed.segments.len(), 1);
        assert_eq!(parsed.segments[0].text, "real");
    }

    #[test]
    fn writing_produces_the_conventional_shape() {
        let written = write(&[CaptionSegment::new(ms(1000), ms(4000), "first")]);
        assert_eq!(
            written,
            "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\nfirst\n\n"
        );
    }

    #[test]
    fn a_round_trip_preserves_the_captions() {
        let original = vec![
            CaptionSegment::new(ms(0), ms(1234), "at the very start"),
            CaptionSegment::new(ms(3_661_789), ms(3_662_000), "past an hour"),
        ];
        let parsed = parse(&write(&original)).expect("parse");
        assert_eq!(parsed.segments, original);
    }

    /// The two formats have to agree, or a file exported as one and reimported
    /// as the other would drift.
    #[test]
    fn the_two_formats_read_the_same_timings() {
        let segments = vec![CaptionSegment::new(ms(3_661_789), ms(3_662_500), "same")];

        let via_srt = srt::parse(&srt::write(&segments)).expect("srt");
        let via_vtt = parse(&write(&segments)).expect("vtt");
        assert_eq!(via_srt.segments, via_vtt.segments);
    }
}
