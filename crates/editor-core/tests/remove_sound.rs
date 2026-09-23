//! A picture's own sound removed, and every lane turned back on.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::TrackFlag;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn the_sound_goes_and_the_picture_stays() {
    let (mut editor, _events) = Editor::new_project("Scratch");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(3),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    let (picture, sound) = (placed[0], placed[1]);
    assert!(editor.audio_clip(sound).is_some());

    assert_eq!(editor.remove_linked_sound(picture).unwrap(), 1);
    assert!(editor.audio_clip(sound).is_none());
    assert!(editor.video_clip(picture).is_some());
    editor.undo().unwrap();
    assert!(
        editor.audio_clip(sound).is_some(),
        "one undo brings it back"
    );
}

#[test]
fn every_lane_comes_back_on_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Lanes");
    let lane = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .set_track_flag(lane, TrackFlag::Enabled, false)
        .unwrap();
    assert_eq!(
        editor
            .set_all_tracks_flag(TrackFlag::Enabled, true)
            .unwrap(),
        1
    );
    assert!(editor.track_flag(lane, TrackFlag::Enabled));
}

#[test]
fn the_whole_selection_loses_its_sound_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Montage");
    let mut pictures = Vec::new();
    for name in ["a", "b"] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(3),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        pictures.push(editor.place_media(media).unwrap()[0]);
    }
    let depth = editor.undo_depth();
    assert_eq!(editor.remove_sound_of(&pictures).unwrap(), 2);
    assert!(
        editor.active_sequence().unwrap().audio_tracks[0]
            .clips()
            .is_empty()
    );
    assert_eq!(editor.undo_depth(), depth + 1);
}
