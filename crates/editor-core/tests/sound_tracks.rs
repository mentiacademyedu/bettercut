//! A file with several sound tracks — a screen recording's game and
//! microphone kept apart — comes onto the timeline with all of them, each on
//! a lane of its own and linked to the picture; the extra tracks are entries
//! of their own, not listed, that follow the file when it is relinked.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

/// A recording with `tracks` sound tracks, the first played.
fn recording(tracks: u16) -> MediaAsset {
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/stream.mkv",
        MediaTime::from_seconds(6),
    );
    asset.audio_codec = Some("aac".to_owned());
    asset.audio_channels = Some(2);
    asset.audio_sample_rate = Some(48_000);
    asset.audio_tracks = tracks;
    asset.audio_track = Some(0);
    asset
}

fn sound_lanes(editor: &Editor) -> Vec<Vec<(Option<u16>, TimelineTime)>> {
    let sequence = editor.active_sequence().unwrap();
    sequence
        .audio_tracks
        .iter()
        .map(|lane| {
            lane.clips()
                .iter()
                .map(|clip| {
                    let asset = editor.project().media_asset(clip.media_id).unwrap();
                    (asset.audio_track, clip.timeline.start)
                })
                .collect()
        })
        .collect()
}

#[test]
fn every_track_comes_in_on_a_lane_of_its_own_linked_to_the_picture() {
    let (mut editor, _rx) = Editor::new_project("Tracks");
    let media = editor.import_media(recording(3));
    let placed = editor.place_media(media).unwrap();
    assert_eq!(placed.len(), 4, "a picture and three sounds");

    let lanes = sound_lanes(&editor);
    assert_eq!(
        lanes,
        vec![
            vec![(Some(0), TimelineTime::ZERO)],
            vec![(Some(1), TimelineTime::ZERO)],
            vec![(Some(2), TimelineTime::ZERO)],
        ],
        "each track on its own lane, under the picture"
    );
    let link = editor.video_clip(placed[0]).unwrap().link;
    assert!(link.is_some());
    for clip in &placed[1..] {
        assert_eq!(
            editor.audio_clip(*clip).unwrap().link,
            link,
            "a track not linked"
        );
    }

    // The further tracks are entries of their own, which the browser does
    // not list; the file is listed once.
    let listed: Vec<_> = editor
        .project()
        .media
        .iter()
        .filter(|m| m.is_listed())
        .collect();
    assert_eq!(listed.len(), 1);
    assert_eq!(editor.project().media.len(), 3);

    // One thing done, one step undone: the picture and every track.
    editor.undo().unwrap();
    assert!(
        sound_lanes(&editor).iter().all(Vec::is_empty),
        "undo left a track"
    );
    assert!(editor.video_clip(placed[0]).is_none());
    editor.redo().unwrap();
    assert_eq!(
        sound_lanes(&editor)
            .iter()
            .filter(|l| !l.is_empty())
            .count(),
        3
    );

    // Asked again, there is nothing more to bring.
    assert!(matches!(
        editor.add_other_sound_tracks(placed[0]),
        Err(EditorError::NoOtherSoundTracks)
    ));
}

#[test]
fn a_second_placing_reuses_the_track_entries() {
    let (mut editor, _rx) = Editor::new_project("Tracks");
    let media = editor.import_media(recording(2));
    editor.place_media(media).unwrap();
    let placed = editor.place_media(media).unwrap();
    assert_eq!(placed.len(), 3);
    assert_eq!(
        editor.project().media.len(),
        2,
        "a second entry for the same track"
    );
    // Both copies of the second track are on its lane, one after the other.
    assert_eq!(sound_lanes(&editor)[1].len(), 2);
}

#[test]
fn a_file_with_one_track_comes_in_as_before() {
    let (mut editor, _rx) = Editor::new_project("Tracks");
    let media = editor.import_media(recording(1));
    let placed = editor.place_media(media).unwrap();
    assert_eq!(placed.len(), 2);
    assert!(matches!(
        editor.add_other_sound_tracks(placed[0]),
        Err(EditorError::NoOtherSoundTracks)
    ));
}

#[test]
fn the_tracks_follow_their_file_when_it_is_relinked() {
    let (mut editor, _rx) = Editor::new_project("Tracks");
    let media = editor.import_media(recording(2));
    editor.place_media(media).unwrap();
    let moved = std::env::temp_dir().join("bettercut-moved-stream.mkv");
    editor.relink_media(media, &moved).unwrap();
    for asset in &editor.project().media {
        assert_eq!(asset.path, moved, "{} did not follow", asset.display_name());
    }
    editor.undo().unwrap();
    for asset in &editor.project().media {
        assert_eq!(asset.path, std::path::PathBuf::from("C:/media/stream.mkv"));
    }
}

