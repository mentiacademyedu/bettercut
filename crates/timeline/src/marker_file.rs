//! Markers as a file another program reads: a CSV for a spreadsheet, an EDL
//! for another editor.
//!
//! Markers are notes — "fix this", "client hates this", every beat of the
//! song — and notes get handed on: to a spreadsheet that tracks what is
//! done, or to the editor that finishes the job. A CSV is what a spreadsheet
//! opens; a CMX 3600 EDL with locator lines is what Resolve, Premiere and
//! Avid read markers from. Both carry the same four things: where, for how
//! long, what colour, and what it says.
//!
//! Times are written as the sequence *calls* them — from its start timecode
//! (§9 keeps positions from zero inside) — as `HH:MM:SS:FF`, counting frames
//! at the sequence's rate. Non-drop throughout: a marker list is not a
//! broadcast master, and drop-frame arithmetic would put every mark a few
//! frames from where the person sees it on the ruler.

use std::path::Path;

use bettercut_foundation::{FrameRate, TICKS_PER_SECOND, TimelineTime};

use crate::clip::ColorLabel;
use crate::marker::Marker;

/// Which file to write, from its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerFileFormat {
    /// One line a marker, with a header row.
    Csv,
    /// CMX 3600, one event a marker, with the locator lines other editors
    /// read markers from.
    Edl,
}

impl MarkerFileFormat {
    /// `.edl` is an EDL; anything else is a CSV, which is the one a person
    /// can always open.
    pub fn of(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("edl") => Self::Edl,
            _ => Self::Csv,
        }
    }
}

/// `HH:MM:SS:FF` at `rate`, non-drop: the frame number within its second,
/// at the rate's whole-number name (29.97 counts to 30).
pub fn frame_timecode(time: TimelineTime, rate: FrameRate) -> String {
    let nominal = rate.as_f64().round().max(1.0) as i64;
    let ticks = time.ticks().max(0);
    let seconds = ticks / TICKS_PER_SECOND;
    let within = ticks - seconds * TICKS_PER_SECOND;
    let frame = (within * nominal / TICKS_PER_SECOND).min(nominal - 1);
    let (s, minutes) = (seconds % 60, seconds / 60);
    let (m, h) = (minutes % 60, minutes / 60);
    format!("{h:02}:{m:02}:{s:02}:{frame:02}")
}

/// How many whole frames `span` covers at `rate`, one at least: a marker on
/// an instant is still one frame long to an editor that reads it back.
fn span_frames(span: TimelineTime, rate: FrameRate) -> i64 {
    let nominal = rate.as_f64().round().max(1.0);
    ((span.ticks().max(0) as f64 / TICKS_PER_SECOND as f64) * nominal).round() as i64
}

/// A CSV field: quoted when it holds a comma, a quote or a line break, with
/// quotes doubled, as every spreadsheet reads them.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

fn seconds(time: TimelineTime) -> String {
    format!(
        "{:.3}",
        time.ticks().max(0) as f64 / TICKS_PER_SECOND as f64
    )
}

/// The markers as CSV: a header, then `number,timecode,seconds,duration,
/// colour,name` a line, timecodes from `start`.
pub fn markers_csv(markers: &[Marker], rate: FrameRate, start: TimelineTime) -> String {
    let mut out = String::from("number,timecode,seconds,duration,colour,name\r\n");
    for (index, marker) in markers.iter().enumerate() {
        let called = start + marker.time;
        out.push_str(&format!(
            "{},{},{},{},{},{}\r\n",
            index + 1,
            frame_timecode(called, rate),
            seconds(called),
            seconds(marker.span),
            marker.color.name(),
            csv_field(&marker.label),
        ));
    }
    out
}

/// One CSV line into its fields: quotes hold a comma, and a doubled quote
/// is a quote — the way [`markers_csv`] wrote them and spreadsheets do.
fn csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    fields.push(field);
    fields
}

/// A time as a CSV may carry it: `HH:MM:SS:FF` (frames at `rate`),
/// `HH:MM:SS.mmm`, `MM:SS`, or plain seconds.
pub fn parse_timecode(text: &str, rate: FrameRate) -> Option<TimelineTime> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let whole = |part: &str| part.trim().parse::<u32>().ok().map(i64::from);
    let secs = |part: &str| {
        let s: f64 = part.trim().parse().ok()?;
        (s.is_finite() && s >= 0.0).then(|| (s * TICKS_PER_SECOND as f64).round() as i64)
    };
    let parts: Vec<&str> = text.split(':').collect();
    let ticks = match parts.as_slice() {
        [s] => secs(s)?,
        [m, s] => whole(m)? * 60 * TICKS_PER_SECOND + secs(s)?,
        [h, m, s] => (whole(h)? * 3600 + whole(m)? * 60) * TICKS_PER_SECOND + secs(s)?,
        [h, m, s, f] => {
            let nominal = rate.as_f64().round().max(1.0);
            (whole(h)? * 3600 + whole(m)? * 60 + whole(s)?) * TICKS_PER_SECOND
                + (whole(f)? as f64 * TICKS_PER_SECOND as f64 / nominal).round() as i64
        }
        _ => return None,
    };
    Some(TimelineTime::from_ticks(ticks))
}

/// Where each thing is in a CSV's columns, if it is there at all.
struct Columns {
    time: Option<usize>,
    seconds: Option<usize>,
    duration: Option<usize>,
    colour: Option<usize>,
    name: Option<usize>,
}

