//! Shuffle selected clips into a random order (`Editor::shuffle_clips`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Shots of 2, 3 and 4 seconds with their sound, one after another, and a
/// one-second gap before the last.
fn montage() -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Montage");
    let mut clips = Vec::new();
    for (length, name) in [(2, "a"), (3, "b"), (4, "c")] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(length),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        clips.push(editor.place_media(media).unwrap()[0]);
    }
    let last = clips[2];
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor.move_clip(track, track, last, seconds(6)).unwrap();
    (editor, clips)
}

fn start(editor: &Editor, clip: ClipId) -> TimelineTime {
    editor
        .active_sequence()
        .unwrap()
        .clip_span(clip)
        .unwrap()
        .timeline
        .start
}

#[test]
fn the_clips_trade_places_keeping_the_gaps_and_their_sound() {
    let (mut editor, clips) = montage();
    let depth = editor.undo_depth();

    let order = editor.shuffle_clips(&clips, 42).unwrap();
    assert_eq!(order.len(), 3);
    assert_ne!(order, clips, "the order did not change");

    // Laid end to end from zero in the new order, with the one-second gap
    // still after the second place.
    let mut at = seconds(0);
    for (slot, clip) in order.iter().enumerate() {
        assert_eq!(start(&editor, *clip), at, "slot {slot}");
        let length = editor.video_clip(*clip).unwrap().timeline.duration();
        at = at + length + if slot == 1 { seconds(1) } else { seconds(0) };
    }
    assert_eq!(at, seconds(10), "the stretch changed length");

    // Each shot's sound came with it.
    for clip in &order {
        let partner = editor
            .linked_with(*clip)
            .into_iter()
            .find(|c| c != clip)
            .unwrap();
        assert_eq!(start(&editor, partner), start(&editor, *clip));
    }
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(start(&editor, clips[0]), seconds(0));
    assert_eq!(start(&editor, clips[1]), seconds(2));
    assert_eq!(start(&editor, clips[2]), seconds(6));
}

#[test]
fn the_same_seed_gives_the_same_order() {
    let (mut one, clips) = montage();
    let first = one.shuffle_clips(&clips, 7).unwrap();
    one.undo().unwrap();
    let again = one.shuffle_clips(&clips, 7).unwrap();
    assert_eq!(first, again);
}

#[test]
fn a_shuffle_needs_neighbours_on_one_lane() {
    let (mut editor, clips) = montage();
    assert!(matches!(
        editor.shuffle_clips(&clips[..1], 1),
        Err(EditorError::NothingToShuffle)
    ));
    // The first and last, with the middle one left out between them.
    assert!(matches!(
        editor.shuffle_clips(&[clips[0], clips[2]], 1),
        Err(EditorError::NothingToShuffle)
    ));
    assert_eq!(start(&editor, clips[0]), seconds(0));
}

#[test]
fn reversing_puts_the_last_shot_first_and_keeps_the_gaps() {
    let (mut editor, clips) = montage();
    let depth = editor.undo_depth();
    let order = editor.reverse_clip_order(&clips).unwrap();
    assert_eq!(order, [clips[2], clips[1], clips[0]]);
    // c (4 s) at 0, b (3 s) at 4, then the one-second gap, a at 8.
    assert_eq!(start(&editor, clips[2]), seconds(0));
    assert_eq!(start(&editor, clips[1]), seconds(4));
    assert_eq!(start(&editor, clips[0]), seconds(8));
    assert_eq!(editor.undo_depth(), depth + 1);
}
