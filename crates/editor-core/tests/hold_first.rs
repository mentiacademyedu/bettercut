//! The opening frame held before a shot plays (`Editor::hold_first_frame`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn the_first_frame_stands_still_and_the_shot_moves_along_with_its_sound() {
    let (mut editor, _events) = Editor::new_project("Hold");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(3),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    let (picture, sound) = (placed[0], placed[1]);
    let depth = editor.undo_depth();

    let held = editor
        .hold_first_frame(picture, TimelineTime::from_seconds(2))
        .unwrap();
    let frozen = editor.video_clip(held).unwrap();
    assert!(frozen.frozen);
    assert_eq!(frozen.timeline.start, TimelineTime::ZERO);
    assert_eq!(frozen.timeline.end, TimelineTime::from_seconds(2));
    assert_eq!(frozen.source.start, MediaTime::ZERO, "not the first frame");
    assert_eq!(
        editor.clip_start(picture),
        Some(TimelineTime::from_seconds(2))
    );
    assert_eq!(
        editor.clip_start(sound),
        Some(TimelineTime::from_seconds(2)),
        "sound left behind"
    );
    assert_eq!(editor.undo_depth(), depth + 1, "one step");
}
