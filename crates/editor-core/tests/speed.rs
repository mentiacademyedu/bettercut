//! Speed through the editor.
//!
//! The arithmetic is tested in the timeline crate. What matters here is what
//! happens to the *timeline*: that a clip's length follows its speed, that a
//! slower clip is refused when there is no room rather than overlapping its
//! neighbour, that undo restores both the speed and the length, and that
//! trimming and splitting a sped-up clip cut the source where the picture
//! actually is.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, Rational, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{MAX_SPEED, MIN_SPEED, SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor, TrimEdge};

fn ratio(num: i64, den: i64) -> Rational {
    Rational::new(num, den).expect("non-zero denominator")
}

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// One four-second clip at the start of an otherwise empty track.
fn editor_with_a_clip() -> (Editor, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Speed");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;

    let clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::from_seconds(1), MediaTime::from_seconds(5)).unwrap(),
    )
    .unwrap();
    let id = clip.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    (editor, id)
}

fn clip_of(editor: &Editor, id: ClipId) -> &VideoClip {
    editor.video_clip(id).expect("clip")
}

#[test]
fn a_clip_starts_at_normal_speed() {
    let (editor, clip) = editor_with_a_clip();
    assert_eq!(clip_of(&editor, clip).speed, Rational::ONE);
}

/// The visible consequence: twice the speed, half the length. A speed control
/// that left the clip the same length would run out of material half way and
/// freeze on the last frame.
#[test]
fn speeding_up_shortens_the_clip() {
    let (mut editor, clip) = editor_with_a_clip();

    editor.set_clip_speed(clip, ratio(2, 1), false).unwrap();

    let clip = clip_of(&editor, clip);
    assert_eq!(clip.speed, ratio(2, 1));
    assert_eq!(clip.timeline.start, TimelineTime::ZERO, "the start moved");
    assert_eq!(clip.timeline.end, seconds(2));
    assert_eq!(
        clip.source,
        SourceRange::new(MediaTime::from_seconds(1), MediaTime::from_seconds(5)).unwrap(),
        "the material it plays changed"
    );
}

#[test]
fn slowing_down_lengthens_the_clip() {
    let (mut editor, clip) = editor_with_a_clip();
    editor.set_clip_speed(clip, ratio(1, 2), false).unwrap();

    assert_eq!(clip_of(&editor, clip).timeline.end, seconds(8));
}

/// Undo has to put back the length as well as the speed. One without the other
/// leaves a clip whose duration does not match the material it plays.
#[test]
fn undo_restores_the_speed_and_the_length_together() {
    let (mut editor, clip) = editor_with_a_clip();
    editor.set_clip_speed(clip, ratio(4, 1), false).unwrap();

    editor.undo().unwrap();

    let restored = clip_of(&editor, clip);
    assert_eq!(restored.speed, Rational::ONE);
    assert_eq!(restored.timeline.end, seconds(4));

    editor.redo().unwrap();
    let redone = clip_of(&editor, clip);
    assert_eq!(redone.speed, ratio(4, 1));
    assert_eq!(redone.timeline.end, seconds(1));
}

/// The neighbours move with it, per track — the same ripple `ripple_remove`
/// already does (§10). Neither alternative works: leaving them opens a gap on
/// every speed-up, and refusing makes slowing down fail on any track that is
/// not the last one.
#[test]
fn re_timing_ripples_the_rest_of_the_track() {
    let (mut editor, clip) = editor_with_a_clip();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let media = editor.project().media[0].id;

    // Butted right up against the first clip's end.
    let next = VideoClip::new(
        media,
        seconds(4),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    let next_id = next.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(next)))
        .unwrap();

    // Half speed: four seconds becomes eight, so the neighbour moves four right.
    editor.set_clip_speed(clip, ratio(1, 2), false).unwrap();
    assert_eq!(clip_of(&editor, clip).timeline.end, seconds(8));
    assert_eq!(clip_of(&editor, next_id).timeline.start, seconds(8));
    assert_eq!(clip_of(&editor, next_id).timeline.end, seconds(12));

    // And back: undo has to move the neighbour home too, or the ripple is a
    // one-way door.
    editor.undo().unwrap();
    assert_eq!(clip_of(&editor, clip).timeline.end, seconds(4));
    assert_eq!(clip_of(&editor, next_id).timeline.start, seconds(4));
    assert_eq!(clip_of(&editor, next_id).timeline.end, seconds(8));
}

