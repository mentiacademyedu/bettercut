//! The freeze-frame punch-in (`Editor::freeze_punch`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::freeze_punch::{PUNCH_TIME, PUNCH_ZOOM};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AnimatedParameter, TransitionKind};

#[test]
fn the_frame_holds_punches_in_and_flashes_in_one_step() {
    let (mut editor, _events) = Editor::new_project("Punch");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/jump.mp4",
        MediaTime::from_seconds(8),
    ));
    let clip = editor.place_media(media).unwrap()[0];
    editor.set_playhead(TimelineTime::from_seconds(3));
    let depth = editor.undo_depth();

    let held = editor
        .freeze_punch(clip, TimelineTime::from_seconds(2))
        .unwrap();
    assert_eq!(editor.undo_depth(), depth + 1, "not one step");

    let frozen = editor.video_clip(held).unwrap();
    assert!(frozen.frozen);
    assert_eq!(frozen.timeline.start, TimelineTime::from_seconds(3));
    let keys = frozen
        .keyframes
        .track(AnimatedParameter::ScaleX)
        .unwrap()
        .keys();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[1].value, PUNCH_ZOOM);
    assert_eq!(
        keys[1].time.ticks() - keys[0].time.ticks(),
        PUNCH_TIME.ticks()
    );

    // The shot before the hold flashes into it.
    let before = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .find(|c| c.timeline.end == frozen.timeline.start)
        .unwrap()
        .clone();
    assert_eq!(
        before.transition_out.map(|t| t.kind),
        Some(TransitionKind::Flash)
    );

    editor.undo().unwrap();
    assert!(editor.video_clip(held).is_none());
    assert!(editor.video_clip(clip).unwrap().transition_out.is_none());
}
