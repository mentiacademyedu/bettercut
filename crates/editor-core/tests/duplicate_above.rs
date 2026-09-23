//! A picture copied onto the lane above (`Editor::duplicate_onto_lane_above`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};

#[test]
fn the_copy_sits_over_the_shot_on_a_lane_of_its_own_without_sound() {
    let (mut editor, _events) = Editor::new_project("Layers");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(3),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let picture = editor.place_media(media).unwrap()[0];
    let lanes = editor.active_sequence().unwrap().video_tracks.len();
    let depth = editor.undo_depth();

    let copy = editor.duplicate_onto_lane_above(picture).unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks.len(), lanes + 1, "a lane was made");
    assert_eq!(editor.track_of(copy), Some(sequence.video_tracks[1].id));
    assert_eq!(editor.clip_start(copy), editor.clip_start(picture));
    assert_eq!(editor.linked_with(copy), [copy], "the copy has no sound");
    assert_eq!(editor.undo_depth(), depth + 1, "one step");

    // Again, from the original: that lane is taken now, so another goes in
    // between, directly above the shot.
    let second = editor.duplicate_onto_lane_above(picture).unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks.len(), lanes + 2);
    assert_eq!(editor.track_of(second), Some(sequence.video_tracks[1].id));
}