/// Markers from a CSV: the one [`markers_csv`] writes, or any with a header
/// naming a `timecode` or `seconds` column — `duration`, `colour`/`color`
/// and `name`/`label` are read when present. Without a header the columns
/// are taken in the exported order. Times are as the sequence *calls* them,
/// so `start` comes off; a line with no time it can read is skipped.
pub fn parse_csv(text: &str, rate: FrameRate, start: TimelineTime) -> Vec<Marker> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(first) = lines.next() else {
        return Vec::new();
    };
    let head: Vec<String> = csv_fields(first)
        .into_iter()
        .map(|f| f.trim().to_ascii_lowercase())
        .collect();
    let column = |names: &[&str]| head.iter().position(|h| names.contains(&h.as_str()));
    let (columns, rows): (Columns, Vec<&str>) =
        if column(&["timecode", "time", "seconds", "start"]).is_some() {
            (
                Columns {
                    time: column(&["timecode", "time", "start"]),
                    seconds: column(&["seconds"]),
                    duration: column(&["duration", "length"]),
                    colour: column(&["colour", "color"]),
                    name: column(&["name", "label", "note", "comment"]),
                },
                lines.collect(),
            )
        } else {
            // No header: the exported order, the first line included.
            (
                Columns {
                    time: Some(1),
                    seconds: Some(2),
                    duration: Some(3),
                    colour: Some(4),
                    name: Some(5),
                },
                std::iter::once(first).chain(lines).collect(),
            )
        };
    let Columns {
        time: time_col,
        seconds: seconds_col,
        duration: duration_col,
        colour: colour_col,
        name: name_col,
    } = columns;

    let field = |fields: &[String], at: Option<usize>| -> Option<String> {
        at.and_then(|i| fields.get(i)).map(|f| f.trim().to_owned())
    };
    rows.into_iter()
        .filter_map(|line| {
            let fields = csv_fields(line);
            let called = field(&fields, seconds_col)
                .and_then(|s| parse_timecode(&s, rate))
                .or_else(|| field(&fields, time_col).and_then(|t| parse_timecode(&t, rate)))?;
            let time = TimelineTime::from_ticks((called - start).ticks().max(0));
            let span = field(&fields, duration_col)
                .and_then(|d| parse_timecode(&d, rate))
                .unwrap_or(TimelineTime::ZERO);
            let color = field(&fields, colour_col)
                .and_then(|c| ColorLabel::from_name(&c))
                .unwrap_or(ColorLabel::None);
            let label = field(&fields, name_col).unwrap_or_default();
            Some(Marker {
                label,
                color,
                ..Marker::over(time, span)
            })
        })
        .collect()
}

/// The colour as Resolve names its marker colours.
fn resolve_colour(label: ColorLabel) -> &'static str {
    match label {
        ColorLabel::None | ColorLabel::Blue => "Blue",
        ColorLabel::Red => "Red",
        ColorLabel::Orange => "Sand",
        ColorLabel::Yellow => "Yellow",
        ColorLabel::Green => "Green",
        ColorLabel::Purple => "Purple",
        ColorLabel::Pink => "Pink",
    }
}

/// The colour as an Avid locator names it.
fn locator_colour(label: ColorLabel) -> &'static str {
    match label {
        ColorLabel::None | ColorLabel::Green => "green",
        ColorLabel::Red => "red",
        ColorLabel::Orange | ColorLabel::Yellow => "yellow",
        ColorLabel::Blue => "blue",
        ColorLabel::Purple | ColorLabel::Pink => "magenta",
    }
}

/// The markers as a CMX 3600 EDL: one event a marker, each carrying a Resolve
/// marker line (`|C:… |M:… |D:…`) and an Avid locator (`* LOC:`), so either
/// reads them back as markers. `title` is the sequence's name.
pub fn markers_edl(
    markers: &[Marker],
    rate: FrameRate,
    start: TimelineTime,
    title: &str,
) -> String {
    let mut out = format!("TITLE: {}\r\nFCM: NON-DROP FRAME\r\n\r\n", title.trim());
    for (index, marker) in markers.iter().enumerate() {
        let called = start + marker.time;
        let frames = span_frames(marker.span, rate).max(1);
        let end = called
            + TimelineTime::from_ticks(
                frames * TICKS_PER_SECOND / rate.as_f64().round().max(1.0) as i64,
            );
        let (from, to) = (frame_timecode(called, rate), frame_timecode(end, rate));
        // A marker's name is one line: an EDL is line-based.
        let name = marker.label.replace(['\r', '\n'], " ");
        out.push_str(&format!(
            "{:03}  001      V     C        {from} {to} {from} {to}\r\n",
            index + 1
        ));
        out.push_str(&format!(
            " |C:ResolveColor{} |M:{name} |D:{frames}\r\n",
            resolve_colour(marker.color)
        ));
        out.push_str(&format!(
            "* LOC: {from} {}  {name}\r\n\r\n",
            locator_colour(marker.color).to_uppercase()
        ));
    }
    out
}

/// The file's text in `format`.
pub fn marker_file(
    markers: &[Marker],
    format: MarkerFileFormat,
    rate: FrameRate,
    start: TimelineTime,
    title: &str,
) -> String {
    match format {
        MarkerFileFormat::Csv => markers_csv(markers, rate, start),
        MarkerFileFormat::Edl => markers_edl(markers, rate, start, title),
    }
}
