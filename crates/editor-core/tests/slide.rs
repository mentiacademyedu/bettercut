//! Slide edit (`Editor::slide_clip`): a clip moves and its neighbours give way.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{
    ClipId, MediaTime, TICKS_PER_SECOND, TimelineTime, TrackId,
};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor, EditorError};

fn secs(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Three shots end to end, each playing from ten seconds into a long file so
/// every one of them has room to grow either way.
fn three_shots() -> (Editor, [ClipId; 3]) {
    let (mut editor, _events) = Editor::new_project("Slide");
    let sequence = editor.active_sequence().unwrap();
    let track: TrackId = sequence.video_tracks[0].id;
    let mut ids = Vec::new();
    for (index, name) in ["a", "b", "c"].into_iter().enumerate() {
        let media = editor.import_media(MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(60),
        ));
        let source =
            SourceRange::new(MediaTime::from_seconds(10), MediaTime::from_seconds(14)).unwrap();
        let clip = VideoClip::new(media, secs(index as i64 * 4), source).unwrap();
        ids.push(clip.id);
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .unwrap();
    }
    (editor, [ids[0], ids[1], ids[2]])
}

fn spans(editor: &Editor) -> Vec<(i64, i64)> {
    editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|clip| {
            (
                clip.timeline.start.ticks() / TICKS_PER_SECOND,
                clip.timeline.end.ticks() / TICKS_PER_SECOND,
            )
        })
        .collect()
}

#[test]
fn sliding_moves_the_clip_and_nothing_else() {
    let (mut editor, [_, middle, _]) = three_shots();
    let depth = editor.undo_depth();
    assert_eq!(spans(&editor), vec![(0, 4), (4, 8), (8, 12)]);

    let moved = editor.slide_clip(middle, TICKS_PER_SECOND).unwrap();

    assert_eq!(moved, TICKS_PER_SECOND);
    // The first grew by a second, the middle moved by one, the last is
    // untouched at its end.
    assert_eq!(spans(&editor), vec![(0, 5), (5, 9), (9, 12)]);
    // And the middle clip still plays exactly what it did.
    let clip = editor.video_clip(middle).unwrap();
    assert_eq!(clip.source.start, MediaTime::from_seconds(10));
    assert_eq!(clip.source.end, MediaTime::from_seconds(14));
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(spans(&editor), vec![(0, 4), (4, 8), (8, 12)]);
}

#[test]
fn sliding_the_other_way_works_the_same() {
    let (mut editor, [_, middle, _]) = three_shots();
    editor.slide_clip(middle, -2 * TICKS_PER_SECOND).unwrap();
    assert_eq!(spans(&editor), vec![(0, 2), (2, 6), (6, 12)]);
}

#[test]
fn a_slide_stops_where_a_neighbour_runs_out() {
    let (mut editor, [first, middle, _]) = three_shots();
    let (earliest, latest) = editor.slide_room(middle).unwrap();
    // The first clip must keep a frame of itself, so the middle cannot slide
    // more than a little under four seconds earlier.
    assert!(
        earliest < 0 && earliest > -4 * TICKS_PER_SECOND,
        "{earliest}"
    );
    assert!(latest > 0, "{latest}");

    // Sliding far further than that lands exactly at the limit.
    let moved = editor.slide_clip(middle, -100 * TICKS_PER_SECOND).unwrap();
    assert_eq!(moved, earliest);
    let first_span = editor.video_clip(first).unwrap().timeline;
    assert!(first_span.duration().ticks() > 0);
}

#[test]
fn a_clip_with_nothing_beside_it_has_nothing_to_slide_against() {
    let (mut editor, _events) = Editor::new_project("Slide");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/alone.mp4",
        MediaTime::from_seconds(10),
    ));
    let alone = editor.place_media(media).unwrap()[0];
    assert_eq!(editor.slide_room(alone), Some((0, 0)));
    assert!(matches!(
        editor.slide_clip(alone, TICKS_PER_SECOND),
        Err(EditorError::NoNeighbour) | Ok(0)
    ));
}
