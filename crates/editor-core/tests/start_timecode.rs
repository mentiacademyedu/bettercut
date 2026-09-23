//! The start timecode (`Sequence::start_timecode`): what the first frame is
//! called. Nothing moves; only the names of positions change, through one
//! place (`Editor::display_time`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};

#[test]
fn a_sequence_starts_at_zero_and_is_renamed_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Broadcast");
    assert_eq!(editor.start_timecode(), TimelineTime::ZERO);
    let depth = editor.undo_depth();

    let hour = TimelineTime::from_seconds(3_600);
    editor.set_start_timecode(hour).unwrap();
    assert_eq!(editor.start_timecode(), hour);
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(
        editor.display_time(TimelineTime::from_seconds(5)),
        TimelineTime::from_seconds(3_605)
    );

    editor.undo().unwrap();
    assert_eq!(editor.start_timecode(), TimelineTime::ZERO);
    assert_eq!(
        editor.display_time(TimelineTime::from_seconds(5)),
        TimelineTime::from_seconds(5)
    );
}

/// Positions on the timeline are what they were: the clip did not move.
#[test]
fn nothing_on_the_timeline_moves() {
    let (mut editor, _events) = Editor::new_project("Broadcast");
    let clip = VideoClip::new(
        MediaId::new(),
        TimelineTime::from_seconds(2),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(3)).unwrap(),
    )
    .unwrap();
    let id = clip.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();

    editor
        .set_start_timecode(TimelineTime::from_seconds(600))
        .unwrap();
    assert_eq!(
        editor.video_clip(id).unwrap().timeline.start,
        TimelineTime::from_seconds(2)
    );
}

/// A first frame called minus something is a countdown, not a timecode.
#[test]
fn a_start_before_zero_is_held_at_zero() {
    let (mut editor, _events) = Editor::new_project("Broadcast");
    editor
        .set_start_timecode(TimelineTime::from_seconds(-30))
        .unwrap();
    assert_eq!(editor.start_timecode(), TimelineTime::ZERO);
}

/// A project written before the start existed loads at zero.
#[test]
fn an_older_sequence_starts_at_zero() {
    let (editor, _events) = Editor::new_project("Old");
    let mut json = serde_json::to_value(editor.active_sequence().unwrap()).unwrap();
    json.as_object_mut().unwrap().remove("start_timecode");
    let back: bettercut_editor_core::timeline::Sequence = serde_json::from_value(json).unwrap();
    assert_eq!(back.start_timecode, TimelineTime::ZERO);
}
