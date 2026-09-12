//! Nudging clips by whole frames, and trimming to the playhead (§57).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, TrimEdge};

/// 30 fps, so a frame is 32,000 ticks.
fn frame(n: i64) -> TimelineTime {
    TimelineTime::from_ticks(n * 32_000)
}

fn editor_with_sound() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Nudge");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0], placed[1])
}

fn start_of(editor: &Editor, clip: ClipId) -> TimelineTime {
    editor
        .clip_payload(clip)
        .map(|p| p.start())
        .expect("clip is gone")
}

#[test]
fn a_nudge_moves_a_clip_and_its_sound_by_whole_frames() {
    let (mut editor, picture, sound) = editor_with_sound();
    let depth = editor.undo_depth();

    editor.nudge_clips(&[picture], 3).unwrap();

    assert_eq!(start_of(&editor, picture), frame(3));
    assert_eq!(
        start_of(&editor, sound),
        frame(3),
        "the sound stayed behind"
    );
    assert_eq!(editor.undo_depth(), depth + 1, "one undo step");

    editor.undo().unwrap();
    assert_eq!(start_of(&editor, picture), TimelineTime::ZERO);
}

#[test]
fn a_nudge_stops_at_the_start_of_the_timeline() {
    let (mut editor, picture, _) = editor_with_sound();
    editor.nudge_clips(&[picture], 2).unwrap();

    editor.nudge_clips(&[picture], -10).unwrap();

    assert_eq!(start_of(&editor, picture), TimelineTime::ZERO);
    // And again from zero does nothing at all.
    assert_eq!(editor.nudge_clips(&[picture], -1).unwrap(), 0);
}

/// Two clips side by side, nudged together: the one in front moves first, so
/// the pair does not collide with itself.
#[test]
fn a_run_of_clips_nudges_together() {
    let (mut editor, first, _) = editor_with_sound();
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/b.mp4",
        MediaTime::from_seconds(4),
    ));
    let second = editor.place_media(media).unwrap()[0];
    assert_eq!(start_of(&editor, second), TimelineTime::from_seconds(4));

    editor.nudge_clips(&[first, second], 2).unwrap();
    assert_eq!(start_of(&editor, first), frame(2));
    assert_eq!(
        start_of(&editor, second),
        TimelineTime::from_seconds(4) + frame(2)
    );

    editor.nudge_clips(&[first, second], -2).unwrap();
    assert_eq!(start_of(&editor, first), TimelineTime::ZERO);
    assert_eq!(start_of(&editor, second), TimelineTime::from_seconds(4));
}

#[test]
fn trimming_to_the_playhead_moves_the_edge_there() {
    let (mut editor, picture, sound) = editor_with_sound();
    editor.set_playhead(TimelineTime::from_seconds(1));

    assert_eq!(
        editor
            .trim_to_playhead(&[picture], TrimEdge::Start)
            .unwrap(),
        1
    );

    assert_eq!(start_of(&editor, picture), TimelineTime::from_seconds(1));
    assert_eq!(
        start_of(&editor, sound),
        TimelineTime::from_seconds(1),
        "the sound was not trimmed with the picture"
    );
}

#[test]
fn trimming_the_end_shortens_the_clip() {
    let (mut editor, picture, _) = editor_with_sound();
    editor.set_playhead(TimelineTime::from_seconds(3));

    editor.trim_to_playhead(&[picture], TrimEdge::End).unwrap();

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.video_tracks[0].clips()[0].timeline.end,
        TimelineTime::from_seconds(3)
    );
}

/// The playhead outside the clip is not a trim: the edge would fly off across
/// the timeline.
#[test]
fn a_playhead_outside_the_clip_trims_nothing() {
    let (mut editor, picture, _) = editor_with_sound();
    let before = editor.project().clone();
    editor.set_playhead(TimelineTime::from_seconds(30));

    assert_eq!(
        editor
            .trim_to_playhead(&[picture], TrimEdge::Start)
            .unwrap(),
        0
    );
    assert_eq!(editor.project(), &before);
}
