//! Putting a finished bounce down in place of the lane it was mixed from
//! (`bettercut_editor_core::bounce`).
//!
//! The mixing itself is the export's, and covered there; this is the edit
//! either side of it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_editor_core::Editor;
use bettercut_editor_core::bounce::{bounce_folder, next_bounce_file};
use bettercut_editor_core::foundation::{MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::VolumePoint;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

/// Three four-second sound clips butted together on A1.
fn setup() -> (Editor, TrackId) {
    let (mut editor, _events) = Editor::new_project("Bounce");
    let mut asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/voice.wav",
        MediaTime::from_seconds(4),
    );
    asset.audio_codec = Some("pcm".to_owned());
    let media = editor.import_media(asset);
    for _ in 0..3 {
        editor.place_media(media).unwrap();
    }
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    (editor, track)
}

fn clips_on(editor: &Editor, track: TrackId) -> Vec<(i64, i64)> {
    editor
        .active_sequence()
        .unwrap()
        .audio_track(track)
        .unwrap()
        .clips()
        .iter()
        .map(|clip| (clip.timeline.start.ticks(), clip.timeline.end.ticks()))
        .collect()
}

#[test]
fn a_lane_with_clips_can_be_bounced_and_an_empty_one_cannot() {
    let (mut editor, track) = setup();
    assert!(editor.can_bounce(track));

    let empty = {
        editor.add_audio_track("A2").unwrap();
        editor.active_sequence().unwrap().audio_tracks[1].id
    };
    assert!(!editor.can_bounce(empty), "there is nothing on it");

    // A picture lane is not a sound lane.
    let video = editor.active_sequence().unwrap().video_tracks[0].id;
    assert!(!editor.can_bounce(video));

    // Nor is a locked lane, which is a lane the user asked to be left alone.
    editor
        .set_track_flag(track, bettercut_editor_core::TrackFlag::Locked, true)
        .unwrap();
    assert!(!editor.can_bounce(track));
}

/// The clips go, one clip arrives, and the lane's level is reset — it is in
/// the file now, and applying it again would apply it twice.
#[test]
fn a_bounce_replaces_the_lane_and_resets_its_level() {
    let (mut editor, track) = setup();
    editor.set_track_mix(track, 0.5, -0.4, false).unwrap();
    editor
        .set_track_volume(
            track,
            vec![
                VolumePoint::new(TimelineTime::ZERO, 1.0),
                VolumePoint::new(TimelineTime::from_seconds(12), 0.2),
            ],
            false,
        )
        .unwrap();
    assert_eq!(clips_on(&editor, track).len(), 3);

    editor
        .place_bounce(track, &fixture("tone-48k.wav"), TimelineTime::ZERO)
        .expect("placed");

    let after = clips_on(&editor, track);
    assert_eq!(after.len(), 1, "the lane should hold one clip now");
    assert_eq!(after[0].0, 0);
    let lane = editor
        .active_sequence()
        .unwrap()
        .audio_track(track)
        .unwrap();
    assert_eq!(lane.gain, 1.0, "the lane's gain is in the file now");
    assert_eq!(lane.pan, 0.0);
    assert!(editor.track_volume(track).is_empty(), "so is its line");
}

/// One undo step for the lot: the clips come back, and so does the level.
#[test]
fn one_undo_puts_the_lane_back() {
    let (mut editor, track) = setup();
    editor.set_track_mix(track, 0.5, 0.0, false).unwrap();
    let before = clips_on(&editor, track);

    editor
        .place_bounce(track, &fixture("tone-48k.wav"), TimelineTime::ZERO)
        .expect("placed");
    assert_eq!(editor.undo_label().as_deref(), Some("Bounce Track"));

    editor.undo().expect("undo");
    assert_eq!(clips_on(&editor, track), before);
    let lane = editor
        .active_sequence()
        .unwrap()
        .audio_track(track)
        .unwrap();
    assert_eq!(lane.gain, 0.5, "the lane's level came back too");
}

/// The bounce is placed where the lane's sound started, not always at zero.
#[test]
fn a_bounce_lands_where_the_sound_started() {
    let (mut editor, track) = setup();
    let at = TimelineTime::from_seconds(5);

    editor
        .place_bounce(track, &fixture("tone-48k.wav"), at)
        .expect("placed");

    assert_eq!(clips_on(&editor, track)[0].0, at.ticks());
}

/// The file is imported like any other sound, so it is in the media browser:
/// the edit depends on it now, and a file a project needs but never lists is a
/// file someone deletes.
#[test]
fn the_bounced_file_joins_the_media_list() {
    let (mut editor, track) = setup();
    let before = editor.project().media.len();

    editor
        .place_bounce(track, &fixture("tone-48k.wav"), TimelineTime::ZERO)
        .expect("placed");

    assert_eq!(editor.project().media.len(), before + 1);
    let added = editor.project().media.last().expect("an asset");
    assert!(
        !added.baked,
        "a bounce is part of the edit, not a cache file"
    );
    assert_eq!(added.kind, MediaKind::Audio);
}

#[test]
fn bounces_are_kept_beside_the_project_and_never_overwrite_each_other() {
    let folder = tempfile::tempdir().expect("temp");
    let project = folder.path().join("Trip.vproj");
    assert_eq!(
        bounce_folder(Some(&project)).file_name().unwrap(),
        "Trip bounces"
    );

    let first = next_bounce_file(folder.path(), "Music");
    assert_eq!(first.file_name().unwrap(), "Music bounce 1.wav");
    std::fs::write(&first, b"a bounce").expect("write");
    let second = next_bounce_file(folder.path(), "Music");
    assert_eq!(second.file_name().unwrap(), "Music bounce 2.wav");

    // A lane named with something a filesystem would refuse still gets a file.
    let awkward = next_bounce_file(folder.path(), "A1: voice/over");
    assert_eq!(awkward.file_name().unwrap(), "A1--voice-over bounce 1.wav");
}

#[test]
fn the_file_is_named_after_the_lane() {
    let (mut editor, track) = setup();
    editor.rename_track(track, "Music").unwrap();
    let path = editor.bounce_file(track);
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("Music bounce"),
        "{path:?}"
    );
}
