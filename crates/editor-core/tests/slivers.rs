//! Cleaning up the slivers a fast cut leaves (`Editor::clean_up_slivers`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};

#[test]
fn a_flash_frame_and_a_blink_of_black_go_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Slivers");
    let media = editor.import_media(bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Image,
        "C:/media/still.png",
        MediaTime::ZERO,
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let mut place = |start: i64, length: i64| -> ClipId {
        let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(length)).unwrap();
        let clip = VideoClip::new(media, TimelineTime::from_ticks(start), source).unwrap();
        let id = clip.id;
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .unwrap();
        id
    };
    let second = 960_000;
    let a = place(0, 2 * second);
    // A 10 ms flash, then 10 ms of nothing, then the next shot.
    let flash = place(2 * second, 9_600);
    let b = place(2 * second + 19_200, 2 * second);
    let depth = editor.undo_depth();

    assert_eq!(editor.clean_up_slivers(2).unwrap(), (1, 1));
    assert!(
        editor.video_clip(flash).is_none(),
        "the flash is still there"
    );
    assert_eq!(
        editor.clip_start(b),
        Some(TimelineTime::from_seconds(2)),
        "the gap is still open"
    );
    assert!(editor.video_clip(a).is_some());
    assert_eq!(editor.undo_depth(), depth + 1, "one step");

    assert_eq!(editor.clean_up_slivers(2).unwrap(), (0, 0), "nothing left");
}
