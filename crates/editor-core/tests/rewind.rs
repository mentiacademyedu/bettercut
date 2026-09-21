//! The rewind effect (`Editor::rewind`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn a_shot_winds_back_fast_with_its_sound_and_a_tape_look() {
    let (mut editor, _events) = Editor::new_project("Rewind");
    let place = |editor: &mut Editor, name: &str, seconds: i64| {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(seconds),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        editor.place_media(media).unwrap()[0]
    };
    let shot = place(&mut editor, "skate", 8);
    let after = place(&mut editor, "next", 2);
    let depth = editor.undo_depth();

    let back = editor.rewind(shot).unwrap();
    assert_eq!(editor.undo_depth(), depth + 1);

    let rewind = editor.video_clip(back).unwrap();
    // Eight seconds wound back at 4× takes two.
    assert_eq!(rewind.timeline.start, TimelineTime::from_seconds(8));
    assert_eq!(rewind.timeline.duration(), TimelineTime::from_seconds(2));
    assert!(rewind.reversed);
    assert!(rewind.rgb_split > 0.0 && rewind.old_film > 0.0);
    // What came next moved along to make room.
    assert_eq!(
        editor.video_clip(after).unwrap().timeline.start,
        TimelineTime::from_seconds(10)
    );
    // The sound winds back with it.
    let sound = editor
        .linked_with(back)
        .into_iter()
        .find(|c| *c != back)
        .expect("the rewind has no sound");
    let sound = editor.audio_clip(sound).unwrap();
    assert!(sound.reversed);
    assert_eq!(sound.timeline, rewind.timeline);

    editor.undo().unwrap();
    assert!(editor.video_clip(back).is_none());
    assert_eq!(
        editor.video_clip(after).unwrap().timeline.start,
        TimelineTime::from_seconds(8)
    );
}
