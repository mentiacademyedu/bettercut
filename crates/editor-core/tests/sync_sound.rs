//! Lining a clip up with another by their sound (`Editor::sync_to_sound`).
//!
//! The listening happens in the cache crate; what is checked here is the
//! arithmetic that turns "these files match five seconds apart" into "this
//! clip starts here".

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor, EditorError};

fn secs(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A clip of a 60-second file, placed at `start`, playing from `from`.
fn place(editor: &mut Editor, track: TrackId, name: &str, start: i64, from: i64) -> ClipId {
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        format!("C:/media/{name}.mp4"),
        MediaTime::from_seconds(60),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let source = SourceRange::new(
        MediaTime::from_seconds(from),
        MediaTime::from_seconds(from + 10),
    )
    .unwrap();
    let clip = VideoClip::new(media, secs(start), source).unwrap();
    let id = clip.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    id
}

/// Two cameras on two lanes: the reference from 20 s into its file, placed at
/// 5 s; the other from the start of its file, placed wherever.
fn two_cameras() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Sync");
    editor.add_video_track("V2").unwrap();
    let sequence = editor.active_sequence().unwrap();
    let (first, second) = (sequence.video_tracks[0].id, sequence.video_tracks[1].id);
    let reference = place(&mut editor, first, "main", 5, 20);
    let other = place(&mut editor, second, "second", 40, 0);
    (editor, other, reference)
}

#[test]
fn a_clip_moves_to_where_its_sound_matches() {
    let (mut editor, other, reference) = two_cameras();
    let depth = editor.undo_depth();

    // The second camera's file matches the reference's file 22 seconds later
    // in it: its own zero is the reference's 22 s.
    let start = editor.sync_to_sound(other, reference, 22.0).unwrap();

    // The reference plays from 20 s at timeline 5 s, so the reference's 22 s
    // is at timeline 7 s — and that is where the other clip's start belongs.
    assert_eq!(start, secs(7));
    assert_eq!(editor.video_clip(other).unwrap().timeline.start, secs(7));
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(editor.video_clip(other).unwrap().timeline.start, secs(40));
}

/// The other way round: a recording that began before the reference's own
/// in-point lands earlier on the timeline.
#[test]
fn a_recording_that_started_earlier_lands_earlier() {
    let (editor, other, reference) = two_cameras();
    let start = editor.sync_start(other, reference, 18.0).unwrap();
    assert_eq!(start, secs(3));
}

#[test]
fn lining_up_before_the_start_of_the_timeline_is_refused() {
    let (mut editor, other, reference) = two_cameras();
    // Matching ten seconds *before* the reference's in-point would put the
    // clip at -5 s.
    assert!(matches!(
        editor.sync_to_sound(other, reference, 10.0),
        Err(EditorError::NoRoomToSync)
    ));
    assert_eq!(editor.video_clip(other).unwrap().timeline.start, secs(40));
}
