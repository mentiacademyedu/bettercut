//! Every lane at once (`Editor::set_all_tracks_flag`) and clips by colour
//! (`Editor::clips_with_colour`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::timeline::ColorLabel;
use bettercut_editor_core::{Editor, TrackFlag};
use bettercut_media::{MediaAsset, MediaKind};

#[test]
fn every_kind_of_lane_locks_and_unlocks_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Lanes");
    let lanes: Vec<_> = {
        let s = editor.active_sequence().unwrap();
        s.video_tracks
            .iter()
            .map(|t| t.id)
            .chain(s.audio_tracks.iter().map(|t| t.id))
            .chain(s.text_tracks.iter().map(|t| t.id))
            .chain(s.adjustment_tracks.iter().map(|t| t.id))
            .collect()
    };
    assert!(lanes.len() >= 2, "a new project has more than one lane");
    let depth = editor.undo_depth();

    let locked = editor.set_all_tracks_flag(TrackFlag::Locked, true).unwrap();
    assert_eq!(locked, lanes.len());
    assert!(
        lanes
            .iter()
            .all(|t| editor.track_flag(*t, TrackFlag::Locked))
    );
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(
        editor.set_all_tracks_flag(TrackFlag::Locked, true).unwrap(),
        0
    );
    assert_eq!(editor.undo_depth(), depth + 1, "already so: no step");

    editor.undo().unwrap();
    assert!(
        lanes
            .iter()
            .all(|t| !editor.track_flag(*t, TrackFlag::Locked))
    );

    editor
        .set_track_flag(lanes[0], TrackFlag::Locked, true)
        .unwrap();
    assert_eq!(
        editor.set_all_tracks_flag(TrackFlag::Locked, true).unwrap(),
        lanes.len() - 1
    );
    assert_eq!(
        editor
            .set_all_tracks_flag(TrackFlag::Locked, false)
            .unwrap(),
        lanes.len()
    );
}

#[test]
fn clips_are_found_by_their_colour_across_lanes() {
    let (mut editor, _events) = Editor::new_project("Colours");
    let mut placed = Vec::new();
    for name in ["a", "b", "c"] {
        let mut asset = MediaAsset::new(
            MediaKind::Video,
            format!("C:/media/{name}.mp4"),
            MediaTime::from_seconds(2),
        );
        asset.audio_codec = Some("aac".to_owned());
        let media = editor.import_media(asset);
        editor.set_playhead(TimelineTime::from_seconds(placed.len() as i64 * 2));
        placed.push(editor.place_media(media).unwrap());
    }
    // The first shot red (its sound comes along), the third green.
    editor
        .set_color_label(&[placed[0][0]], ColorLabel::Red)
        .unwrap();
    editor
        .set_color_label(&[placed[2][0]], ColorLabel::Green)
        .unwrap();

    let mut red = editor.clips_with_colour(ColorLabel::Red);
    red.sort();
    let mut expected = placed[0].clone();
    expected.sort();
    assert_eq!(red, expected, "the red shot and its sound");
    assert_eq!(editor.clips_with_colour(ColorLabel::Green).len(), 2);
    assert!(editor.clips_with_colour(ColorLabel::Blue).is_empty());
}
