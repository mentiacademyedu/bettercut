//! Multicam: cameras folded into one clip, and cutting between them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor, EditorError};

fn secs(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn place(editor: &mut Editor, track: TrackId, name: &str) -> ClipId {
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        format!("C:/media/{name}.mp4"),
        MediaTime::from_seconds(30),
    ));
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap();
    let clip = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let id = clip.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    id
}

/// Two cameras on two lanes, both running 0–10 s.
fn two_cameras() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Multicam");
    editor.add_video_track("V2").unwrap();
    let sequence = editor.active_sequence().unwrap();
    let (first, second) = (sequence.video_tracks[0].id, sequence.video_tracks[1].id);
    let a = place(&mut editor, first, "wide");
    let b = place(&mut editor, second, "close");
    (editor, a, b)
}

#[test]
fn two_cameras_become_one_clip_showing_the_first() {
    let (mut editor, a, b) = two_cameras();
    let multicam = editor.make_multicam(&[a, b], "Interview").unwrap();

    assert_eq!(editor.angle_count(multicam), Some(2));
    assert_eq!(editor.angle_of(multicam), Some(0));
    // One clip on the timeline where two were.
    let on_screen: usize = editor
        .active_sequence()
        .unwrap()
        .video_tracks
        .iter()
        .map(|track| track.clips().len())
        .sum();
    assert_eq!(on_screen, 1);
}

#[test]
fn cutting_to_another_angle_splits_the_clip() {
    let (mut editor, a, b) = two_cameras();
    let multicam = editor.make_multicam(&[a, b], "Interview").unwrap();
    let depth = editor.undo_depth();

    let after = editor.cut_to_angle(multicam, secs(4), 1).unwrap();

    // A split gives both halves new identities, so the first half is found
    // where it sits rather than by the id it had.
    let before = editor
        .active_sequence()
        .unwrap()
        .clip_spans()
        .find(|span| span.timeline.start == secs(0))
        .map(|span| span.clip)
        .expect("a first half");
    assert_ne!(after, before);
    assert_eq!(
        editor.angle_of(before),
        Some(0),
        "the first half is angle 1"
    );
    assert_eq!(editor.angle_of(after), Some(1), "the second is angle 2");
    assert_eq!(editor.video_clip(after).unwrap().timeline.start, secs(4));
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(editor.video_clip(multicam).unwrap().timeline.end, secs(10));
    assert_eq!(editor.angle_of(multicam), Some(0));
}

/// A cut asked for at the very start is a change of angle, not a cut.
#[test]
fn cutting_at_the_start_just_changes_the_angle() {
    let (mut editor, a, b) = two_cameras();
    let multicam = editor.make_multicam(&[a, b], "Interview").unwrap();
    let same = editor.cut_to_angle(multicam, secs(0), 1).unwrap();
    assert_eq!(same, multicam);
    assert_eq!(editor.angle_of(multicam), Some(1));
}

#[test]
fn an_angle_that_is_not_there_is_refused() {
    let (mut editor, a, b) = two_cameras();
    let multicam = editor.make_multicam(&[a, b], "Interview").unwrap();
    assert!(matches!(
        editor.set_angle(multicam, Some(5)),
        Err(EditorError::NoSuchAngle { angle: 6, count: 2 })
    ));
    assert_eq!(editor.angle_of(multicam), Some(0));
}

#[test]
fn one_camera_is_not_a_multicam() {
    let (mut editor, a, _) = two_cameras();
    assert!(matches!(
        editor.make_multicam(&[a], "Solo"),
        Err(EditorError::NotEnoughAngles)
    ));
    assert_eq!(editor.angle_count(a), None);
    assert!(matches!(
        editor.set_angle(a, Some(0)),
        Err(EditorError::NotMulticam)
    ));
}
