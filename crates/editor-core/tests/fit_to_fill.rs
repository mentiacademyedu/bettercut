//! Fit to fill: a clip re-timed to end exactly at the next clip or a chosen time.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, Rational, TimelineTime, TrackId};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A 4 s shot with its sound at 0 s and an 8 s shot at 10 s: a 6 s gap between.
fn two_shots() -> (Editor, TrackId, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Fill");
    let mut short = MediaAsset::new(
        MediaKind::Video,
        "C:/media/short.mp4",
        MediaTime::from_seconds(4),
    );
    short.audio_codec = Some("aac".to_owned());
    let short = editor.import_media(short);
    let long = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/long.mp4",
        MediaTime::from_seconds(8),
    ));
    editor.place_media(short).unwrap();
    editor.place_media(long).unwrap();
    let sequence = editor.active_sequence().unwrap().clone();
    let track = sequence.video_tracks[0].id;
    let first = sequence.video_tracks[0].clips()[0].id;
    let second = sequence.video_tracks[0].clips()[1].id;
    editor.move_clip(track, track, second, seconds(10)).unwrap();
    (editor, track, first, second)
}

fn span(editor: &Editor, clip: ClipId) -> (TimelineTime, TimelineTime) {
    let span = editor.active_sequence().unwrap().clip_span(clip).unwrap();
    (span.timeline.start, span.timeline.end)
}

/// Four seconds of material stretched over ten: 0.4×, ending on the very
/// tick the next clip starts, with the sound stretched alongside.
#[test]
fn a_clip_slows_to_fill_the_gap_after_it() {
    let (mut editor, _track, first, second) = two_shots();
    assert_eq!(editor.fill_end(first), Some(seconds(10)));

    let speed = editor.fit_to_fill(first).unwrap();
    assert_eq!(speed, Rational::new(2, 5).unwrap());
    assert_eq!(span(&editor, first), (seconds(0), seconds(10)));
    let sound = editor
        .linked_with(first)
        .into_iter()
        .find(|c| *c != first)
        .unwrap();
    assert_eq!(
        span(&editor, sound),
        (seconds(0), seconds(10)),
        "the sound was left behind"
    );
    assert_eq!(
        span(&editor, second),
        (seconds(10), seconds(18)),
        "the next clip moved"
    );
    assert_eq!(
        editor.fill_end(first),
        None,
        "a filled gap is still offered"
    );

    editor.undo().unwrap();
    assert_eq!(span(&editor, first), (seconds(0), seconds(4)));
    assert_eq!(span(&editor, sound), (seconds(0), seconds(4)));
}

/// Nothing after the clip, or the next clip touching it, is nothing to fill.
#[test]
fn no_gap_is_nothing_to_fill() {
    let (mut editor, _track, first, second) = two_shots();
    assert_eq!(editor.fill_end(second), None);
    assert!(matches!(
        editor.fit_to_fill(second),
        Err(EditorError::NoGapThere)
    ));

    editor.fit_to_fill(first).unwrap();
    assert!(matches!(
        editor.fit_to_fill(first),
        Err(EditorError::NoGapThere)
    ));
}

/// Ending at a time before the clip's own end speeds it up; one exactly on a
/// frame between gives an exact speed.
#[test]
fn a_clip_speeds_up_to_end_sooner() {
    let (mut editor, _track, first, second) = two_shots();
    // A third shot straight after the second, touching it.
    let third = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/third.mp4",
        MediaTime::from_seconds(2),
    ));
    editor.place_media(third).unwrap();
    let third = editor.active_sequence().unwrap().video_tracks[0].clips()[2].id;
    assert_eq!(span(&editor, third), (seconds(18), seconds(20)));

    let speed = editor
        .fit_to_end(first, TimelineTime::from_millis(2_500))
        .unwrap();
    assert_eq!(speed, Rational::new(8, 5).unwrap());
    assert_eq!(
        span(&editor, first),
        (seconds(0), TimelineTime::from_millis(2_500))
    );
    assert_eq!(
        span(&editor, second),
        (seconds(10), seconds(18)),
        "a later clip moved"
    );
    assert_eq!(
        span(&editor, third),
        (seconds(18), seconds(20)),
        "a later clip moved"
    );
}

/// Past the speed control's limits, or not after the start, nothing changes.
#[test]
fn a_fit_beyond_the_limits_is_refused() {
    let (mut editor, _track, first, second) = two_shots();
    // 8 s of material into 0.5 s would be 16×.
    let (start, _) = span(&editor, second);
    assert!(matches!(
        editor.fit_to_end(second, start + TimelineTime::from_millis(500)),
        Err(EditorError::FillOutOfRange)
    ));
    assert!(matches!(
        editor.fit_to_end(second, start),
        Err(EditorError::FillOutOfRange)
    ));
    // 4 s into 41 s would be slower than 0.1×; into 40 s is exactly 0.1×.
    assert!(matches!(
        editor.fit_to_end(first, seconds(41)),
        Err(EditorError::FillOutOfRange)
    ));
    assert_eq!(span(&editor, first), (seconds(0), seconds(4)));
    assert_eq!(span(&editor, second), (seconds(10), seconds(18)));
}

/// A sound blocked on its own track leaves both picture and sound as they were.
#[test]
fn a_blocked_sound_refuses_the_whole_fit() {
    let (mut editor, _track, first, _second) = two_shots();
    let sound = editor
        .linked_with(first)
        .into_iter()
        .find(|c| *c != first)
        .unwrap();
    let sound_track = editor.track_of(sound).unwrap();
    let music = editor.import_media(MediaAsset::new(
        MediaKind::Audio,
        "C:/media/music.wav",
        MediaTime::from_seconds(3),
    ));
    let clip = bettercut_editor_core::timeline::AudioClip::new(
        music,
        seconds(6),
        bettercut_editor_core::timeline::SourceRange::new(
            MediaTime::ZERO,
            MediaTime::from_seconds(3),
        )
        .unwrap(),
    )
    .unwrap();
    editor
        .add_clip(
            sound_track,
            bettercut_editor_core::ClipPayload::Audio(Box::new(clip)),
        )
        .unwrap();
    assert!(editor.fit_to_fill(first).is_err());
    assert_eq!(span(&editor, first), (seconds(0), seconds(4)));
    assert_eq!(span(&editor, sound), (seconds(0), seconds(4)));
}

/// Every preset speed the clip menu offers applies, and plays the clip for its
/// material's length divided by the speed; the labels read as a menu would.
#[test]
fn every_speed_preset_applies() {
    use bettercut_editor_core::timeline::{SPEED_PRESETS, speed_label};

    for speed in SPEED_PRESETS {
        let (mut editor, _track, first, _second) = two_shots();
        // Far enough apart that slowing down has room without moving anything.
        editor.set_clip_speed(first, speed, false).unwrap();
        let (start, end) = span(&editor, first);
        let expected = TimelineTime::from_seconds(4).ticks() * speed.den() / speed.num();
        assert!(
            ((end.ticks() - start.ticks()) - expected).abs() <= 1,
            "{} lasts {} ticks, not {expected}",
            speed_label(speed),
            end.ticks() - start.ticks()
        );
    }
    let labels: Vec<String> = SPEED_PRESETS.iter().map(|s| speed_label(*s)).collect();
    assert_eq!(
        labels,
        ["0.25×", "0.5×", "0.75×", "1×", "1.5×", "2×", "3×", "4×"]
    );
}
