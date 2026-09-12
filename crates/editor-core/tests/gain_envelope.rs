//! A sound clip's volume envelope, through the editor (§24, §20a.4).
//!
//! The envelope is what ducking writes and what riding a level by hand writes,
//! so the editor's side of it has to be exact about three things: keys are
//! anchored to source time, replacing an envelope replaces it, and the whole
//! shape is one undo step.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::AnimatedParameter;
use bettercut_editor_core::{Editor, EditorError};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A twenty-second video with its sound, both from timeline zero.
fn editor_with_sound() -> (Editor, ClipId, ClipId) {
    let (mut editor, _events) = Editor::new_project("Envelope");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/music.mp4",
        MediaTime::from_seconds(20),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, placed[0], placed[1])
}

fn gain_at(editor: &Editor, clip: ClipId, at: TimelineTime) -> f32 {
    editor.audio_clip(clip).unwrap().gain_at(at)
}

#[test]
fn an_envelope_is_written_and_read_back() {
    let (mut editor, _picture, sound) = editor_with_sound();

    let count = editor
        .set_gain_envelope(
            sound,
            &[(seconds(0), 1.0), (seconds(4), 0.2), (seconds(8), 1.0)],
            false,
        )
        .unwrap();

    assert_eq!(count, 3);
    assert_eq!(gain_at(&editor, sound, seconds(0)), 1.0);
    assert_eq!(gain_at(&editor, sound, seconds(4)), 0.2);
    assert!(
        (gain_at(&editor, sound, seconds(6)) - 0.6).abs() < 1e-5,
        "halfway back up"
    );
    assert_eq!(
        gain_at(&editor, sound, seconds(12)),
        1.0,
        "held past the last key"
    );
}

/// One shape, one step. Undoing half a duck would leave the music at a level
/// nobody chose.
#[test]
fn a_whole_envelope_is_one_undo_step() {
    let (mut editor, _picture, sound) = editor_with_sound();
    let before = editor.undo_depth();

    editor
        .set_gain_envelope(
            sound,
            &[(seconds(0), 1.0), (seconds(4), 0.2), (seconds(8), 1.0)],
            false,
        )
        .unwrap();
    assert_eq!(editor.undo_depth(), before + 1);

    editor.undo().unwrap();
    assert!(
        !editor
            .audio_clip(sound)
            .unwrap()
            .keyframes
            .is_animated(AnimatedParameter::Gain),
        "undo left some of the envelope behind"
    );
}

/// Writing a new envelope replaces the old one rather than merging with it —
/// otherwise ducking twice would leave the keys of the first duck between the
/// keys of the second, at levels belonging to neither.
#[test]
fn a_second_envelope_replaces_the_first() {
    let (mut editor, _picture, sound) = editor_with_sound();
    editor
        .set_gain_envelope(sound, &[(seconds(2), 0.1), (seconds(6), 0.1)], false)
        .unwrap();

    editor
        .set_gain_envelope(sound, &[(seconds(10), 0.5), (seconds(14), 0.5)], false)
        .unwrap();

    let keys = editor
        .audio_clip(sound)
        .unwrap()
        .keyframes
        .track(AnimatedParameter::Gain)
        .unwrap()
        .keys()
        .len();
    assert_eq!(keys, 2, "the first envelope's keys are still there");
    assert_eq!(
        gain_at(&editor, sound, seconds(4)),
        0.5,
        "the old duck still dips"
    );
}

#[test]
fn an_empty_envelope_clears_it() {
    let (mut editor, _picture, sound) = editor_with_sound();
    editor
        .set_gain_envelope(sound, &[(seconds(2), 0.1)], false)
        .unwrap();

    assert_eq!(editor.set_gain_envelope(sound, &[], false).unwrap(), 0);
    assert!(
        !editor
            .audio_clip(sound)
            .unwrap()
            .keyframes
            .is_animated(AnimatedParameter::Gain)
    );
    // Back to the clip's own volume, not to silence.
    assert_eq!(gain_at(&editor, sound, seconds(2)), 1.0);
}

/// Clearing an envelope that is not there is not an edit.
#[test]
fn clearing_nothing_changes_nothing() {
    let (mut editor, _picture, sound) = editor_with_sound();
    let before = editor.undo_depth();

    assert_eq!(editor.set_gain_envelope(sound, &[], false).unwrap(), 0);
    assert_eq!(editor.undo_depth(), before);
}

/// §24: keys are anchored to source time, so trimming the clip's start slides
/// it along its own envelope instead of dragging the envelope with it — the
/// duck stays over the words it was put on.
#[test]
fn the_envelope_is_anchored_to_the_source() {
    let (mut editor, picture, sound) = editor_with_sound();
    editor
        .set_gain_envelope(sound, &[(seconds(0), 1.0), (seconds(10), 0.0)], false)
        .unwrap();

    // Trim two seconds off the front of the pair (§12 carries the picture).
    let track = editor.track_of(picture).unwrap();
    editor
        .trim_clip(
            track,
            picture,
            bettercut_editor_core::TrimEdge::Start,
            seconds(2),
        )
        .unwrap();

    // Four seconds in is six seconds of source: still 0.4 on the ramp.
    assert!(
        (gain_at(&editor, sound, seconds(6)) - 0.4).abs() < 1e-5,
        "the envelope moved with the trim: {}",
        gain_at(&editor, sound, seconds(6))
    );
}

/// A picture has no volume and a sound has no opacity. Accepting a key nothing
/// would ever read is worse than refusing it.
#[test]
fn a_video_clip_has_no_volume_envelope() {
    let (mut editor, picture, _sound) = editor_with_sound();

    let refused = editor.set_gain_envelope(picture, &[(seconds(1), 0.5)], false);

    assert!(
        matches!(refused, Err(EditorError::ClipKindMismatch)),
        "a video clip accepted a volume envelope: {refused:?}"
    );
}

/// Volume is the one parameter a sound clip has. The rest describe a picture,
/// and a key nothing would ever read is worse than a refusal — it would sit in
/// the project file looking like an effect that had stopped working.
#[test]
fn a_sound_clip_takes_no_picture_keyframes() {
    let (mut editor, _picture, sound) = editor_with_sound();
    let key = bettercut_editor_core::timeline::Keyframe::new(
        MediaTime::from_seconds(1),
        0.5,
        bettercut_editor_core::timeline::Interpolation::Linear,
    );

    let refused = editor.set_keyframe(sound, AnimatedParameter::Opacity, key);
    assert!(
        matches!(refused, Err(EditorError::ClipKindMismatch)),
        "a sound clip accepted an opacity key: {refused:?}"
    );

    // And the one it does have is accepted.
    editor
        .set_keyframe(sound, AnimatedParameter::Gain, key)
        .expect("a sound clip takes a volume key");
    assert_eq!(gain_at(&editor, sound, seconds(1)), 0.5);
}
