//! Closing the gaps on every lane at once (`Editor::close_gaps_everywhere`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};
use bettercut_editor_core::{Editor, EditorError};

#[test]
fn every_lane_is_butted_up_as_one_step() {
    let (mut editor, _events) = Editor::new_project("Everywhere");
    let media = editor.import_media(bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Image,
        "C:/media/still.png",
        MediaTime::ZERO,
    ));
    let sequence = editor.active_sequence().unwrap();
    let (pictures, sounds) = (sequence.video_tracks[0].id, sequence.audio_tracks[0].id);
    let one = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(1)).unwrap();
    let at = TimelineTime::from_seconds;
    let a = VideoClip::new(media, at(0), one).unwrap();
    let b = VideoClip::new(media, at(3), one).unwrap();
    let c = AudioClip::new(media, at(2), one).unwrap();
    let (b_id, c_id) = (b.id, c.id);
    editor
        .add_clip(pictures, ClipPayload::Video(Box::new(a)))
        .unwrap();
    editor
        .add_clip(pictures, ClipPayload::Video(Box::new(b)))
        .unwrap();
    editor
        .add_clip(sounds, ClipPayload::Audio(Box::new(c)))
        .unwrap();
    let depth = editor.undo_depth();

    assert_eq!(editor.close_gaps_everywhere().unwrap(), 2);
    assert_eq!(editor.clip_start(b_id), Some(at(1)));
    assert_eq!(editor.clip_start(c_id), Some(at(0)));
    assert_eq!(editor.undo_depth(), depth + 1);
    assert!(matches!(
        editor.close_gaps_everywhere(),
        Err(EditorError::NoGapThere)
    ));
}
