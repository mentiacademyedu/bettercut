//! Slip edit (`Editor::slip_clip`): same place and length on the timeline, a
//! different stretch of the file.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TICKS_PER_SECOND, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AnimatedParameter, Interpolation, Keyframe};
use bettercut_editor_core::{Command, Editor, EditorError, TrimEdge};

fn source_seconds(editor: &Editor, clip: ClipId) -> (f64, f64) {
    let source = editor
        .video_clip(clip)
        .map(|c| c.source)
        .or_else(|| editor.audio_clip(clip).map(|c| c.source))
        .unwrap();
    let s = |t: MediaTime| t.ticks() as f64 / TICKS_PER_SECOND as f64;
    (s(source.start), s(source.end))
}

/// A 10-second shot with sound, trimmed to play seconds 2–6 of its file.
fn trimmed_shot() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Slip");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/slip.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let picture = editor.place_media(media).unwrap()[0];
    let track = editor.track_of(picture).unwrap();
    editor
        .trim_clip(
            track,
            picture,
            TrimEdge::Start,
            TimelineTime::from_seconds(2),
        )
        .unwrap();
    editor
        .trim_clip(track, picture, TrimEdge::End, TimelineTime::from_seconds(6))
        .unwrap();
    let sound = editor
        .linked_with(picture)
        .into_iter()
        .find(|c| *c != picture)
        .unwrap();
    (editor, picture, sound)
}

#[test]
fn slipping_plays_a_later_stretch_without_moving_the_clip() {
    let (mut editor, picture, sound) = trimmed_shot();
    assert_eq!(source_seconds(&editor, picture), (2.0, 6.0));
    let span = editor.video_clip(picture).unwrap().timeline;
    let depth = editor.undo_depth();

    editor.slip_clip(picture, TICKS_PER_SECOND).unwrap();

    assert_eq!(source_seconds(&editor, picture), (3.0, 7.0));
    assert_eq!(editor.video_clip(picture).unwrap().timeline, span);
    // The sound slips with the picture, so they stay in sync.
    assert_eq!(source_seconds(&editor, sound), (3.0, 7.0));
    assert_eq!(editor.audio_clip(sound).unwrap().timeline, span);
    assert_eq!(editor.undo_depth(), depth + 1);

    editor.undo().unwrap();
    assert_eq!(source_seconds(&editor, picture), (2.0, 6.0));
    assert_eq!(source_seconds(&editor, sound), (2.0, 6.0));
}

#[test]
fn a_slip_stops_at_either_end_of_the_file() {
    let (mut editor, picture, _) = trimmed_shot();
    assert_eq!(
        editor.slip_room(picture),
        Some((-2 * TICKS_PER_SECOND, 4 * TICKS_PER_SECOND))
    );

    editor.slip_clip(picture, 100 * TICKS_PER_SECOND).unwrap();
    assert_eq!(source_seconds(&editor, picture), (6.0, 10.0));

    editor.slip_clip(picture, -100 * TICKS_PER_SECOND).unwrap();
    assert_eq!(source_seconds(&editor, picture), (0.0, 4.0));

    // Already against the start: nothing to do, and no undo step for it.
    let depth = editor.undo_depth();
    editor.slip_clip(picture, -TICKS_PER_SECOND).unwrap();
    assert_eq!(editor.undo_depth(), depth);
}

#[test]
fn keyframes_stay_where_they_were_on_the_timeline() {
    let (mut editor, picture, _) = trimmed_shot();
    let sequence = editor.active_sequence().unwrap().id;
    let track = editor.track_of(picture).unwrap();
    editor
        .dispatch(Command::SetKeyframe {
            sequence,
            track,
            clip: picture,
            parameter: AnimatedParameter::Opacity,
            key: Keyframe::new(MediaTime::from_seconds(3), 0.5, Interpolation::Linear),
        })
        .unwrap();

    editor.slip_clip(picture, 2 * TICKS_PER_SECOND).unwrap();

    let clip = editor.video_clip(picture).unwrap();
    let times = clip.keyframes.times();
    assert_eq!(times, vec![MediaTime::from_seconds(5)]);
    // One second into the clip before, one second into the clip after.
    assert_eq!(
        times[0].ticks() - clip.source.start.ticks(),
        TICKS_PER_SECOND
    );

    editor.undo().unwrap();
    assert_eq!(
        editor.video_clip(picture).unwrap().keyframes.times(),
        vec![MediaTime::from_seconds(3)]
    );
}

#[test]
fn a_photo_has_nothing_to_slip() {
    let (mut editor, _events) = Editor::new_project("Slip");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/photo.png",
        MediaTime::from_seconds(1),
    ));
    let photo = editor.place_media(media).unwrap()[0];
    assert_eq!(editor.slip_room(photo), None);
    assert!(matches!(
        editor.slip_clip(photo, TICKS_PER_SECOND),
        Err(EditorError::NothingToSlip)
    ));
}
