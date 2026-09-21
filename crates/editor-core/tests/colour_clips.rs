//! Colour clips (`Editor::add_colour_clip`, `Editor::set_colour`): a solid or
//! gradient picture with no file behind it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{Generated, MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

const SUNSET: Generated = Generated::Colour {
    top: [255, 120, 60],
    bottom: [120, 40, 140],
};

#[test]
fn an_empty_lane_takes_it_at_the_playhead() {
    let (mut editor, _events) = Editor::new_project("Colour");
    editor.set_playhead(TimelineTime::from_seconds(2));
    let lanes = editor.active_sequence().unwrap().video_tracks.len();

    let clip = editor.add_colour_clip(SUNSET).unwrap();
    let video = editor.video_clip(clip).unwrap();
    assert_eq!(video.timeline.start, TimelineTime::from_seconds(2));
    assert_eq!(video.timeline.duration(), TimelineTime::from_seconds(5));
    assert_eq!(editor.active_sequence().unwrap().video_tracks.len(), lanes);

    let (media, colour) = editor.colour_of(clip).unwrap();
    assert_eq!(colour, SUNSET);
    let asset = editor.project().media_asset(media).unwrap();
    assert_eq!(asset.file_name, "Gradient #FF783C to #78288C");
    // The sequence's shape, so it covers the frame.
    let resolution = editor.active_sequence().unwrap().resolution;
    assert_eq!(
        (asset.width, asset.height),
        (resolution.width, resolution.height)
    );

    // No file, and nothing to report missing.
    assert_eq!(editor.refresh_missing_media(), 0);
    assert!(!editor.project().media_asset(media).unwrap().missing);
}

#[test]
fn over_footage_it_gets_a_lane_underneath() {
    let (mut editor, _events) = Editor::new_project("Colour");
    let shot = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(10),
    ));
    editor.place_media(shot).unwrap();
    editor.set_playhead(TimelineTime::from_seconds(1));
    let lanes = editor.active_sequence().unwrap().video_tracks.len();
    let depth = editor.undo_depth();

    let clip = editor.add_colour_clip(Generated::solid([0, 0, 0])).unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks.len(), lanes + 1);
    assert_eq!(sequence.video_tracks[0].name, "Background");
    assert!(
        sequence.video_tracks[0].get(clip).is_some(),
        "not at the bottom"
    );
    assert_eq!(editor.undo_depth(), depth + 1, "not one step");

    editor.undo().unwrap();
    assert!(editor.video_clip(clip).is_none());
    assert_eq!(editor.active_sequence().unwrap().video_tracks.len(), lanes);
}

#[test]
fn recolouring_is_undoable_and_a_drag_is_one_step() {
    let (mut editor, _events) = Editor::new_project("Colour");
    let clip = editor.add_colour_clip(Generated::solid([0, 0, 0])).unwrap();
    let (media, _) = editor.colour_of(clip).unwrap();
    let depth = editor.undo_depth();

    assert!(
        editor
            .set_colour(media, Generated::solid([10, 10, 10]), false)
            .unwrap()
    );
    assert!(
        editor
            .set_colour(media, Generated::solid([20, 20, 20]), true)
            .unwrap()
    );
    assert!(editor.set_colour(media, SUNSET, true).unwrap());
    assert!(
        !editor.set_colour(media, SUNSET, false).unwrap(),
        "no change"
    );
    assert_eq!(editor.undo_depth(), depth + 1, "the drag was not one step");
    assert_eq!(editor.colour_of(clip).unwrap().1, SUNSET);
    assert!(
        editor
            .project()
            .media_asset(media)
            .unwrap()
            .file_name
            .starts_with("Gradient")
    );

    editor.undo().unwrap();
    assert_eq!(
        editor.colour_of(clip).unwrap().1,
        Generated::solid([0, 0, 0])
    );
    assert_eq!(
        editor.project().media_asset(media).unwrap().file_name,
        "Colour #000000"
    );
}

#[test]
fn a_file_is_not_a_colour_clip() {
    let (mut editor, _events) = Editor::new_project("Colour");
    let shot = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(10),
    ));
    let clip = editor.place_media(shot).unwrap()[0];
    assert!(editor.colour_of(clip).is_none());
    assert!(matches!(
        editor.set_colour(shot, SUNSET, false),
        Err(EditorError::NotAColourClip)
    ));
}

#[test]
fn it_survives_saving() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("colour.vproj");
    let (mut editor, _events) = Editor::new_project("Colour");
    let clip = editor.add_colour_clip(SUNSET).unwrap();
    editor.save_as(&path).unwrap();

    let (opened, _) = Editor::open(&path).unwrap();
    assert_eq!(opened.colour_of(clip).unwrap().1, SUNSET);
    let (media, _) = opened.colour_of(clip).unwrap();
    assert!(!opened.project().media_asset(media).unwrap().missing);
}
