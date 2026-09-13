//! Which adjustments are running, and what they grade (`crate::adjustment`).
//!
//! The renderer is tested on its own for *how* a grade is drawn. These are the
//! two decisions made before that, and made once for both the preview and the
//! export (§46): which adjustments apply at an instant, and how far up the stack
//! of drawn layers they reach.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::TimelineTime;
use bettercut_playback::{adjustments_at, graded_beneath};
use bettercut_timeline::{AdjustmentClip, AdjustmentLook, AdjustmentTrack, ColorAdjust, Sequence};

/// A sequence with one adjustment lane holding a warming grade from 2 s to 5 s.
fn with_adjustment() -> Sequence {
    let mut sequence = Sequence::default_hd();
    let mut clip =
        AdjustmentClip::with_duration(TimelineTime::from_seconds(2), TimelineTime::from_seconds(3))
            .unwrap();
    clip.look = warm();
    let mut lane = AdjustmentTrack::new("Adj 1");
    lane.insert(clip).unwrap();
    sequence.adjustment_tracks.push(lane);
    sequence
}

fn warm() -> AdjustmentLook {
    AdjustmentLook {
        color: ColorAdjust {
            temperature: 0.5,
            ..ColorAdjust::default()
        },
        ..AdjustmentLook::default()
    }
}

#[test]
fn an_adjustment_applies_while_it_runs_and_not_outside() {
    let sequence = with_adjustment();

    assert_eq!(
        adjustments_at(&sequence, TimelineTime::from_seconds(3)),
        vec![warm()],
        "the grade was not running in the middle of its clip"
    );
    for outside in [1, 5, 9] {
        assert!(
            adjustments_at(&sequence, TimelineTime::from_seconds(outside)).is_empty(),
            "the grade applied at {outside} s, outside its clip"
        );
    }
}

/// A hidden lane grades nothing — the rule every other kind of lane follows.
#[test]
fn a_hidden_lane_grades_nothing() {
    let mut sequence = with_adjustment();
    sequence.adjustment_tracks[0].enabled = false;
    assert!(adjustments_at(&sequence, TimelineTime::from_seconds(3)).is_empty());
}

/// While any adjustment lane is soloed, only those grade.
#[test]
fn soloing_one_lane_silences_the_others() {
    let mut sequence = with_adjustment();
    let mut second = AdjustmentTrack::new("Adj 2");
    let mut cool =
        AdjustmentClip::with_duration(TimelineTime::ZERO, TimelineTime::from_seconds(10)).unwrap();
    cool.look.color.temperature = -0.5;
    second.insert(cool.clone()).unwrap();
    sequence.adjustment_tracks.push(second);

    let at = TimelineTime::from_seconds(3);
    assert_eq!(
        adjustments_at(&sequence, at).len(),
        2,
        "both lanes should grade"
    );

    sequence.adjustment_tracks[1].solo = true;
    assert_eq!(
        adjustments_at(&sequence, at),
        vec![cool.look],
        "a soloed lane should be the only one grading"
    );
}

/// Bottom lane first, because a grade works on what the one beneath it left.
#[test]
fn stacked_lanes_come_bottom_first() {
    let mut sequence = with_adjustment();
    let mut upper = AdjustmentTrack::new("Adj 2");
    let mut cool =
        AdjustmentClip::with_duration(TimelineTime::ZERO, TimelineTime::from_seconds(10)).unwrap();
    cool.look.color.temperature = -0.5;
    upper.insert(cool.clone()).unwrap();
    sequence.adjustment_tracks.push(upper);

    assert_eq!(
        adjustments_at(&sequence, TimelineTime::from_seconds(3)),
        vec![warm(), cool.look]
    );
}

/// An adjustment left at its defaults changes nothing, so it is not handed to
/// the renderer to cost a pass.
#[test]
fn an_adjustment_that_changes_nothing_is_left_out() {
    let mut sequence = with_adjustment();
    let clip = sequence.adjustment_tracks[0].clips()[0].id;
    sequence.adjustment_tracks[0].get_mut(clip).unwrap().look = AdjustmentLook::default();
    assert!(adjustments_at(&sequence, TimelineTime::from_seconds(3)).is_empty());
}

/// §50: a project file can carry anything. What reaches the renderer is in
/// range, whatever the file said.
#[test]
fn what_reaches_the_renderer_is_in_range() {
    let mut sequence = with_adjustment();
    let clip = sequence.adjustment_tracks[0].clips()[0].id;
    sequence.adjustment_tracks[0].get_mut(clip).unwrap().look = AdjustmentLook {
        color: ColorAdjust {
            brightness: f32::NAN,
            temperature: 40.0,
            ..ColorAdjust::default()
        },
        blur: -10.0,
        strength: 7.0,
        vignette: 9.0,
        grain: f32::INFINITY,
    };

    let looks = adjustments_at(&sequence, TimelineTime::from_seconds(3));
    let look = looks
        .first()
        .expect("still a grade: the temperature is off");
    assert!(
        look.color.brightness.is_finite(),
        "a NaN brightness got through"
    );
    assert_eq!(look.color.temperature, 1.0);
    assert_eq!(look.blur, 0.0);
    assert_eq!(look.strength, 1.0);
    assert_eq!(look.vignette, 1.0, "a runaway vignette got through");
    assert_eq!(look.grain, 0.0, "an infinite grain got through");
}

/// Grain is drawn for a frame number: the same all through one frame, one
/// more on the next, whatever instant inside the frame is asked about.
#[test]
fn grain_moves_once_a_frame() {
    let sequence = with_adjustment();
    let frame = sequence.ticks_per_frame();
    let at =
        |ticks: i64| bettercut_playback::grain_seed(&sequence, TimelineTime::from_ticks(ticks));

    assert_eq!(at(0), 0);
    assert_eq!(at(frame - 1), 0, "grain changed inside a frame");
    assert_eq!(at(frame), 1);
    assert_eq!(at(frame * 250 + 3), 250);
    assert_eq!(at(-5), 0, "a position before the start is not a frame");
}

// ---- how far up the stack a grade reaches -----------------------------------

/// Every picture, and none of the titles.
#[test]
fn a_grade_reaches_the_pictures_and_stops_at_the_titles() {
    let sequence = Sequence::default_hd();
    let video = sequence.video_tracks[0].id;
    let text = sequence.text_tracks[0].id;

    assert_eq!(graded_beneath(&sequence, [video, video, text]), 2);
    assert_eq!(graded_beneath(&sequence, [video]), 1);
    assert_eq!(graded_beneath(&sequence, [text]), 0);
    assert_eq!(graded_beneath(&sequence, []), 0);
}

/// Counted from the layers actually drawn. When a picture is dropped — its
/// frame not decoded yet, or its media gone — the count drops with it, and the
/// title that followed does not slide in under the grade.
#[test]
fn a_dropped_picture_does_not_pull_a_title_under_the_grade() {
    let sequence = Sequence::default_hd();
    let video = sequence.video_tracks[0].id;
    let text = sequence.text_tracks[0].id;

    // Two pictures were requested; one was dropped before drawing.
    let drawn = [video, text];
    assert_eq!(
        graded_beneath(&sequence, drawn),
        1,
        "the grade must stop before the title, however many pictures were asked for"
    );
}

/// The count stops at the first title rather than counting pictures wherever
/// they are, so nothing drawn after a title can be graded along with it.
#[test]
fn the_count_stops_at_the_first_title() {
    let sequence = Sequence::default_hd();
    let video = sequence.video_tracks[0].id;
    let text = sequence.text_tracks[0].id;
    assert_eq!(graded_beneath(&sequence, [video, text, video]), 1);
}
