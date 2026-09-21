//! J and L cuts: one edge of one half of a linked pair
//! (`bettercut_editor_core::split_edit`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError, TrimEdge};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Two eight-second shots with their own sound, butted together, each placed
/// from the middle of a long file so there is footage to reach either way.
fn two_shots() -> (Editor, Vec<[ClipId; 2]>) {
    let (mut editor, _events) = Editor::new_project("Split edits");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/scene.mp4",
        MediaTime::from_seconds(60),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);

    let pairs: Vec<[ClipId; 2]> = (0..2)
        .map(|_| {
            let placed = editor
                .place_media_range(
                    media,
                    Some((MediaTime::from_seconds(20), MediaTime::from_seconds(28))),
                )
                .unwrap();
            [placed[0], placed[1]]
        })
        .collect();
    (editor, pairs)
}

fn span(editor: &Editor, clip: ClipId) -> (i64, i64) {
    let range = editor
        .active_sequence()
        .unwrap()
        .clip_span(clip)
        .unwrap()
        .timeline;
    (range.start.ticks(), range.end.ticks())
}

#[test]
fn the_setup_is_two_straight_cuts() {
    let (editor, pairs) = two_shots();
    assert_eq!(span(&editor, pairs[0][0]), (0, seconds(8).ticks()));
    assert_eq!(span(&editor, pairs[0][0]), span(&editor, pairs[0][1]));
    assert_eq!(
        editor.split_edit_offset(pairs[0][0], TrimEdge::End),
        Some(TimelineTime::ZERO),
        "a straight cut is no offset"
    );
}

/// An L cut between two shots that touch: the sound cut rolls, both pictures
/// stay, and the first shot's sound hangs over the second shot's picture.
#[test]
fn an_l_cut_leaves_both_pictures_where_they_were() {
    let (mut editor, pairs) = two_shots();
    let [picture, sound] = pairs[0];
    let pictures = (span(&editor, picture), span(&editor, pairs[1][0]));

    editor
        .roll_sound_cut(picture, seconds(2))
        .expect("an L cut");

    assert_eq!(
        (span(&editor, picture), span(&editor, pairs[1][0])),
        pictures,
        "a picture moved"
    );
    assert_eq!(span(&editor, sound), (0, seconds(10).ticks()));
    assert_eq!(
        span(&editor, pairs[1][1]),
        (seconds(10).ticks(), seconds(16).ticks()),
        "the next sound should have given up exactly what this one took"
    );
    assert_eq!(editor.undo_label().as_deref(), Some("L Cut"));
    assert_eq!(
        editor.split_edit_offset(picture, TrimEdge::End),
        Some(seconds(2))
    );
    // Which is a J cut for the shot after it, read from the other side.
    assert_eq!(
        editor.split_edit_offset(pairs[1][0], TrimEdge::Start),
        Some(seconds(-2))
    );
}

/// Rolling the other way brings the next shot's sound in early.
#[test]
fn rolling_the_sound_cut_back_makes_a_j_cut() {
    let (mut editor, pairs) = two_shots();

    editor
        .roll_sound_cut(pairs[0][0], seconds(-2))
        .expect("a J cut");

    assert_eq!(span(&editor, pairs[0][1]), (0, seconds(6).ticks()));
    assert_eq!(
        span(&editor, pairs[1][1]),
        (seconds(6).ticks(), seconds(16).ticks())
    );
    assert_eq!(editor.undo_label().as_deref(), Some("J Cut"));
    assert_eq!(
        editor.split_edit_offset(pairs[1][0], TrimEdge::Start),
        Some(seconds(2))
    );
}

/// And a roll of nothing is nothing: no step, no change.
#[test]
fn rolling_the_sound_cut_nowhere_is_no_edit() {
    let (mut editor, pairs) = two_shots();
    let steps = editor.undo_label();
    editor
        .roll_sound_cut(pairs[0][0], TimelineTime::ZERO)
        .expect("ok");
    assert_eq!(editor.undo_label(), steps);
}

/// Leading a sound into room that is free: the second shot's sound arrives
/// before its picture, once the sound in front of it has given way.
#[test]
fn a_sound_can_be_led_into_free_room() {
    let (mut editor, pairs) = two_shots();
    let [picture, sound] = pairs[1];

    // The sound in front of it has to give way first, or there is nowhere to
    // reach back into.
    editor
        .lead_sound(pairs[0][0], TrimEdge::End, seconds(-2))
        .expect("shorten the first sound");
    editor
        .lead_sound(picture, TrimEdge::Start, seconds(2))
        .expect("a J cut");

    assert_eq!(
        span(&editor, picture),
        (seconds(8).ticks(), seconds(16).ticks())
    );
    assert_eq!(
        span(&editor, sound),
        (seconds(6).ticks(), seconds(16).ticks())
    );
    assert_eq!(editor.undo_label().as_deref(), Some("J Cut"));
    assert_eq!(
        editor.split_edit_offset(picture, TrimEdge::Start),
        Some(seconds(2))
    );
}

