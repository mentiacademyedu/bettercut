//! The cut as a row of cards, and dragging one of them somewhere else
//! (`bettercut_editor_core::storyboard`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Three four-second shots with their own sound, butted together, each from a
/// differently named file so the cards can be told apart.
fn three_shots() -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Storyboard");
    let clips = ["wide", "mid", "close"]
        .into_iter()
        .map(|name| {
            let mut asset = MediaAsset::new(
                MediaKind::Video,
                format!("C:/media/{name}.mp4"),
                MediaTime::from_seconds(4),
            );
            asset.audio_codec = Some("aac".to_owned());
            let media = editor.import_media(asset);
            editor.place_media(media).unwrap()[0]
        })
        .collect();
    (editor, clips)
}

fn names(editor: &Editor) -> Vec<String> {
    editor
        .storyboard()
        .into_iter()
        .map(|card| card.name)
        .collect()
}

fn spans(editor: &Editor) -> Vec<(i64, i64)> {
    editor
        .storyboard()
        .into_iter()
        .map(|card| {
            (
                card.start.ticks() / 960_000,
                card.duration.ticks() / 960_000,
            )
        })
        .collect()
}

#[test]
fn the_cards_are_the_spine_of_the_edit_in_order() {
    let (editor, clips) = three_shots();
    let cards = editor.storyboard();

    assert_eq!(cards.len(), 3);
    assert_eq!(
        cards.iter().map(|card| card.clip).collect::<Vec<_>>(),
        clips
    );
    assert_eq!(names(&editor), ["wide.mp4", "mid.mp4", "close.mp4"]);
    assert_eq!(spans(&editor), [(0, 4), (4, 4), (8, 4)]);
    assert!(
        cards[0].media.is_some(),
        "a card should have a file to show"
    );
    assert_eq!(cards[1].poster, MediaTime::ZERO);
}

/// The one thing a storyboard is for: putting the third shot first.
#[test]
fn a_card_can_be_dragged_to_the_front() {
    let (mut editor, _clips) = three_shots();

    editor.reorder_storyboard(2, 0).expect("reordered");

    assert_eq!(names(&editor), ["close.mp4", "wide.mp4", "mid.mp4"]);
    assert_eq!(
        spans(&editor),
        [(0, 4), (4, 4), (8, 4)],
        "the cut kept its shape; only the order changed"
    );
    assert_eq!(editor.undo_label().as_deref(), Some("Reorder Clips"));
}

#[test]
fn a_card_can_be_dragged_to_the_end() {
    let (mut editor, _clips) = three_shots();
    editor.reorder_storyboard(0, 2).expect("reordered");
    assert_eq!(names(&editor), ["mid.mp4", "close.mp4", "wide.mp4"]);
}

/// The sound comes with the picture (§12), and one undo puts it all back.
#[test]
fn the_sound_travels_and_one_undo_takes_it_back() {
    let (mut editor, clips) = three_shots();
    let sound_of = |editor: &Editor, clip: ClipId| {
        let sound = editor.sound_of(clip).expect("linked sound");
        editor
            .active_sequence()
            .unwrap()
            .clip_span(sound)
            .unwrap()
            .timeline
    };
    let before = sound_of(&editor, clips[2]);
    assert_eq!(before.start, seconds(8));

    editor.reorder_storyboard(2, 0).expect("reordered");
    assert_eq!(
        sound_of(&editor, clips[2]).start,
        TimelineTime::ZERO,
        "the sound stayed behind"
    );

    editor.undo().expect("undo");
    assert_eq!(names(&editor), ["wide.mp4", "mid.mp4", "close.mp4"]);
    assert_eq!(sound_of(&editor, clips[2]), before);
}

/// Gaps between the shots are part of the cut's shape, so they stay where they
/// are while the shots move through them.
#[test]
fn the_gaps_stay_where_they_are() {
    let (mut editor, clips) = three_shots();
    // Push the last shot two seconds later, leaving a gap in front of it.
    let track = editor.track_of(clips[2]).unwrap();
    editor
        .move_clip(track, track, clips[2], seconds(10))
        .expect("moved");
    assert_eq!(spans(&editor), [(0, 4), (4, 4), (10, 4)]);

    editor.reorder_storyboard(2, 0).expect("reordered");

    assert_eq!(names(&editor), ["close.mp4", "wide.mp4", "mid.mp4"]);
    assert_eq!(
        spans(&editor),
        [(0, 4), (4, 4), (10, 4)],
        "the slots should be where they were"
    );
}

#[test]
fn putting_a_card_where_it_already_is_changes_nothing() {
    let (mut editor, _clips) = three_shots();
    let steps = editor.undo_label();

    editor.reorder_storyboard(1, 1).expect("ok");
    editor.reorder_storyboard(9, 0).expect("ok");

    assert_eq!(names(&editor), ["wide.mp4", "mid.mp4", "close.mp4"]);
    assert_eq!(editor.undo_label(), steps, "an empty edit made a step");
}

#[test]
fn a_cut_with_one_shot_has_nothing_to_reorder() {
    let (mut editor, _events) = Editor::new_project("One");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/only.mp4",
        MediaTime::from_seconds(4),
    ));
    editor.place_media(media).unwrap();

    assert_eq!(editor.storyboard().len(), 1);
    assert!(matches!(
        editor.reorder_storyboard(0, 0).unwrap_err(),
        EditorError::NothingToShuffle
    ));
}

/// An empty project has no spine, and asking for one is not a failure.
#[test]
fn an_empty_cut_has_no_cards() {
    let (editor, _events) = Editor::new_project("Empty");
    assert!(editor.storyboard().is_empty());
    assert!(editor.storyboard_track().is_some(), "but the lane is there");
}
