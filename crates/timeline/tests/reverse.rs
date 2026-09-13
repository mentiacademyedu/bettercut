//! Reversed clips (`VideoClip::reversed`, `bettercut_timeline::source_time`).
//!
//! The invariant everything else rests on: **an edit never changes what is on
//! screen at any instant it keeps.** Splitting a reversed clip, or trimming
//! either end, must leave every remaining instant showing exactly the frame it
//! showed before — which, backwards, means the opposite source edge moves from
//! the one a forward clip would move. Asserted tick for tick across the clip,
//! at normal speed and at 2×.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{ClipId, MediaId, MediaTime, Rational, TimelineTime};
use bettercut_timeline::{SourceRange, TimelineError, VideoClip, VideoTrack};

/// A clip at 2 s on the timeline playing 1 s..9 s of its file, at `speed`,
/// backwards.
fn reversed_clip(speed: Rational) -> VideoClip {
    let mut clip = VideoClip::new(
        MediaId::new(),
        TimelineTime::from_seconds(2),
        SourceRange::new(MediaTime::from_seconds(1), MediaTime::from_seconds(9)).unwrap(),
    )
    .unwrap();
    clip.speed = speed;
    let length = bettercut_timeline::timeline_ticks_for(clip.source.duration(), speed);
    clip.timeline.end = TimelineTime::from_ticks(clip.timeline.start.ticks() + length);
    clip.reversed = true;
    clip
}

fn track_with(clip: &VideoClip) -> VideoTrack {
    let mut track = VideoTrack::new("V1");
    track.insert(clip.clone()).unwrap();
    track
}

/// Every hundredth of a second across `range`.
fn instants(start: TimelineTime, end: TimelineTime) -> impl Iterator<Item = TimelineTime> {
    (start.ticks()..end.ticks())
        .step_by(9_600)
        .map(TimelineTime::from_ticks)
}

/// What the track shows at `at`: the source instant of whichever clip covers it.
fn shown(track: &VideoTrack, at: TimelineTime) -> Option<MediaTime> {
    track.clip_at(at).map(|clip| clip.source_time_at(at))
}

/// Backwards means the first instant shows the last frame of the material and
/// the last instant the first; forwards is untouched.
#[test]
fn a_reversed_clip_runs_its_material_from_the_end() {
    for speed in [Rational::ONE, Rational::new(2, 1).unwrap()] {
        let clip = reversed_clip(speed);
        assert_eq!(
            clip.source_time_at(clip.timeline.start),
            MediaTime::from_ticks(clip.source.end.ticks() - 1),
            "at {speed}"
        );
        let last = TimelineTime::from_ticks(clip.timeline.end.ticks() - 1);
        let at_last = clip.source_time_at(last);
        assert!(
            at_last >= clip.source.start && at_last.ticks() - clip.source.start.ticks() <= 2,
            "at {speed} the last instant shows {at_last:?}"
        );

        let forward = VideoClip {
            reversed: false,
            ..clip.clone()
        };
        assert_eq!(
            forward.source_time_at(forward.timeline.start),
            forward.source.start
        );
    }
}

/// Splitting a reversed clip anywhere leaves every instant showing what it
/// showed — the left half keeps the later material.
#[test]
fn splitting_a_reversed_clip_changes_nothing_on_screen() {
    for speed in [Rational::ONE, Rational::new(2, 1).unwrap()] {
        let clip = reversed_clip(speed);
        let whole = track_with(&clip);
        let at = TimelineTime::from_ticks(clip.timeline.start.ticks() + 1_234_567);

        let mut split = whole.clone();
        split
            .split(clip.id, at, ClipId::new(), ClipId::new())
            .unwrap();

        for instant in instants(clip.timeline.start, clip.timeline.end) {
            assert_eq!(
                shown(&split, instant),
                shown(&whole, instant),
                "at {speed}, {instant:?} changed after a split"
            );
        }
        let left = split.clip_at(clip.timeline.start).unwrap();
        assert!(
            left.reversed && left.source.end == clip.source.end,
            "the left half lost the later material"
        );
    }
}

/// Trimming either end of a reversed clip, inward, leaves every remaining
/// instant showing what it showed.
#[test]
fn trimming_a_reversed_clip_changes_nothing_on_screen() {
    for speed in [Rational::ONE, Rational::new(2, 1).unwrap()] {
        let clip = reversed_clip(speed);
        let whole = track_with(&clip);
        let new_start = TimelineTime::from_ticks(clip.timeline.start.ticks() + 700_000);
        let new_end = TimelineTime::from_ticks(clip.timeline.end.ticks() - 500_000);

        let mut trimmed = whole.clone();
        trimmed.trim_start(clip.id, new_start).unwrap();
        trimmed.trim_end(clip.id, new_end, None).unwrap();

        for instant in instants(new_start, new_end) {
            assert_eq!(
                shown(&trimmed, instant),
                shown(&whole, instant),
                "at {speed}, {instant:?} changed after a trim"
            );
        }
    }
}

/// Extending a reversed clip's end reaches back into the material, and past
/// the start of the file that is refused, as extending a forward clip's start
/// is.
#[test]
fn a_reversed_clip_cannot_extend_before_its_file_starts() {
    let clip = reversed_clip(Rational::ONE);
    let mut track = track_with(&clip);
    let too_far =
        TimelineTime::from_ticks(clip.timeline.end.ticks() + MediaTime::from_seconds(2).ticks());

    let result = track.trim_end(clip.id, too_far, None);

    assert!(
        matches!(result, Err(TimelineError::BeyondSourceStart { .. })),
        "{result:?}"
    );
}

/// Keys ride on the frames of a reversed clip: trimming its start does not
/// slide an animation along the timeline, and a key maps back to the instant
/// it is evaluated at.
#[test]
fn trimming_a_reversed_clip_leaves_its_keys_in_place() {
    use bettercut_timeline::{AnimatedParameter, Interpolation, Keyframe};

    let mut clip = reversed_clip(Rational::ONE);
    let key_at = TimelineTime::from_seconds(6);
    let key = clip.progress_time_at(key_at);
    clip.keyframes.set(
        AnimatedParameter::Opacity,
        Keyframe::new(key, 0.25, Interpolation::Linear),
    );
    assert_eq!(clip.timeline_time_of(key), key_at);

    let mut track = track_with(&clip);
    track
        .trim_start(clip.id, TimelineTime::from_seconds(3))
        .unwrap();
    let trimmed = track.get(clip.id).unwrap();

    assert_eq!(
        trimmed.timeline_time_of(key),
        key_at,
        "the key slid when the start was trimmed"
    );
}
