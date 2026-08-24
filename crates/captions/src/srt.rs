//! SubRip (`.srt`) — read and written (§27, Milestone 10).
//!
//! The format has no specification, only thirty years of practice, so this
//! parses what tools actually emit rather than what a grammar would allow:
//!
//! ```text
//! 1
//! 00:00:01,000 --> 00:00:04,000
//! The first line
//! and its second
//!
//! 2
//! ...
//! ```
//!
//! What is tolerated, because real files contain all of it: a UTF-8 BOM, CRLF
//! line endings, a missing or wrong index, extra blank lines, `.` instead of
//! `,` before the milliseconds, missing milliseconds altogether, spaces around
//! the arrow, and WebVTT-style positioning junk after the end time.
//!
//! What is *not* tolerated is a timing line that cannot be read at all: that
//! block is skipped and counted, because silently dropping a third of someone's
//! subtitles with no word said about it is worse than a warning (§50).

use bettercut_foundation::TimelineTime;

use crate::error::CaptionError;
use crate::segment::{CaptionSegment, tidy};

/// Ticks in one millisecond. §9's timebase is 960,000 per second, so a
/// millisecond is exactly 960 ticks and nothing rounds.
const TICKS_PER_MS: i64 = 960;

/// What a parse produced, including what it could not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub segments: Vec<CaptionSegment>,
    /// Blocks whose timing line could not be read. Reported rather than
    /// swallowed — see the module note.
    pub skipped: usize,
}

/// Read a SubRip file.
///
/// The result is [`tidy`]'d: sorted, non-overlapping, no blanks — the state a
/// track will accept.
pub fn parse(text: &str) -> Result<Parsed, CaptionError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut segments = Vec::new();
    let mut skipped = 0;

    // Split on blank lines. `\r` is stripped per line rather than up front, so
    // a file with mixed endings — which happens when two tools have edited it —
    // parses the same as a consistent one.
    let mut lines = text.lines().map(str::trim_end).peekable();
    while lines.peek().is_some() {
        // Skip the blank run between blocks.
        while lines.peek().is_some_and(|l| l.trim().is_empty()) {
            lines.next();
        }
        let mut block: Vec<&str> = Vec::new();
        while let Some(line) = lines.peek() {
            if line.trim().is_empty() {
                break;
            }
            block.push(lines.next().unwrap_or_default());
        }
        if block.is_empty() {
            continue;
        }

        // The index line is optional: plenty of tools omit it, and a wrong one
        // is more common than a missing one. The timing line is found by
        // looking for the arrow, not by counting lines.
        let Some(arrow_at) = block.iter().position(|l| l.contains("-->")) else {
            skipped += 1;
            continue;
        };
        let Some((start, end)) = parse_timing(block[arrow_at]) else {
            skipped += 1;
            continue;
        };

        let body = block[arrow_at + 1..].join("\n");
        segments.push(CaptionSegment::new(start, end, body.trim()));
    }

    if segments.is_empty() && skipped == 0 {
        return Err(CaptionError::NoCaptions);
    }

    Ok(Parsed {
        segments: tidy(segments),
        skipped,
    })
}

/// `00:00:01,000 --> 00:00:04,000`, plus everything tools put after it.
fn parse_timing(line: &str) -> Option<(TimelineTime, TimelineTime)> {
    let (left, right) = line.split_once("-->")?;
    let start = parse_timecode(left.trim())?;
    // WebVTT cue settings — `line:90%`, `align:middle` — ride along after the
    // end time in files converted from that format. Everything past the first
    // space is positioning, which this does not implement.
    let right = right.trim();
    let end = parse_timecode(right.split_whitespace().next()?)?;
    Some((start, end))
}

