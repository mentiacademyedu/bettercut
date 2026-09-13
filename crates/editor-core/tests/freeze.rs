//! Freeze frame: holding one frame, with room made for it (§10).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A ten-second video with sound at zero.
fn editor_with_clip() -> (Editor, ClipId) {
    let (mut editor, _events) = Editor::new_project("Freeze");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0])
}

fn spans(editor: &Editor) -> Vec<(i64, i64)> {
    editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| {
            (
                c.timeline.start.ticks() / 960_000,
                c.timeline.end.ticks() / 960_000,
            )
        })
        .collect()
}

#[test]
fn a_freeze_holds_the_frame_and_makes_room_for_it() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(seconds(4));
    let depth = editor.undo_depth();

    let held = editor.freeze_frame(clip, seconds(2)).unwrap();

    // Four seconds, two held, then the rest — twelve in all.
    assert_eq!(spans(&editor), [(0, 4), (4, 6), (6, 12)]);
    let frozen = editor.video_clip(held).expect("the held clip");
    assert!(frozen.frozen);
    assert_eq!(
        frozen.source_time_at(seconds(5)),
        MediaTime::from_seconds(4),
        "the hold is not on the frame the playhead was over"
    );
    assert_eq!(
        frozen.source_time_at(seconds(4)),
        frozen.source_time_at(seconds(6)),
        "the hold moved through its source"
    );

    // The sound was cut and moved, leaving silence under the hold.
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.audio_tracks[0]
            .clips()
            .iter()
            .map(|c| (
                c.timeline.start.ticks() / 960_000,
                c.timeline.end.ticks() / 960_000
            ))
            .collect::<Vec<_>>(),
        [(0, 4), (6, 12)]
    );

    assert_eq!(editor.undo_depth(), depth + 1, "one undo step");
    editor.undo().unwrap();
    assert_eq!(spans(&editor), [(0, 10)]);
}

/// The hold keeps the shot's framing and grade, so it does not jump at the
/// moment it starts.
#[test]
fn the_held_frame_keeps_the_shots_look() {
    use bettercut_editor_core::ClipProperty;

    let (mut editor, clip) = editor_with_clip();
    editor
        .set_clip_property(clip, ClipProperty::Scale { x: 1.4, y: 1.4 }, false)
        .unwrap();
    editor
        .set_clip_property(clip, ClipProperty::Saturation(0.5), false)
        .unwrap();
    editor.set_playhead(seconds(4));

    let held = editor.freeze_frame(clip, seconds(2)).unwrap();

    let frozen = editor.video_clip(held).unwrap();
    assert_eq!(frozen.transform.scale.x, 1.4);
    assert_eq!(frozen.color.saturation, 0.5);
}

/// A hold can be dragged out past the end of its own footage: it is one frame,
/// and there is no more of it to run out of. An ordinary clip stops at the end
/// of its media.
#[test]
fn a_hold_can_be_stretched_past_the_media() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(seconds(4));
    let held = editor.freeze_frame(clip, seconds(2)).unwrap();

    // Take away what follows the hold, so there is room to stretch into.
    let tail = editor.active_sequence().unwrap().video_tracks[0].clips()[2].id;
    let track = editor.track_of(tail).unwrap();
    editor.ripple_delete(track, tail).unwrap();

    let track = editor.track_of(held).unwrap();
    editor
        .trim_clip(
            track,
            held,
            bettercut_editor_core::TrimEdge::End,
            seconds(20),
        )
        .unwrap();

    let frozen = editor.video_clip(held).unwrap();
    assert_eq!(frozen.timeline.end, seconds(20));
    assert_eq!(
        frozen.source_time_at(seconds(19)),
        MediaTime::from_seconds(4),
        "a stretched hold stopped holding its frame"
    );
}

#[test]
fn freezing_needs_the_playhead_over_the_clip() {
    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(seconds(30));
    let before = editor.project().clone();

    let err = editor.freeze_frame(clip, seconds(2)).unwrap_err();

    assert!(matches!(err, EditorError::PlayheadOffClip), "{err}");
    assert_eq!(
        editor.project(),
        &before,
        "a refused freeze changed the edit"
    );
}

/// A freeze survives the journal, so a crash does not lose it (§38.2).
#[test]
fn a_freeze_survives_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("freeze.vproj");
    {
        let (mut editor, clip) = editor_with_clip();
        editor.save_as(&path).unwrap();
        editor.set_playhead(seconds(4));
        editor.freeze_frame(clip, seconds(2)).unwrap();
        std::mem::forget(editor);
    }

    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).unwrap();
    assert_eq!(session.failed, 0);
    let held: Vec<_> = session.project.active().unwrap().video_tracks[0]
        .clips()
        .iter()
        .filter(|c| c.frozen)
        .collect();
    assert_eq!(held.len(), 1, "the hold did not come back");
    assert_eq!(held[0].timeline.duration(), seconds(2));
}

/// Re-timing applies to footage. A hold is one picture, and taking its length from its
/// source range — as a speed change does — used to collapse a two-second hold
/// to the single frame it holds.
#[test]
fn a_hold_cannot_be_re_timed() {
    use bettercut_editor_core::foundation::Rational;

    let (mut editor, clip) = editor_with_clip();
    editor.set_playhead(seconds(4));
    let held = editor.freeze_frame(clip, seconds(2)).unwrap();
    assert!(!editor.can_retime(held));

    let err = editor
        .set_clip_speed(held, Rational::new(2, 1).unwrap(), false)
        .unwrap_err();

    assert!(matches!(err, EditorError::NoMotionToRetime), "{err}");
    assert_eq!(
        editor.video_clip(held).unwrap().timeline.duration(),
        seconds(2),
        "the hold was resized anyway"
    );
    assert!(editor.can_retime(clip), "ordinary footage still re-times");
}

/// A photo is one picture too.
#[test]
fn a_photo_cannot_be_re_timed() {
    use bettercut_editor_core::foundation::Rational;

    let (mut editor, _) = editor_with_clip();
    let photo = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/p.jpg",
        MediaTime::ZERO,
    ));
    let placed = editor.place_media(photo).unwrap()[0];

    assert!(!editor.can_retime(placed));
    assert!(matches!(
        editor.set_clip_speed(placed, Rational::new(2, 1).unwrap(), false),
        Err(EditorError::NoMotionToRetime)
    ));
}