/// Speeding up closes the gap it would otherwise leave. Without this, the
/// commonest thing anyone does — make a clip 2× — leaves a hole in the cut.
#[test]
fn speeding_up_pulls_the_rest_of_the_track_back() {
    let (mut editor, clip) = editor_with_a_clip();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let media = editor.project().media[0].id;

    let next = VideoClip::new(
        media,
        seconds(4),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    let next_id = next.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(next)))
        .unwrap();

    editor.set_clip_speed(clip, ratio(2, 1), false).unwrap();

    assert_eq!(clip_of(&editor, clip).timeline.end, seconds(2));
    assert_eq!(
        clip_of(&editor, next_id).timeline.start,
        seconds(2),
        "speeding up left a gap"
    );
}

/// The clamps live in the model, because §38.2 replays commands after a crash
/// and a limit that only existed in a slider would come back unapplied.
#[test]
fn an_absurd_speed_is_clamped() {
    let (mut editor, clip) = editor_with_a_clip();

    editor.set_clip_speed(clip, ratio(1000, 1), false).unwrap();
    assert_eq!(clip_of(&editor, clip).speed, MAX_SPEED);

    editor.set_clip_speed(clip, ratio(1, 1000), false).unwrap();
    assert_eq!(clip_of(&editor, clip).speed, MIN_SPEED);
}

/// A drag across the slider is one undo step, not sixty (§11).
#[test]
fn dragging_the_slider_collapses_into_one_undo_step() {
    let (mut editor, clip) = editor_with_a_clip();

    for step in 1..=8 {
        editor
            .set_clip_speed(clip, ratio(step + 1, 2), step > 1)
            .unwrap();
    }
    assert_eq!(clip_of(&editor, clip).speed, ratio(9, 2));

    editor.undo().unwrap();
    assert_eq!(
        clip_of(&editor, clip).speed,
        Rational::ONE,
        "one undo did not take the whole drag back"
    );
    assert_eq!(clip_of(&editor, clip).timeline.end, seconds(4));
}

/// Trimming a sped-up clip has to cut the source where the *picture* is. At 2×,
/// dragging the end in by a second removes two seconds of material — unscaled,
/// the clip would keep more source than it has room to play.
#[test]
fn trimming_a_fast_clip_consumes_source_at_that_speed() {
    let (mut editor, clip) = editor_with_a_clip();
    editor.set_clip_speed(clip, ratio(2, 1), false).unwrap();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;

    // Now 0–2 s on the timeline, playing source 1–5 s. Trim to 0–1 s.
    editor
        .trim_clip(track, clip, TrimEdge::End, seconds(1))
        .unwrap();

    let trimmed = clip_of(&editor, clip);
    assert_eq!(trimmed.timeline.end, seconds(1));
    assert_eq!(
        trimmed.source.end,
        MediaTime::from_seconds(3),
        "one second of timeline should have given back two of source"
    );
    // And the invariant that matters: the clip's length still matches what it
    // plays at its speed.
    assert_eq!(trimmed.timeline_duration(), seconds(1));
}

