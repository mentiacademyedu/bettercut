//! Compound clips: several clips folded into a sequence of their own and
//! played as one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn secs(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Three 2-second shots with sound, end to end: 0–2, 2–4, 4–6.
fn three_shots() -> (Editor, [ClipId; 3]) {
    let (mut editor, _events) = Editor::new_project("Compound");
    let place = |editor: &mut Editor, name: &str| {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(2),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        editor.place_media(media).unwrap()[0]
    };
    let a = place(&mut editor, "a");
    let b = place(&mut editor, "b");
    let c = place(&mut editor, "c");
    (editor, [a, b, c])
}

fn video_clips(editor: &Editor) -> Vec<(TimelineTime, TimelineTime)> {
    editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|clip| (clip.timeline.start, clip.timeline.end))
        .collect()
}

#[test]
fn folding_two_shots_leaves_one_clip_covering_both() {
    let (mut editor, [a, b, _]) = three_shots();
    let sound = |editor: &Editor, clip: ClipId| {
        editor
            .linked_with(clip)
            .into_iter()
            .find(|c| *c != clip)
            .unwrap()
    };
    let (a_sound, b_sound) = (sound(&editor, a), sound(&editor, b));
    let depth = editor.undo_depth();

    let compound = editor
        .make_compound(&[a, b, a_sound, b_sound], "Opening")
        .unwrap();

    // One clip where two were, and the third shot untouched after it.
    assert_eq!(
        video_clips(&editor),
        vec![(secs(0), secs(4)), (secs(4), secs(6))]
    );
    assert_eq!(
        editor.active_sequence().unwrap().audio_tracks[0]
            .clips()
            .len(),
        1,
        "the folded sound should have gone inside"
    );
    assert_eq!(editor.undo_depth(), depth + 1);

    // And the sequence it made holds them, starting at zero.
    let inner_id = editor.compound_of(compound).expect("a compound clip");
    let inner = editor.project().sequence(inner_id).unwrap();
    assert_eq!(inner.name, "Opening");
    let inside: Vec<_> = inner.video_tracks[0]
        .clips()
        .iter()
        .map(|clip| (clip.timeline.start, clip.timeline.end))
        .collect();
    assert_eq!(inside, vec![(secs(0), secs(2)), (secs(2), secs(4))]);
    assert_eq!(inner.audio_tracks[0].clips().len(), 2);

    editor.undo().unwrap();
    assert_eq!(
        video_clips(&editor),
        vec![(secs(0), secs(2)), (secs(2), secs(4)), (secs(4), secs(6))]
    );
}

#[test]
fn a_compound_can_be_opened_to_edit_what_is_inside() {
    let (mut editor, [a, b, _]) = three_shots();
    let compound = editor.make_compound(&[a, b], "Opening").unwrap();
    let inner = editor.compound_of(compound).unwrap();

    assert!(editor.open_compound(compound));
    assert_eq!(editor.active_sequence().unwrap().id, inner);
    // And a shot that is not a compound is not a door to anywhere.
    let ordinary = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    assert_eq!(editor.compound_of(ordinary), None);
    assert!(!editor.open_compound(ordinary));
}

#[test]
fn folding_nothing_says_so() {
    let (mut editor, _) = three_shots();
    assert!(matches!(
        editor.make_compound(&[], "Nothing"),
        Err(EditorError::NothingToCompound)
    ));
}

/// A compound is a clip like any other: it trims, moves and grades, and what
/// is inside it is untouched by any of that.
#[test]
fn a_compound_trims_like_a_clip() {
    let (mut editor, [a, b, _]) = three_shots();
    let compound = editor.make_compound(&[a, b], "Opening").unwrap();
    let track = editor.track_of(compound).unwrap();
    editor
        .trim_clip(
            track,
            compound,
            bettercut_editor_core::TrimEdge::Start,
            secs(1),
        )
        .unwrap();

    let clip = editor.video_clip(compound).unwrap();
    assert_eq!(clip.timeline.start, secs(1));
    // Trimming a second off the front reads a second further into it.
    assert_eq!(clip.source.start, MediaTime::from_seconds(1));
    let inner = editor.compound_of(compound).unwrap();
    assert_eq!(
        editor.project().sequence(inner).unwrap().video_tracks[0]
            .clips()
            .len(),
        2
    );
}
