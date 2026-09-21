//! Making a shot fit a length by re-timing it
//! (`bettercut_editor_core::rate_stretch`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// Two five-second shots with their own sound, butted together.
fn two_shots() -> (Editor, Vec<ClipId>) {
    let (mut editor, _events) = Editor::new_project("Stretch");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(5),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let clips = (0..2)
        .map(|_| editor.place_media(media).unwrap()[0])
        .collect();
    (editor, clips)
}

fn span(editor: &Editor, clip: ClipId) -> (i64, i64) {
    let range = editor
        .active_sequence()
        .unwrap()
        .clip_span(clip)
        .unwrap()
        .timeline;
    (range.start.ticks(), range.end.ticks())
}

fn source_of(editor: &Editor, clip: ClipId) -> (i64, i64) {
    let video = editor.video_clip(clip).unwrap();
    (video.source.start.ticks(), video.source.end.ticks())
}

/// Shorter: the footage is all still there, and it plays faster.
#[test]
fn stretching_a_shot_shorter_speeds_it_up() {
    let (mut editor, clips) = two_shots();
    let material = source_of(&editor, clips[0]);

    let speed = editor
        .rate_stretch(clips[0], seconds(4), false)
        .expect("stretched");

    assert_eq!(span(&editor, clips[0]), (0, seconds(4).ticks()));
    assert_eq!(
        source_of(&editor, clips[0]),
        material,
        "a stretch must not trim the footage"
    );
    // Five seconds of material in four: a quarter faster.
    assert!((speed.as_f64() - 1.25).abs() < 0.01, "{}", speed.as_f64());
}

/// And longer: it plays slower, and the shot after it moves along.
#[test]
fn stretching_a_shot_longer_slows_it_down_and_ripples() {
    let (mut editor, clips) = two_shots();

    let speed = editor
        .rate_stretch(clips[0], seconds(10), false)
        .expect("stretched");

    assert_eq!(span(&editor, clips[0]), (0, seconds(10).ticks()));
    assert!((speed.as_f64() - 0.5).abs() < 0.01, "{}", speed.as_f64());
    assert_eq!(
        span(&editor, clips[1]).0,
        seconds(10).ticks(),
        "the shot after it should have moved along"
    );
}

/// The sound is re-timed with the picture (§12), or they drift apart.
#[test]
fn the_sound_is_stretched_too() {
    let (mut editor, clips) = two_shots();
    let sound = editor.sound_of(clips[0]).expect("linked sound");

    editor
        .rate_stretch(clips[0], seconds(4), false)
        .expect("stretched");

    assert_eq!(span(&editor, sound), (0, seconds(4).ticks()));
}

/// The number the interface shows while dragging is the number the edit will
/// land on.
#[test]
fn the_speed_can_be_read_before_it_is_applied() {
    let (mut editor, clips) = two_shots();
    let shown = editor
        .rate_stretch_speed(clips[0], seconds(4))
        .expect("a speed");

    let applied = editor
        .rate_stretch(clips[0], seconds(4), false)
        .expect("stretched");

    assert_eq!(shown, applied);
}

/// A drag past the speed limits stops there rather than being refused.
#[test]
fn a_stretch_past_the_limits_is_held_to_them() {
    let (mut editor, clips) = two_shots();
    let fastest = bettercut_editor_core::timeline::MAX_SPEED;

    // A hundredth of a second for five seconds of material is far past it.
    let speed = editor
        .rate_stretch(clips[0], TimelineTime::from_millis(10), false)
        .expect("stretched");

    assert_eq!(speed, fastest);
}

/// A photo has no motion to re-time: stretching it is an ordinary trim, and
/// saying so is better than quietly doing nothing.
#[test]
fn a_photo_has_nothing_to_re_time() {
    let (mut editor, _events) = Editor::new_project("Photo");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/still.jpg",
        MediaTime::ZERO,
    ));
    let clip = editor.place_media(media).unwrap()[0];

    assert!(editor.rate_stretch_speed(clip, seconds(2)).is_none());
    assert!(matches!(
        editor.rate_stretch(clip, seconds(2), false).unwrap_err(),
        bettercut_editor_core::EditorError::NoMotionToRetime
    ));
}

/// Dragging the end back past the start is not a length at all.
#[test]
fn an_end_before_the_start_is_refused() {
    let (mut editor, clips) = two_shots();
    let track = editor.track_of(clips[1]).unwrap();
    editor
        .move_clip(track, track, clips[1], seconds(20))
        .expect("moved");

    assert!(editor.rate_stretch_speed(clips[1], seconds(19)).is_none());
}

/// One drag is one undo step (§11), and undo puts the speed and the length
/// back together.
#[test]
fn a_drag_collapses_into_one_undo_step() {
    let (mut editor, clips) = two_shots();
    let before = span(&editor, clips[0]);

    editor.rate_stretch(clips[0], seconds(6), false).unwrap();
    for end in [7, 8, 9] {
        editor.rate_stretch(clips[0], seconds(end), true).unwrap();
    }
    assert_eq!(span(&editor, clips[0]), (0, seconds(9).ticks()));

    editor.undo().expect("undo");
    assert_eq!(span(&editor, clips[0]), before);
}
