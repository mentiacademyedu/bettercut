//! The contact sheet's arithmetic: where the frames are taken, how big the
//! sheet is, and what shrinking a frame does
//! (`bettercut_export::contact_sheet`).
//!
//! The rendering itself is `render_still`'s, covered where that is; what is
//! worth testing here is the layout, which is where a sheet goes wrong.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_export::{MAX_TILES, SheetSettings, shrink};
use bettercut_foundation::TimelineTime;
use bettercut_timeline::{Resolution, TimelineRange};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn sheet(tiles: u32, columns: u32) -> SheetSettings {
    let range = TimelineRange::new(TimelineTime::ZERO, seconds(60)).expect("non-empty");
    SheetSettings {
        tiles,
        columns,
        ..SheetSettings::of(std::path::PathBuf::from("sheet.png"), range)
    }
}

/// Frames come from the middle of each slice: the start of a cut is usually
/// black, and a sheet of black tiles says nothing.
#[test]
fn the_frames_are_taken_from_the_middle_of_each_slice() {
    let instants = sheet(6, 3).instants();
    assert_eq!(instants.len(), 6);

    let secs: Vec<i64> = instants.iter().map(|at| at.ticks() / 960_000).collect();
    assert_eq!(secs, vec![5, 15, 25, 35, 45, 55]);
    assert!(instants[0] > TimelineTime::ZERO, "the first tile is black");
    assert!(
        *instants.last().unwrap() < seconds(60),
        "the last is past the end"
    );
}

/// A sheet of one is the middle of the whole stretch.
#[test]
fn one_tile_is_the_middle_of_the_cut() {
    let instants = sheet(1, 1).instants();
    assert_eq!(instants, vec![seconds(30)]);
}

/// A sheet of a marked range covers that range and nothing else.
#[test]
fn the_frames_stay_inside_the_range() {
    let range = TimelineRange::new(seconds(20), seconds(30)).expect("non-empty");
    let settings = SheetSettings {
        tiles: 8,
        ..SheetSettings::of(std::path::PathBuf::from("sheet.png"), range)
    };
    for at in settings.instants() {
        assert!(at > range.start && at < range.end, "{at:?}");
    }
}

/// Rows follow from the count, the tiles keep the frame's shape, and the
/// gutters are counted in.
#[test]
fn the_sheet_is_as_big_as_its_grid() {
    let settings = SheetSettings {
        tile_width: 100,
        ..sheet(7, 3)
    };
    let (size, tile_height) = settings.layout(Resolution::new(1920, 1080));

    // Seven tiles, three across, is three rows — the last one half empty.
    assert_eq!(tile_height, 56, "a 16:9 tile 100 wide");
    assert_eq!(size.width, 3 * 100 + 4 * 6);
    assert_eq!(size.height, 3 * 56 + 4 * 6);
}

/// A vertical cut makes a vertical tile: the shape is the sequence's.
#[test]
fn a_vertical_frame_makes_a_vertical_tile() {
    let (_, tile_height) = SheetSettings {
        tile_width: 90,
        ..sheet(4, 2)
    }
    .layout(Resolution::new(1080, 1920));
    assert_eq!(tile_height, 160);
}

/// More tiles than a sheet can hold is held to the limit rather than making a
/// picture nobody can open.
#[test]
fn the_tile_count_is_held_to_the_limit() {
    let settings = sheet(500, 8);
    assert_eq!(settings.instants().len(), MAX_TILES as usize);
    let (size, _) = settings.layout(Resolution::new(1920, 1080));
    assert!(size.width < 10_000 && size.height < 10_000, "{size:?}");
}

/// Shrinking averages: a frame of one colour stays that colour, and a frame
/// split down the middle keeps its halves.
#[test]
fn shrinking_averages_rather_than_picking() {
    let from = Resolution::new(8, 8);
    let flat: Vec<u8> = (0..64).flat_map(|_| [200, 100, 50, 255]).collect();
    let small = shrink(&flat, from, Resolution::new(2, 2));
    assert_eq!(small.len(), 2 * 2 * 4);
    assert!(small.chunks_exact(4).all(|p| p == [200, 100, 50, 255]));

    // Black left, white right: the halves survive, and nothing in between is
    // invented at this size.
    let split: Vec<u8> = (0..8)
        .flat_map(|_| {
            (0..8).flat_map(|x| {
                let v = if x < 4 { 0 } else { 255 };
                [v, v, v, 255]
            })
        })
        .collect();
    let small = shrink(&split, from, Resolution::new(2, 1));
    assert_eq!(small[0], 0, "the left half went grey");
    assert_eq!(small[4], 255, "the right half went grey");
}

/// A tile bigger than nothing, from a frame of nothing, is not a panic.
#[test]
fn an_empty_frame_shrinks_to_something() {
    let small = shrink(&[], Resolution::new(0, 0), Resolution::new(4, 4));
    assert_eq!(small.len(), 4 * 4 * 4);
}
