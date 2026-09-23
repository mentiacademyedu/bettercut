//! Markers as a file (`bettercut_timeline::marker_file`): a CSV a spreadsheet
//! opens, an EDL another editor reads markers from, timecodes as the sequence
//! calls them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use bettercut_foundation::{FrameRate, TimelineTime};
use bettercut_timeline::{
    ColorLabel, Marker, MarkerFileFormat, frame_timecode, marker_file, markers_csv, markers_edl,
    parse_csv, parse_timecode,
};

fn markers() -> Vec<Marker> {
    vec![
        Marker {
            label: "Intro".to_owned(),
            color: ColorLabel::Green,
            ..Marker::at(TimelineTime::ZERO)
        },
        Marker {
            label: "Fix, the \"drone\" shot".to_owned(),
            color: ColorLabel::Red,
            ..Marker::over(
                TimelineTime::from_millis(1_500),
                TimelineTime::from_seconds(2),
            )
        },
    ]
}

/// Frames count at the rate's whole-number name, non-drop, from the start
/// timecode.
#[test]
fn timecodes_count_frames_from_the_start_timecode() {
    let one_and_a_half = TimelineTime::from_millis(1_500);
    assert_eq!(
        frame_timecode(one_and_a_half, FrameRate::PAL_25),
        "00:00:01:12"
    );
    assert_eq!(
        frame_timecode(one_and_a_half, FrameRate::NTSC_29_97),
        "00:00:01:15"
    );
    let hour = TimelineTime::from_seconds(3_600);
    assert_eq!(
        frame_timecode(hour + one_and_a_half, FrameRate::FILM_24),
        "01:00:01:12"
    );
    // The last frame of a second never rounds up into the next one.
    let almost = TimelineTime::from_ticks(TimelineTime::from_seconds(1).ticks() - 1);
    assert_eq!(frame_timecode(almost, FrameRate::PAL_25), "00:00:00:24");
}

/// A header, one line a marker, and a name with a comma and quotes quoted
/// the way a spreadsheet reads it.
#[test]
fn the_csv_has_a_header_and_quotes_what_needs_it() {
    let csv = markers_csv(
        &markers(),
        FrameRate::PAL_25,
        TimelineTime::from_seconds(3_600),
    );
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "number,timecode,seconds,duration,colour,name");
    assert_eq!(lines[1], "1,01:00:00:00,3600.000,0.000,Green,Intro");
    assert_eq!(
        lines[2],
        "2,01:00:01:12,3601.500,2.000,Red,\"Fix, the \"\"drone\"\" shot\""
    );
    assert_eq!(lines.len(), 3);
}

/// An EDL: the title, non-drop, one event a marker with the marker line and
/// locator both editors read, the span as its frame count.
#[test]
fn the_edl_carries_an_event_a_marker_with_its_locator() {
    let edl = markers_edl(&markers(), FrameRate::PAL_25, TimelineTime::ZERO, "Trip");
    let lines: Vec<&str> = edl.lines().collect();
    assert_eq!(lines[0], "TITLE: Trip");
    assert_eq!(lines[1], "FCM: NON-DROP FRAME");
    assert!(
        lines.contains(
            &"001  001      V     C        00:00:00:00 00:00:00:01 00:00:00:00 00:00:00:01"
        ),
        "{edl}"
    );
    assert!(
        lines.contains(&" |C:ResolveColorGreen |M:Intro |D:1"),
        "{edl}"
    );
    assert!(lines.contains(&"* LOC: 00:00:00:00 GREEN  Intro"), "{edl}");
    // The ranged marker: two seconds at 25 is fifty frames, ending at 3.5 s.
    assert!(
        lines.contains(
            &"002  001      V     C        00:00:01:12 00:00:03:12 00:00:01:12 00:00:03:12"
        ),
        "{edl}"
    );
    assert!(
        lines.contains(&" |C:ResolveColorRed |M:Fix, the \"drone\" shot |D:50"),
        "{edl}"
    );
}

/// What was written is what is read back, start timecode and all.
#[test]
fn the_csv_reads_back_as_the_markers_it_was_written_from() {
    let start = TimelineTime::from_seconds(3_600);
    let csv = markers_csv(&markers(), FrameRate::PAL_25, start);
    assert_eq!(parse_csv(&csv, FrameRate::PAL_25, start), markers());
}

/// A spreadsheet's own list: columns in any order, headed by name, a time
/// as a timecode, colours however capitalised, a quoted name with a comma.
#[test]
fn a_headed_csv_in_any_column_order_is_read() {
    let text = "Name,Color,Timecode\r\n\"Fix, this\",red,00:01:02:12\r\nQuiet bit,GREEN,2:03\r\n,,\r\nno time here,,\r\n";
    let read = parse_csv(text, FrameRate::PAL_25, TimelineTime::ZERO);
    assert_eq!(read.len(), 2, "{read:?}");
    assert_eq!(read[0].label, "Fix, this");
    assert_eq!(read[0].color, ColorLabel::Red);
    assert_eq!(read[0].time, TimelineTime::from_millis(62_480));
    assert_eq!(read[1].time, TimelineTime::from_seconds(123));
    assert_eq!(read[1].color, ColorLabel::Green);
    assert_eq!(read[1].span, TimelineTime::ZERO);
}

/// No header: the exported order is assumed, and seconds win over the
/// timecode when both are there.
#[test]
fn a_bare_csv_takes_the_exported_order() {
    let text = "1,00:00:05:00,5.5,2,Blue,Note\n";
    let read = parse_csv(text, FrameRate::PAL_25, TimelineTime::ZERO);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].time, TimelineTime::from_millis(5_500));
    assert_eq!(read[0].span, TimelineTime::from_seconds(2));
    assert_eq!(read[0].color, ColorLabel::Blue);
    assert_eq!(read[0].label, "Note");
    assert!(parse_csv("", FrameRate::PAL_25, TimelineTime::ZERO).is_empty());
}

#[test]
fn timecodes_are_read_in_every_short_form() {
    let rate = FrameRate::PAL_25;
    assert_eq!(
        parse_timecode("45", rate),
        Some(TimelineTime::from_seconds(45))
    );
    assert_eq!(
        parse_timecode("2:03.5", rate),
        Some(TimelineTime::from_millis(123_500))
    );
    assert_eq!(
        parse_timecode("1:02:03", rate),
        Some(TimelineTime::from_seconds(3_723))
    );
    assert_eq!(
        parse_timecode("01:00:01:12", rate),
        Some(TimelineTime::from_seconds(3_601) + TimelineTime::from_millis(480))
    );
    assert_eq!(parse_timecode("abc", rate), None);
    assert_eq!(parse_timecode("", rate), None);
}

/// The extension picks the format; anything unknown is the CSV.
#[test]
fn the_extension_picks_the_format() {
    assert_eq!(
        MarkerFileFormat::of(Path::new("notes.EDL")),
        MarkerFileFormat::Edl
    );
    assert_eq!(
        MarkerFileFormat::of(Path::new("notes.csv")),
        MarkerFileFormat::Csv
    );
    assert_eq!(
        MarkerFileFormat::of(Path::new("notes.txt")),
        MarkerFileFormat::Csv
    );
    let text = marker_file(
        &markers(),
        MarkerFileFormat::Edl,
        FrameRate::PAL_25,
        TimelineTime::ZERO,
        "Trip",
    );
    assert!(text.starts_with("TITLE: Trip"));
}
