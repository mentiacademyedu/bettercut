//! Zooming the timeline to the selection.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::TimelineRange;
use bettercut_ui::UiState;

/// The whole range is on screen, not crammed against either edge, and as
/// close as the zoom ladder allows — one step in would not fit it.
#[test]
fn a_range_fills_the_lanes_with_room_either_side() {
    let mut state = UiState::default();
    let range = TimelineRange::new(
        TimelineTime::from_seconds(100),
        TimelineTime::from_seconds(162),
    )
    .unwrap();
    let width = 1_000.0;
    state.zoom_to_range(range, width);

    let per_pixel = state.ticks_per_pixel();
    let left = state.scroll_ticks;
    let right = left + width as i64 * per_pixel;
    assert!(
        left < range.start.ticks(),
        "the start is on the left edge or off it"
    );
    assert!(right > range.end.ticks(), "the end is off the right edge");

    let mut tighter = UiState::default();
    tighter.zoom_to_range(range, width);
    if tighter.can_zoom_in() {
        tighter.zoom_in();
        let span = width as i64 * tighter.ticks_per_pixel();
        // The range with its tenth spare either side.
        let padded = range.duration().ticks() * 12 / 10;
        assert!(
            span < padded,
            "a closer zoom would still have fitted the range and its margins"
        );
    }
}

/// The selection's range runs from its first clip's start to its last clip's
/// end, on any track.
#[test]
fn the_selection_spans_its_clips() {
    let (mut editor, _events) = Editor::new_project("Zoom");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    ));
    let first = editor.place_media(media).unwrap()[0];
    editor.place_media(media).unwrap();
    let third = editor.place_media(media).unwrap()[0];

    let mut state = UiState::default();
    assert_eq!(state.selection_range(&editor), None);
    state.selected_clips.extend([first, third]);
    let range = state.selection_range(&editor).unwrap();
    assert_eq!(
        (range.start, range.end),
        (TimelineTime::ZERO, TimelineTime::from_seconds(12))
    );
}