/// The pair is still a pair: a split edit changes one edge, not the link.
#[test]
fn a_split_edit_keeps_the_two_linked() {
    let (mut editor, pairs) = two_shots();
    let [picture, sound] = pairs[0];
    editor
        .roll_sound_cut(picture, seconds(2))
        .expect("an L cut");

    let mut linked = editor.linked_with(picture);
    linked.sort();
    let mut expected = vec![picture, sound];
    expected.sort();
    assert_eq!(linked, expected, "the split edit broke the link");

    // And they still move together, offset and all.
    let track = editor.track_of(picture).unwrap();
    editor
        .move_clip(track, track, picture, seconds(20))
        .expect("moved");
    assert_eq!(
        span(&editor, picture),
        (seconds(20).ticks(), seconds(28).ticks())
    );
    assert_eq!(
        span(&editor, sound),
        (seconds(20).ticks(), seconds(30).ticks())
    );
}

#[test]
fn straightening_puts_the_cut_back() {
    let (mut editor, pairs) = two_shots();
    let [picture, sound] = pairs[0];
    let before = span(&editor, sound);

    editor
        .roll_sound_cut(picture, seconds(2))
        .expect("an L cut");
    editor
        .straighten_cut(picture, TrimEdge::End)
        .expect("straightened");

    assert_eq!(span(&editor, sound), before);
    assert_eq!(
        editor.split_edit_offset(picture, TrimEdge::End),
        Some(TimelineTime::ZERO)
    );
    // And straightening a cut that is already straight is nothing at all.
    let steps = editor.undo_label();
    editor.straighten_cut(picture, TrimEdge::End).expect("ok");
    assert_eq!(editor.undo_label(), steps, "an empty edit made a step");
}

/// One undo step, and it puts the sound back where it was.
#[test]
fn one_undo_takes_the_split_edit_off() {
    let (mut editor, pairs) = two_shots();
    let [picture, sound] = pairs[0];
    let before = (span(&editor, sound), span(&editor, pairs[1][1]));

    editor
        .roll_sound_cut(picture, seconds(2))
        .expect("an L cut");
    editor.undo().expect("undo");

    assert_eq!(
        (span(&editor, sound), span(&editor, pairs[1][1])),
        before,
        "both sides of the rolled cut should come back"
    );
}

/// The sound cannot reach past the end of its own file.
#[test]
fn a_split_edit_stops_at_the_end_of_the_footage() {
    let (mut editor, pairs) = two_shots();
    let [picture, sound] = pairs[1];
    let before = span(&editor, sound);

    // The file has 32 seconds left past the out point, but the clip in front
    // is what actually stops it here — reach far enough and it is the file.
    let err = editor
        .lead_sound(picture, TrimEdge::End, seconds(600))
        .unwrap_err();
    assert!(!matches!(err, EditorError::NothingLinked), "{err}");
    assert_eq!(span(&editor, sound), before, "a refused trim moved it");
}

/// And it cannot be pushed into the sound beside it.
#[test]
fn a_split_edit_stops_at_the_clip_beside_it() {
    let (mut editor, pairs) = two_shots();
    let [picture, sound] = pairs[0];
    let before = span(&editor, sound);

    let err = editor
        .lead_sound(picture, TrimEdge::End, seconds(2))
        .unwrap_err();
    assert!(!matches!(err, EditorError::NothingLinked), "{err}");
    assert_eq!(span(&editor, sound), before);
    // Which is what `roll_sound_cut` is for: it takes the room from the sound
    // that is in the way rather than being refused by it.
    editor.roll_sound_cut(picture, seconds(2)).expect("a roll");
}

/// A shot with no sound has no split edit to make, and says so rather than
/// quietly doing nothing.
#[test]
fn a_shot_without_sound_is_refused() {
    let (mut editor, _events) = Editor::new_project("Silent");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/silent.mp4",
        MediaTime::from_seconds(30),
    ));
    let clip = editor.place_media(media).unwrap()[0];

    let err = editor
        .lead_sound(clip, TrimEdge::End, seconds(1))
        .unwrap_err();
    assert!(matches!(err, EditorError::NothingLinked), "{err}");
    assert_eq!(editor.split_edit_offset(clip, TrimEdge::End), None);
}
