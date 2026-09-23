//! Moving a clip a lane up or down (`Editor::move_to_neighbour_lane`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};

#[test]
fn a_clip_moves_up_a_lane_keeping_its_time_and_back_down() {
    let (mut editor, _events) = Editor::new_project("Lanes");
    editor.add_video_track("V2").unwrap();
    let (v1, v2) = {
        let s = editor.active_sequence().unwrap();
        (s.video_tracks[0].id, s.video_tracks[1].id)
    };
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(3)).unwrap();
    let clip = VideoClip::new(MediaId::new(), TimelineTime::from_seconds(2), source).unwrap();
    let id = clip.id;
    editor
        .add_clip(v1, ClipPayload::Video(Box::new(clip)))
        .unwrap();

    // V1 is the bottom picture lane: there is nothing below it.
    assert_eq!(editor.neighbour_lane(id, false), None);
    assert_eq!(editor.neighbour_lane(id, true), Some(v2));

    editor.move_to_neighbour_lane(id, true).unwrap();
    let span = editor.active_sequence().unwrap().clip_span(id).unwrap();
    assert_eq!(span.track, v2);
    assert_eq!(
        span.timeline.start,
        TimelineTime::from_seconds(2),
        "the time moved"
    );

    assert!(
        editor.move_to_neighbour_lane(id, true).is_err(),
        "nothing above V2"
    );
    editor.move_to_neighbour_lane(id, false).unwrap();
    assert_eq!(
        editor
            .active_sequence()
            .unwrap()
            .clip_span(id)
            .unwrap()
            .track,
        v1
    );
}