#[test]
fn dropped_on_the_timeline_it_brings_every_track_too() {
    use bettercut_editor_core::three_point::DropKind;
    for kind in [DropKind::Overwrite, DropKind::Insert] {
        let (mut editor, _rx) = Editor::new_project("Tracks");
        let media = editor.import_media(recording(2));
        let at = TimelineTime::from_seconds(2);
        let placed = editor.place_media_at(media, None, at, kind).unwrap();
        assert_eq!(placed.len(), 3, "{kind:?}: a picture and two sounds");
        assert_eq!(
            sound_lanes(&editor),
            vec![vec![(Some(0), at)], vec![(Some(1), at)]],
            "{kind:?}: each track on its own lane, under the picture"
        );
        let link = editor.video_clip(placed[0]).unwrap().link;
        assert!(link.is_some());
        assert_eq!(editor.audio_clip(placed[2]).unwrap().link, link);
        editor.undo().unwrap();
        assert!(
            sound_lanes(&editor).iter().all(Vec::is_empty),
            "{kind:?}: one undo step"
        );
    }
}

#[test]
fn a_sound_file_with_two_tracks_comes_in_as_two_sounds() {
    let (mut editor, _rx) = Editor::new_project("Tracks");
    let mut sound = recording(2);
    sound.kind = MediaKind::Audio;
    let media = editor.import_media(sound);
    let placed = editor.place_media(media).unwrap();
    assert_eq!(placed.len(), 2);
    assert_eq!(
        sound_lanes(&editor),
        vec![
            vec![(Some(0), TimelineTime::ZERO)],
            vec![(Some(1), TimelineTime::ZERO)]
        ]
    );
    // Linked, so they move together; each keeps its own volume and effects.
    let a = editor.audio_clip(placed[0]).unwrap().link;
    assert!(a.is_some());
    assert_eq!(editor.audio_clip(placed[1]).unwrap().link, a);
}

#[test]
fn two_mics_on_left_and_right_become_two_sounds() {
    use bettercut_editor_core::timeline::ChannelMode;
    let (mut editor, _rx) = Editor::new_project("Tracks");
    let media = editor.import_media(recording(1));
    let placed = editor.place_media(media).unwrap();
    let (picture, sound) = (placed[0], placed[1]);
    let right = editor.split_left_right(sound).unwrap();

    let left = editor.audio_clip(sound).unwrap();
    assert_eq!(left.channels, ChannelMode::LeftToBoth);
    let made = editor.audio_clip(right).unwrap();
    assert_eq!(made.channels, ChannelMode::RightToBoth);
    assert_eq!(made.timeline, left.timeline, "the right under the left");
    assert_ne!(
        editor.track_of(right),
        editor.track_of(sound),
        "a lane of its own"
    );
    let link = editor.video_clip(picture).unwrap().link;
    assert!(link.is_some());
    assert_eq!(left.link, link);
    assert_eq!(made.link, link, "the right not linked to the picture");

    // One step back to the one stereo sound.
    editor.undo().unwrap();
    assert!(editor.audio_clip(right).is_none());
    assert_eq!(
        editor.audio_clip(sound).unwrap().channels,
        ChannelMode::Stereo
    );

    // A mono sound has nothing to split.
    let mut mono = recording(1);
    mono.path = "C:/media/voice.mkv".into();
    mono.audio_channels = Some(1);
    let media = editor.import_media(mono);
    let placed = editor.place_media(media).unwrap();
    assert!(matches!(
        editor.split_left_right(placed[1]),
        Err(EditorError::NotStereo)
    ));
}

#[test]
fn each_track_is_muted_and_faded_by_itself() {
    let (mut editor, _rx) = Editor::new_project("Tracks");
    let media = editor.import_media(recording(2));
    let placed = editor.place_media(media).unwrap();
    let (picture, game, mic) = (placed[0], placed[1], placed[2]);

    // The game muted, the microphone still heard.
    assert_eq!(editor.set_muted(game, true).unwrap(), 1);
    assert!(editor.audio_clip(game).unwrap().muted);
    assert!(
        !editor.audio_clip(mic).unwrap().muted,
        "muting one track muted the other"
    );
    assert!(!editor.is_muted(mic));

    // A fade on the microphone is the microphone's.
    let second = TimelineTime::from_seconds(1);
    editor
        .set_clip_fades(mic, second, TimelineTime::ZERO, false)
        .unwrap();
    assert_eq!(editor.audio_clip(mic).unwrap().fade_in, second);
    assert_eq!(editor.audio_clip(game).unwrap().fade_in, TimelineTime::ZERO);

    // Asked of the picture, every sound under it.
    assert_eq!(
        editor.set_muted(picture, true).unwrap(),
        1,
        "only the mic was left to mute"
    );
    assert!(editor.audio_clip(mic).unwrap().muted);
    assert_eq!(editor.set_muted(picture, false).unwrap(), 2);
}