/// `HH:MM:SS,mmm`, and the four other things this turns up as.
///
/// Accepts `,` or `.` before the milliseconds, a missing milliseconds field,
/// and a missing hours field (`MM:SS,mmm`), which is how WebVTT writes anything
/// under an hour.
fn parse_timecode(text: &str) -> Option<TimelineTime> {
    let (clock, fraction) = match text.split_once([',', '.']) {
        Some((clock, fraction)) => (clock, fraction),
        None => (text, "0"),
    };

    let parts: Vec<&str> = clock.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [h, m, s] => (
            h.parse::<i64>().ok()?,
            m.parse::<i64>().ok()?,
            s.parse::<i64>().ok()?,
        ),
        [m, s] => (0, m.parse::<i64>().ok()?, s.parse::<i64>().ok()?),
        _ => return None,
    };
    if !(0..60).contains(&minutes) || !(0..60).contains(&seconds) || hours < 0 {
        return None;
    }

    // Padded rather than parsed as a fraction: `,5` means 500 ms, not 5.
    let digits: String = fraction
        .chars()
        .filter(char::is_ascii_digit)
        .take(3)
        .collect();
    if digits.is_empty() && !fraction.is_empty() {
        return None;
    }
    let millis: i64 = format!("{digits:0<3}").parse().ok()?;

    let total_ms = ((hours * 60 + minutes) * 60 + seconds) * 1000 + millis;
    Some(TimelineTime::from_ticks(total_ms * TICKS_PER_MS))
}

/// Write a SubRip file.
///
/// Millisecond precision, which is all the format has. §9's timebase divides
/// exactly into milliseconds, so what comes back from a round trip is what went
/// in — up to the truncation the format itself imposes, which a test pins down.
pub fn write(segments: &[CaptionSegment]) -> String {
    let mut out = String::new();
    for (index, segment) in segments.iter().enumerate() {
        out.push_str(&format!("{}\n", index + 1));
        out.push_str(&format!(
            "{} --> {}\n",
            format_timecode(segment.start),
            format_timecode(segment.end)
        ));
        out.push_str(segment.text.trim());
        out.push_str("\n\n");
    }
    out
}

