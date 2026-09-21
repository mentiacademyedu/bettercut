//! Boomerang: a shot followed by itself backwards (`Editor::boomerang`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A 3 s shot with sound, then a 2 s shot.
fn two_shots() -> Editor {
    let (mut editor, _events) = Editor::new_project("Boomerang");
    for (name, length) in [("jump", 3), ("after", 2)] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(length),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        editor.place_media(media).unwrap();
    }
    editor
}

#[test]
fn a_shot_is_followed_by_itself_backwards() {
    let mut editor = two_shots();
    let sequence = editor.active_sequence().unwrap().clone();
    let jump = sequence.video_tracks[0].clips()[0].clone();
    let after = sequence.video_tracks[0].clips()[1].id;
    let depth = editor.undo_depth();

    let back = editor.boomerang(jump.id).unwrap();
    assert_eq!(editor.undo_depth(), depth + 1);

    let copy = editor.video_clip(back).unwrap();
    assert_eq!(copy.timeline.start, seconds(3));
    assert_eq!(copy.timeline.end, seconds(6));
    assert!(copy.reversed, "the copy plays forwards");
    assert_eq!(copy.source, jump.source);

    // The next shot moved along by the shot's length.
    let span = editor.active_sequence().unwrap().clip_span(after).unwrap();
    assert_eq!(span.timeline.start, seconds(6));

    // The copy's sound is there, backwards, and tied to the copy — not to the
    // original.
    let tied = editor.linked_with(back);
    assert_eq!(tied.len(), 2, "the reversed picture has no sound");
    let sound = tied.into_iter().find(|c| *c != back).unwrap();
    let sound_clip = editor.audio_clip(sound).unwrap();
    assert!(sound_clip.reversed);
    assert_eq!(sound_clip.timeline, copy.timeline);
    assert!(!editor.linked_with(jump.id).contains(&back));

    editor.undo().unwrap();
    assert!(editor.video_clip(back).is_none());
    let span = editor.active_sequence().unwrap().clip_span(after).unwrap();
    assert_eq!(span.timeline.start, seconds(3));
}

/// A reversed shot boomerangs forwards.
#[test]
fn a_reversed_shot_boomerangs_forwards() {
    let mut editor = two_shots();
    let jump = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    editor.set_reversed(jump, true).unwrap();
    let back = editor.boomerang(jump).unwrap();
    assert!(!editor.video_clip(back).unwrap().reversed);
}

/// A photo has no motion to play back.
#[test]
fn a_photo_is_refused() {
    let (mut editor, _events) = Editor::new_project("Boomerang");
    let photo = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/photo.jpg",
        MediaTime::ZERO,
    ));
    editor.place_media(photo).unwrap();
    let clip = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    assert!(matches!(
        editor.boomerang(clip),
        Err(EditorError::NoMotionToRetime)
    ));
}
