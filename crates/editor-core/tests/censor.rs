//! Censoring part of a shot (`Editor::censor_clip`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::ClipProperty;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::MaskShape;
use bettercut_editor_core::{Editor, EditorError};

fn with_shot() -> (Editor, bettercut_editor_core::foundation::ClipId) {
    let (mut editor, _events) = Editor::new_project("Censor");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/street.mp4",
        MediaTime::from_seconds(6),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    editor.place_media(media).unwrap();
    let clip = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    (editor, clip)
}

/// With nothing above, a lane is made just above the shot, holding a
/// pixelated, masked copy at the same time and framing — one undo step.
#[test]
fn a_censor_is_a_masked_pixelated_copy_above() {
    let (mut editor, clip) = with_shot();
    let tracks_before = editor.active_sequence().unwrap().video_tracks.len();
    let depth = editor.undo_depth();

    let copy = editor.censor_clip(clip).unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks.len(), tracks_before + 1);
    assert_eq!(editor.undo_depth(), depth + 1);
    let lane = &sequence.video_tracks[1];
    let patch = lane.get(copy).expect("the copy is not on the lane above");
    let original = editor.video_clip(clip).unwrap();
    assert_eq!(patch.timeline, original.timeline);
    assert_eq!(patch.source, original.source);
    assert_eq!(patch.transform, original.transform);
    assert!(patch.pixelate > 0.0);
    assert_eq!(patch.mask.unwrap().shape, MaskShape::Ellipse);
    assert!(patch.link.is_none(), "the copy is tied to the shot's sound");
    assert!(editor.video_clip(clip).unwrap().link.is_some());

    editor.undo().unwrap();
    assert_eq!(
        editor.active_sequence().unwrap().video_tracks.len(),
        tracks_before
    );
}

/// A free lane above is used rather than making another; a busy one is
/// passed over.
#[test]
fn a_free_lane_above_is_used() {
    let (mut editor, clip) = with_shot();
    let first = editor.censor_clip(clip).unwrap();
    let tracks = editor.active_sequence().unwrap().video_tracks.len();

    // The first censor's lane is busy over the shot: a second gets a new
    // lane just above the shot, so anything already over it stays over it.
    let second = editor.censor_clip(clip).unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks.len(), tracks + 1);
    assert!(sequence.video_tracks[1].get(second).is_some());
    assert!(sequence.video_tracks[2].get(first).is_some());

    // Remove the second's clip but keep its lane: a third reuses that lane.
    let lane = sequence.video_tracks[1].id;
    editor.remove_clip(lane, second).unwrap();
    let third = editor.censor_clip(clip).unwrap();
    let sequence = editor.active_sequence().unwrap();
    assert_eq!(
        sequence.video_tracks.len(),
        tracks + 1,
        "a lane was made needlessly"
    );
    assert!(sequence.video_tracks[1].get(third).is_some());
}

/// A blurred censor is soft rather than blocky, over the same oval.
#[test]
fn a_censor_can_blur_instead() {
    use bettercut_editor_core::censor::{CENSOR_BLUR, CensorStyle};

    let (mut editor, clip) = with_shot();
    let copy = editor.censor_clip_with(clip, CensorStyle::Blur).unwrap();
    let patch = editor.video_clip(copy).unwrap();
    assert_eq!(patch.pixelate, 0.0);
    assert_eq!(patch.blur, CENSOR_BLUR);
    assert_eq!(patch.mask.unwrap().shape, MaskShape::Ellipse);
}

/// The pixelate amount is held to its range and refused on sound.
#[test]
fn pixelate_is_held_and_picture_only() {
    let (mut editor, clip) = with_shot();
    editor
        .set_clip_property(clip, ClipProperty::Pixelate(500.0), false)
        .unwrap();
    assert_eq!(editor.video_clip(clip).unwrap().pixelate, 100.0);
    let sound = editor.active_sequence().unwrap().audio_tracks[0].clips()[0].id;
    assert!(
        editor
            .set_clip_property(sound, ClipProperty::Pixelate(10.0), false)
            .is_err()
    );
    assert!(matches!(
        editor.censor_clip(sound),
        Err(EditorError::ClipKindMismatch)
    ));
    let _ = TimelineTime::ZERO;
}