/// The same for splitting: the cut lands where the frame on screen is, not
/// where a normal-speed clip would have been.
#[test]
fn splitting_a_fast_clip_cuts_the_source_at_the_right_frame() {
    let (mut editor, clip) = editor_with_a_clip();
    editor.set_clip_speed(clip, ratio(2, 1), false).unwrap();

    // A second into a clip running 0–2 s at 2×: source 1 s + 2 s = 3 s.
    editor.set_playhead(seconds(1));
    assert_eq!(editor.split_at_playhead(&[clip]).unwrap(), 1);

    let track = &editor.active_sequence().unwrap().video_tracks[0];
    assert_eq!(track.len(), 2);
    let left = &track.clips()[0];
    let right = &track.clips()[1];

    assert_eq!(left.source.end, MediaTime::from_seconds(3));
    assert_eq!(right.source.start, MediaTime::from_seconds(3));
    assert_eq!(left.speed, ratio(2, 1), "the halves lost the speed");
    assert_eq!(right.speed, ratio(2, 1));

    // Both halves still play exactly the material they have room for.
    assert_eq!(left.timeline_duration(), left.timeline.duration());
    assert_eq!(right.timeline_duration(), right.timeline.duration());
}

/// §38.2: an edit lost to a crash is worse than one never made.
#[test]
fn speed_survives_a_save_and_load() {
    let (mut editor, clip) = editor_with_a_clip();
    editor.set_clip_speed(clip, ratio(3, 2), false).unwrap();

    let json = serde_json::to_string(editor.project()).unwrap();
    let loaded: bettercut_editor_core::project_format::Project =
        serde_json::from_str(&json).unwrap();

    let restored = loaded.sequences[0].video_tracks[0]
        .clips()
        .iter()
        .find(|c| c.id == clip)
        .unwrap();
    assert_eq!(restored.speed, ratio(3, 2));
}

/// A project written before speed existed has no such field, and must load as
/// what it was: normal.
#[test]
fn an_older_project_loads_at_normal_speed() {
    let (editor, clip) = editor_with_a_clip();
    let mut json: serde_json::Value = serde_json::to_value(editor.project()).unwrap();
    strip(&mut json, "speed");

    let loaded: bettercut_editor_core::project_format::Project =
        serde_json::from_value(json).unwrap();
    let restored = loaded.sequences[0].video_tracks[0]
        .clips()
        .iter()
        .find(|c| c.id == clip)
        .unwrap();
    assert_eq!(restored.speed, Rational::ONE);
}

fn strip(value: &mut serde_json::Value, field: &str) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove(field);
            for (_, v) in map.iter_mut() {
                strip(v, field);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| strip(v, field)),
        _ => {}
    }
}

/// §25's handles are measured in source, and a crossfade window is measured in
/// timeline. A clip at 2× burns two ticks of footage for every tick of
/// transition, so its handle is worth half as much — offered unscaled, the
/// dissolve would run past the material and flash black.
#[test]
fn a_fast_clip_gets_less_crossfade_from_the_same_handle() {
    use bettercut_editor_core::timeline::TransitionKind;

    let (mut editor, _rx) = Editor::new_project("Speed and transitions");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;

    // A ends a second short of the file; B starts a second in. One second of
    // handle each way, so a normal-speed crossfade can run two seconds.
    let a = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::from_seconds(50), MediaTime::from_seconds(59)).unwrap(),
    )
    .unwrap();
    let a_id = a.id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(a)))
        .unwrap();

    let b = VideoClip::new(
        media,
        seconds(9),
        SourceRange::new(MediaTime::from_seconds(1), MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    editor
        .add_clip(track, ClipPayload::Video(Box::new(b)))
        .unwrap();

    let at_normal = editor
        .transition_room(a_id, TransitionKind::Crossfade)
        .expect("a cut with room");
    // The ripple keeps them butted together, so it is still a cut afterwards.

    assert_eq!(at_normal, seconds(2), "twice the smaller handle");

    editor.set_clip_speed(a_id, ratio(2, 1), false).unwrap();

    let at_double = editor
        .transition_room(a_id, TransitionKind::Crossfade)
        .expect("still a cut");
    assert_eq!(
        at_double,
        seconds(1),
        "a 2x clip's second of footage is only half a second of transition"
    );
}
