//! Drawing, moving and clearing a lane's volume line
//! (`Editor::set_track_volume`, `bettercut_timeline::track_volume`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::VolumePoint;
use bettercut_editor_core::{Editor, EditorError};

fn point(seconds: i64, gain: f32) -> VolumePoint {
    VolumePoint::new(TimelineTime::from_seconds(seconds), gain)
}

/// The lanes of a fresh project with one sound clip on it.
fn setup() -> (Editor, TrackId, TrackId) {
    let (mut editor, _events) = Editor::new_project("Volume");
    let mut asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/bed.wav",
        MediaTime::from_seconds(30),
    );
    asset.audio_codec = Some("pcm".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    let sequence = editor.active_sequence().unwrap();
    let audio = sequence.audio_tracks[0].id;
    let video = sequence.video_tracks[0].id;
    (editor, audio, video)
}

#[test]
fn a_lane_starts_with_no_line() {
    let (editor, audio, _video) = setup();
    assert!(editor.track_volume(audio).is_empty());
}

#[test]
fn a_line_is_written_sorted_and_undone_whole() {
    let (mut editor, audio, _video) = setup();
    editor
        .set_track_volume(audio, vec![point(10, 0.2), point(0, 1.0)], false)
        .expect("ok");

    let points = editor.track_volume(audio);
    assert_eq!(points.len(), 2);
    assert_eq!(points[0].at, TimelineTime::ZERO, "sorted on the way in");
    assert_eq!(points[1].gain, 0.2);
    assert_eq!(editor.undo_label().as_deref(), Some("Track Volume"));

    editor.undo().expect("undo");
    assert!(editor.track_volume(audio).is_empty(), "undo left points");
}

/// A drag is one undo step, not one per frame of the drag (§11).
#[test]
fn a_drag_collapses_into_one_undo_step() {
    let (mut editor, audio, _video) = setup();
    editor
        .set_track_volume(audio, vec![point(0, 1.0), point(10, 1.0)], false)
        .expect("ok");
    for gain in [0.9, 0.8, 0.7] {
        editor
            .set_track_volume(audio, vec![point(0, 1.0), point(10, gain)], true)
            .expect("ok");
    }
    assert_eq!(editor.track_volume(audio)[1].gain, 0.7);

    editor.undo().expect("undo");
    assert!(
        editor.track_volume(audio).is_empty(),
        "the whole drag should be one step"
    );
}

#[test]
fn a_point_is_added_at_the_level_the_line_already_has() {
    let (mut editor, audio, _video) = setup();
    editor
        .set_track_volume(audio, vec![point(0, 1.0), point(10, 0.0)], false)
        .expect("ok");

    editor
        .add_track_volume_point(audio, TimelineTime::from_seconds(5))
        .expect("ok");

    let points = editor.track_volume(audio);
    assert_eq!(points.len(), 3);
    assert_eq!(points[1].at, TimelineTime::from_seconds(5));
    assert!(
        (points[1].gain - 0.5).abs() < 1e-4,
        "the shape jumped: {}",
        points[1].gain
    );
}

/// The first point on an empty line takes the lane's own level, so turning
/// automation on changes nothing until something is moved.
#[test]
fn the_first_point_takes_the_lanes_static_level() {
    let (mut editor, audio, _video) = setup();
    editor.set_track_mix(audio, 0.4, 0.0, false).expect("ok");

    editor
        .add_track_volume_point(audio, TimelineTime::from_seconds(3))
        .expect("ok");

    let points = editor.track_volume(audio);
    assert_eq!(points.len(), 1);
    assert!((points[0].gain - 0.4).abs() < 1e-6, "{}", points[0].gain);
}

#[test]
fn clearing_takes_the_line_off() {
    let (mut editor, audio, _video) = setup();
    editor
        .set_track_volume(audio, vec![point(0, 1.0), point(10, 0.0)], false)
        .expect("ok");

    assert_eq!(editor.clear_track_volume(audio).expect("ok"), 2);
    assert!(editor.track_volume(audio).is_empty());
    assert_eq!(
        editor.undo_label().as_deref(),
        Some("Clear Track Volume"),
        "clearing says what it was"
    );
    assert_eq!(
        editor.clear_track_volume(audio).expect("ok"),
        0,
        "and again is nothing to do"
    );
}

/// A picture lane has no volume to ride.
#[test]
fn a_picture_lane_is_refused() {
    let (mut editor, _audio, video) = setup();
    let err = editor
        .set_track_volume(video, vec![point(0, 0.5)], false)
        .unwrap_err();
    assert!(matches!(err, EditorError::ClipKindMismatch), "{err}");
    assert!(editor.track_volume(video).is_empty());
}

#[test]
fn a_line_survives_a_save_and_an_open() {
    let folder = tempfile::tempdir().expect("temp");
    let (mut editor, audio, _video) = setup();
    editor
        .set_track_volume(audio, vec![point(0, 1.0), point(10, 0.25)], false)
        .expect("ok");

    let path = folder.path().join("project.vproj");
    editor.save_as(&path).expect("saved");
    let (opened, _rx) = Editor::open(&path).expect("opened");

    let points = opened.track_volume(audio);
    assert_eq!(points.len(), 2);
    assert_eq!(points[1].gain, 0.25);
}
