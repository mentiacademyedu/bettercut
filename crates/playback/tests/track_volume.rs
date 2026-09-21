//! A whole lane's volume line, from the timeline to the mixer
//! (`bettercut_timeline::track_volume`, §20a.4).
//!
//! The audio thread must not walk a data structure or allocate (§54), so the
//! line is evaluated at both ends of each block and handed over as the line
//! between them — exactly as a clip's envelope is. These are the rules that
//! has to keep, and the one rule that ties the two together: the clip says how
//! loud that piece is, the lane says how loud the lane is under it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_playback::resolve_audio_tracks;
use bettercut_timeline::{AudioClip, AudioTrack, SourceRange, VolumePoint};

fn point(seconds: i64, gain: f32) -> VolumePoint {
    VolumePoint::new(TimelineTime::from_seconds(seconds), gain)
}

/// One ten-second clip from timeline zero, on a lane the caller sets up.
fn lane(build: impl FnOnce(&mut AudioTrack)) -> Vec<AudioTrack> {
    let clip = AudioClip::new(
        MediaId::new(),
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    let mut track = AudioTrack::new("A1");
    track.insert(clip).unwrap();
    build(&mut track);
    vec![track]
}

#[test]
fn a_lane_without_a_line_keeps_its_static_level() {
    let tracks = lane(|track| track.gain = 0.5);
    let audible = resolve_audio_tracks(&tracks, TimelineTime::ZERO, TimelineTime::from_millis(10));

    assert_eq!(audible.len(), 1);
    assert!(audible[0].track_automation.is_none());
    assert_eq!(audible[0].track_gain, 0.5, "the lane's level was lost");
}

/// The same rule §24 gives a clip, one stage up: a line replaces the static
/// level rather than scaling it, or a lane set to 0.5 and then ridden at 0.5
/// would come out at a quarter.
#[test]
fn a_line_replaces_the_static_level() {
    let tracks = lane(|track| {
        track.gain = 0.5;
        track.volume.set([point(0, 1.0), point(10, 1.0)]);
    });
    let audible = resolve_audio_tracks(&tracks, TimelineTime::ZERO, TimelineTime::from_millis(10));

    assert_eq!(audible[0].track_gain, 1.0);
    let ramp = audible[0].track_automation.expect("a ramp");
    assert_eq!(ramp.from, 1.0);
    assert_eq!(ramp.to, 1.0);
}

/// A scene riding down: the ramp handed over spans the block, so the level
/// moves smoothly instead of stepping at every block boundary.
#[test]
fn the_ramp_follows_the_line_across_the_block() {
    let tracks = lane(|track| track.volume.set([point(0, 1.0), point(10, 0.0)]));
    let audible = resolve_audio_tracks(&tracks, TimelineTime::ZERO, TimelineTime::from_seconds(1));

    let ramp = audible[0].track_automation.expect("a ramp");
    assert_eq!(ramp.from, 1.0);
    assert!((ramp.to - 0.9).abs() < 1e-4, "{}", ramp.to);
    assert!(ramp.frames > 0, "a ramp over no frames holds one level");

    // And a block from the middle picks up where that one left off.
    let later = resolve_audio_tracks(
        &tracks,
        TimelineTime::from_seconds(5),
        TimelineTime::from_seconds(1),
    );
    let ramp = later[0].track_automation.expect("a ramp");
    assert!((ramp.from - 0.5).abs() < 1e-4, "{}", ramp.from);
    assert!((ramp.to - 0.4).abs() < 1e-4, "{}", ramp.to);
}

/// A line drawn across the middle of an edit says nothing about either end.
#[test]
fn the_line_is_flat_outside_its_points() {
    let tracks = lane(|track| track.volume.set([point(4, 0.25), point(6, 0.25)]));
    let audible = resolve_audio_tracks(&tracks, TimelineTime::ZERO, TimelineTime::from_millis(10));

    let ramp = audible[0].track_automation.expect("a ramp");
    assert_eq!(ramp.from, 0.25, "the level before the first point");
    assert_eq!(ramp.to, 0.25);
}

/// The two lines multiply: a clip at half under a lane at half is a quarter.
#[test]
fn the_clip_and_the_lane_multiply() {
    let planes = vec![vec![1.0_f32; 4]];
    let mut out = vec![0.0_f32; 8];
    bettercut_audio::mix_into(
        &mut out,
        2,
        &planes,
        0,
        bettercut_audio::MixParams {
            clip_gain: 1.0,
            track_gain: 1.0,
            track_pan: 0.0,
            fades: bettercut_audio::Fades::default(),
            automation: Some(bettercut_audio::GainRamp::steady(0.5)),
            track_automation: Some(bettercut_audio::GainRamp::steady(0.5)),
        },
    );
    // Constant-power panning puts a centred source at 1/sqrt(2) a side.
    let expected = 0.25 / 2.0_f32.sqrt();
    assert!(
        (out[0] - expected).abs() < 1e-5,
        "{} should be {expected}",
        out[0]
    );
    assert!((out[1] - expected).abs() < 1e-5);
}

/// And a lane ridden to nothing is silence, however loud the clip is.
#[test]
fn a_lane_ridden_to_nothing_is_silent() {
    let planes = vec![vec![1.0_f32; 4]];
    let mut out = vec![0.0_f32; 8];
    bettercut_audio::mix_into(
        &mut out,
        2,
        &planes,
        0,
        bettercut_audio::MixParams {
            clip_gain: 4.0,
            track_automation: Some(bettercut_audio::GainRamp::steady(0.0)),
            ..bettercut_audio::MixParams::default()
        },
    );
    assert!(out.iter().all(|sample| sample.abs() < 1e-6), "{out:?}");
}
