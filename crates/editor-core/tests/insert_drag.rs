//! Insert drag (`Editor::insert_clip_at`): a clip dropped between others
//! pushes the rest along.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn secs(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Three 2-second shots with sound, end to end: 0–2, 2–4, 4–6.
fn three_shots() -> (Editor, [ClipId; 3]) {
    let (mut editor, _events) = Editor::new_project("Insert");
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

fn start(editor: &Editor, clip: ClipId) -> TimelineTime {
    editor
        .video_clip(clip)
        .map(|c| c.timeline.start)
        .or_else(|| editor.audio_clip(clip).map(|c| c.timeline.start))
        .unwrap()
}

fn sound_of(editor: &Editor, clip: ClipId) -> ClipId {
    editor
        .linked_with(clip)
        .into_iter()
        .find(|c| *c != clip)
        .unwrap()
}

#[test]
fn dropping_the_last_shot_between_the_first_two_pushes_the_second_along() {
    let (mut editor, [a, b, c]) = three_shots();
    let track = editor.track_of(c).unwrap();
    let depth = editor.undo_depth();

    editor.insert_clip_at(c, track, secs(2)).unwrap();

    assert_eq!(start(&editor, a), secs(0));
    assert_eq!(start(&editor, c), secs(2));
    assert_eq!(start(&editor, b), secs(4));
    // Each shot's sound went with it.
    assert_eq!(start(&editor, sound_of(&editor, c)), secs(2));
    assert_eq!(start(&editor, sound_of(&editor, b)), secs(4));
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(start(&editor, b), secs(2));
    assert_eq!(start(&editor, c), secs(4));
    assert_eq!(start(&editor, sound_of(&editor, c)), secs(4));
}

#[test]
fn dropping_inside_a_shot_splits_it_around_the_new_one() {
    let (mut editor, [_, _, c]) = three_shots();
    let track = editor.track_of(c).unwrap();

    editor.insert_clip_at(c, track, secs(1)).unwrap();

    let starts: Vec<TimelineTime> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|clip| clip.timeline.start)
        .collect();
    // A's first second, C, the rest of A, then B.
    assert_eq!(starts, vec![secs(0), secs(1), secs(3), secs(4)]);
    assert_eq!(start(&editor, c), secs(1));
}

#[test]
fn a_picture_cannot_be_inserted_on_a_sound_track() {
    let (mut editor, [a, _, c]) = three_shots();
    let sound_track = editor.track_of(sound_of(&editor, a)).unwrap();
    assert!(matches!(
        editor.insert_clip_at(c, sound_track, secs(2)),
        Err(EditorError::ClipKindMismatch)
    ));
}
