//! Tidying lanes (`Editor::remove_empty_lanes`, `Editor::merge_lane_down`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::{ClipPayload, Command, TrackKindRepr};
use bettercut_editor_core::foundation::{MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{Editor, EditorError};

fn add_lane(editor: &mut Editor) -> TrackId {
    let sequence = editor.active_sequence().unwrap().id;
    let before: Vec<TrackId> = editor
        .active_sequence()
        .unwrap()
        .video_tracks
        .iter()
        .map(|t| t.id)
        .collect();
    editor
        .dispatch(Command::AddTrack {
            sequence,
            kind: TrackKindRepr::Video,
            name: "Extra".to_owned(),
            id: TrackId::new(),
        })
        .unwrap();
    editor
        .active_sequence()
        .unwrap()
        .video_tracks
        .iter()
        .map(|t| t.id)
        .find(|t| !before.contains(t))
        .unwrap()
}

#[test]
fn lanes_merge_down_and_the_empty_ones_go() {
    let (mut editor, _events) = Editor::new_project("Lanes");
    let media = editor.import_media(bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Image,
        "C:/media/still.png",
        MediaTime::ZERO,
    ));
    let bottom = editor.active_sequence().unwrap().video_tracks[0].id;
    let upper = add_lane(&mut editor);
    let spare = add_lane(&mut editor);
    let one = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(1)).unwrap();
    let low = VideoClip::new(media, TimelineTime::ZERO, one).unwrap();
    let high = VideoClip::new(media, TimelineTime::from_seconds(2), one).unwrap();
    let high_id = high.id;
    editor
        .add_clip(bottom, ClipPayload::Video(Box::new(low)))
        .unwrap();
    editor
        .add_clip(upper, ClipPayload::Video(Box::new(high)))
        .unwrap();

    assert_eq!(editor.lane_below(upper), Some(bottom));
    let depth = editor.undo_depth();
    assert_eq!(editor.merge_lane_down(upper).unwrap(), 1);
    assert_eq!(editor.track_of(high_id), Some(bottom));
    assert_eq!(editor.undo_depth(), depth + 1, "one step");
    assert_eq!(editor.active_sequence().unwrap().video_tracks.len(), 2);

    // The spare lane is empty: it goes, the bottom one stays.
    assert_eq!(editor.remove_empty_lanes().unwrap(), 1);
    let lanes: Vec<TrackId> = editor
        .active_sequence()
        .unwrap()
        .video_tracks
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(lanes, [bottom]);
    assert!(!lanes.contains(&spare));

    // Overlapping clips refuse to share.
    let again = add_lane(&mut editor);
    let clash = VideoClip::new(media, TimelineTime::ZERO, one).unwrap();
    editor
        .add_clip(again, ClipPayload::Video(Box::new(clash)))
        .unwrap();
    assert!(matches!(
        editor.merge_lane_down(again),
        Err(EditorError::LanesOverlap)
    ));
}

#[test]
fn a_lane_moves_up_and_down_and_back_on_undo() {
    let (mut editor, _events) = Editor::new_project("Order");
    let bottom = editor.active_sequence().unwrap().video_tracks[0].id;
    let top = add_lane(&mut editor);
    let order = |editor: &Editor| -> Vec<TrackId> {
        editor
            .active_sequence()
            .unwrap()
            .video_tracks
            .iter()
            .map(|t| t.id)
            .collect()
    };
    assert_eq!(order(&editor), [bottom, top]);

    assert!(
        editor.move_lane(bottom, true).unwrap(),
        "up draws it on top"
    );
    assert_eq!(order(&editor), [top, bottom]);
    assert!(!editor.move_lane(bottom, true).unwrap(), "already on top");

    editor.undo().unwrap();
    assert_eq!(order(&editor), [bottom, top]);

    // Sound lanes are drawn in stored order: down moves it later.
    let sound = editor.active_sequence().unwrap().audio_tracks[0].id;
    assert!(!editor.move_lane(sound, true).unwrap());
}
