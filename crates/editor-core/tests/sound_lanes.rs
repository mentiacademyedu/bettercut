//! A sound dropped over a sound: it goes on the lane it was dropped on where
//! there is room, else the next lane down with room, else a new lane — and
//! nothing already on the timeline moves.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn song(name: &str, seconds: i64) -> MediaAsset {
    let mut asset = MediaAsset::new(
        MediaKind::Audio,
        format!("C:/media/{name}.mp3"),
        MediaTime::from_seconds(seconds),
    );
    asset.audio_codec = Some("mp3".to_owned());
    asset.audio_channels = Some(2);
    asset.audio_sample_rate = Some(44_100);
    asset
}

/// Each sound lane's clips, as (start seconds, which file).
fn lanes(editor: &Editor) -> Vec<Vec<(i64, String)>> {
    editor
        .active_sequence()
        .unwrap()
        .audio_tracks
        .iter()
        .map(|lane| {
            lane.clips()
                .iter()
                .map(|c| {
                    let name = editor
                        .project()
                        .media_asset(c.media_id)
                        .unwrap()
                        .file_name
                        .clone();
                    (
                        c.timeline.start.ticks() / TimelineTime::from_seconds(1).ticks(),
                        name,
                    )
                })
                .collect()
        })
        .collect()
}

#[test]
fn a_sound_over_a_sound_goes_on_a_new_lane_and_both_stay() {
    let (mut editor, _rx) = Editor::new_project("Lanes");
    let a1 = editor.active_sequence().unwrap().audio_tracks[0].id;
    let music = editor.import_media(song("music", 10));
    let voice = editor.import_media(song("voice", 4));
    let effect = editor.import_media(song("effect", 1));

    editor
        .place_sound_at(music, None, TimelineTime::ZERO, Some(a1))
        .unwrap();
    // Onto the music: a lane of its own, under it, and the music untouched.
    editor
        .place_sound_at(voice, None, TimelineTime::from_seconds(2), Some(a1))
        .unwrap();
    assert_eq!(
        lanes(&editor),
        vec![
            vec![(0, "music.mp3".to_owned())],
            vec![(2, "voice.mp3".to_owned())],
        ]
    );
    // Over both: a third lane.
    editor
        .place_sound_at(effect, None, TimelineTime::from_seconds(3), Some(a1))
        .unwrap();
    assert_eq!(lanes(&editor).len(), 3);
    assert_eq!(lanes(&editor)[2], vec![(3, "effect.mp3".to_owned())]);

    // Where the second lane has room, it goes there rather than a new one.
    editor
        .place_sound_at(effect, None, TimelineTime::from_seconds(7), Some(a1))
        .unwrap();
    assert_eq!(lanes(&editor).len(), 3);
    assert_eq!(lanes(&editor)[1].len(), 2);

    // Dropped on a free lane, it lands on that one.
    let a3 = editor.active_sequence().unwrap().audio_tracks[2].id;
    editor
        .place_sound_at(effect, None, TimelineTime::from_seconds(8), Some(a3))
        .unwrap();
    assert_eq!(lanes(&editor)[2].len(), 2);

    // Below every lane: a new one, even with room above.
    editor
        .place_sound_at(effect, None, TimelineTime::from_seconds(20), None)
        .unwrap();
    assert_eq!(lanes(&editor).len(), 4);

    // Each drop is one undo step, the lane it made with it.
    editor.undo().unwrap();
    assert_eq!(lanes(&editor).len(), 3);
}

#[test]
fn an_empty_sound_lane_can_be_added() {
    let (mut editor, _rx) = Editor::new_project("Lanes");
    let before = editor.active_sequence().unwrap().audio_tracks.len();
    let lane = editor.add_sound_lane().unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.audio_tracks.len(), before + 1);
    assert_eq!(sequence.audio_tracks.last().unwrap().id, lane);
    assert_eq!(
        sequence.audio_tracks.last().unwrap().name,
        format!("A{}", before + 1)
    );
    editor.undo().unwrap();
    assert_eq!(editor.active_sequence().unwrap().audio_tracks.len(), before);
}