fn format_timecode(time: TimelineTime) -> String {
    let total_ms = time.ticks().max(0) / TICKS_PER_MS;
    let millis = total_ms % 1000;
    let total_seconds = total_ms / 1000;
    let seconds = total_seconds % 60;
    let minutes = (total_seconds / 60) % 60;
    let hours = total_seconds / 3600;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: i64) -> TimelineTime {
        TimelineTime::from_millis(n)
    }

    #[test]
    fn a_plain_file_parses() {
        let parsed = parse(
            "1\n00:00:01,000 --> 00:00:04,000\nThe first line\n\n\
             2\n00:00:05,500 --> 00:00:07,250\nThe second\n",
        )
        .expect("parse");

        assert_eq!(parsed.skipped, 0);
        assert_eq!(parsed.segments.len(), 2);
        assert_eq!(parsed.segments[0].start, ms(1000));
        assert_eq!(parsed.segments[0].end, ms(4000));
        assert_eq!(parsed.segments[0].text, "The first line");
        assert_eq!(parsed.segments[1].start, ms(5500));
        assert_eq!(parsed.segments[1].end, ms(7250));
    }

    /// A caption spanning two lines is one caption. Joining them with a space
    /// instead would lose the line break the author chose.
    #[test]
    fn a_multi_line_cue_keeps_its_break() {
        let parsed = parse("1\n00:00:01,000 --> 00:00:04,000\nfirst\nsecond\n").expect("parse");
        assert_eq!(parsed.segments[0].text, "first\nsecond");
    }

    /// Every one of these turns up in files people actually have.
    #[test]
    fn the_usual_deviations_all_parse() {
        let awkward = "\u{feff}\r\n\
             00:00:01.000 --> 00:00:04.000\r\n\
             dots instead of commas, and no index\r\n\
             \r\n\
             \r\n\
             7\r\n\
             00:00:05,000  -->  00:00:06,000  line:90%\r\n\
             wrong index, extra spaces, vtt settings\r\n";

        let parsed = parse(awkward).expect("parse");
        assert_eq!(parsed.skipped, 0, "a real-world file was rejected");
        assert_eq!(parsed.segments.len(), 2);
        assert_eq!(parsed.segments[0].start, ms(1000));
        assert_eq!(parsed.segments[1].end, ms(6000));
    }

    /// `MM:SS` with no hours is how anything under an hour is written in VTT,
    /// and the two formats get mixed constantly.
    #[test]
    fn a_missing_hours_field_parses() {
        let parsed = parse("1\n01:30,000 --> 01:35,000\nshort form\n").expect("parse");
        assert_eq!(parsed.segments[0].start, ms(90_000));
    }

    /// A short fraction is padded, not parsed as a decimal: `,5` is half a
    /// second, and reading it as five milliseconds would put the caption a
    /// frame early instead of half a second early.
    #[test]
    fn a_short_fraction_is_padded_not_scaled() {
        let parsed = parse("1\n00:00:01,5 --> 00:00:02,25\npadded\n").expect("parse");
        assert_eq!(parsed.segments[0].start, ms(1500));
        assert_eq!(parsed.segments[0].end, ms(2250));
    }

    /// §50: an unreadable block is counted, not silently dropped.
    #[test]
    fn an_unreadable_block_is_reported() {
        let parsed = parse(
            "1\nnot a timing line at all\nsome words\n\n\
             2\n00:00:05,000 --> 00:00:06,000\nfine\n",
        )
        .expect("parse");

        assert_eq!(parsed.skipped, 1);
        assert_eq!(parsed.segments.len(), 1);
    }

    #[test]
    fn a_file_with_no_captions_is_an_error() {
        assert!(matches!(parse(""), Err(CaptionError::NoCaptions)));
        assert!(matches!(parse("   \n\n  "), Err(CaptionError::NoCaptions)));
    }

    /// Timing that is out of range is a corrupt line, not a caption at 90
    /// minutes past the minute.
    #[test]
    fn out_of_range_fields_are_refused() {
        assert!(parse_timecode("00:90:00,000").is_none());
        assert!(parse_timecode("00:00:75,000").is_none());
        assert!(parse_timecode("gibberish").is_none());
    }

    #[test]
    fn writing_produces_the_conventional_shape() {
        let written = write(&[
            CaptionSegment::new(ms(1000), ms(4000), "first"),
            CaptionSegment::new(ms(5500), ms(7250), "second"),
        ]);

        assert_eq!(
            written,
            "1\n00:00:01,000 --> 00:00:04,000\nfirst\n\n\
             2\n00:00:05,500 --> 00:00:07,250\nsecond\n\n"
        );
    }

    #[test]
    fn a_round_trip_preserves_everything_the_format_can_hold() {
        let original = vec![
            CaptionSegment::new(ms(0), ms(1234), "at the very start"),
            CaptionSegment::new(ms(3_661_789), ms(3_662_000), "past an hour"),
            CaptionSegment::new(ms(3_700_000), ms(3_701_000), "two\nlines"),
        ];

        let parsed = parse(&write(&original)).expect("parse");
        assert_eq!(parsed.segments, original);
    }

    /// Sub-millisecond timing does not survive, because the format has no room
    /// for it. Saying so here beats someone discovering it from a drifting
    /// subtitle.
    #[test]
    fn sub_millisecond_timing_is_truncated_by_the_format() {
        let original = CaptionSegment::new(
            TimelineTime::from_ticks(1_000_500),
            TimelineTime::from_ticks(2_000_500),
            "odd ticks",
        );
        let parsed = parse(&write(&[original])).expect("parse");

        assert_eq!(parsed.segments[0].start.ticks(), 1_000_320);
        assert!(
            parsed.segments[0].start.ticks() % TICKS_PER_MS == 0,
            "a millisecond boundary is the finest this format holds"
        );
    }
}
