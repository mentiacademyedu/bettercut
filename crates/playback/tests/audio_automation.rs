//! A clip's volume envelope, from the timeline to the mixer (§24, §20a.4).
//!
//! The engine is what turns keyframes into something the audio thread can use:
//! the thread must not walk a data structure or allocate (§54), so the envelope
//! is evaluated at both ends of the block and handed over as the line between
//! them. These are the rules that has to keep.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaId, MediaTime, Rational, TimelineTime};
use bettercut_playback::resolve_audio_tracks;
use bettercut_timeline::{
    AnimatedParameter, AudioClip, AudioTrack, Interpolation, Keyframe, SourceRange,
};

fn key(seconds: i64, value: f32) -> Keyframe {
    Keyframe {
        time: MediaTime::from_seconds(seconds),
        value,
        interpolation: Interpolation::Linear,
    }
}

/// One ten-second clip from timeline zero, playing its file from the start.
fn track_with_clip(build: impl FnOnce(&mut AudioClip)) -> Vec<AudioTrack> {
    let mut clip = AudioClip::new(
        MediaId::new(),
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    build(&mut clip);

    let mut track = AudioTrack::new("A1");
    track.insert(clip).unwrap();
    vec![track]
}

#[test]
fn a_clip_with_no_keyframes_has_no_envelope() {
    let tracks = track_with_clip(|clip| clip.gain = 0.5);
    let audible = resolve_audio_tracks(&tracks, TimelineTime::ZERO, TimelineTime::from_millis(10));

    assert_eq!(audible.len(), 1);
    assert!(audible[0].automation.is_none());
    assert_eq!(audible[0].gain, 0.5, "the static gain was lost");
}

/// §24: the keyframed value replaces the static one rather than scaling it, so
/// the static gain handed to the mixer is 1.0 — otherwise a clip set to 0.5 and
/// then ducked to 0.5 would come out at a quarter.
#[test]
fn an_envelope_replaces_the_static_gain() {
    let tracks = track_with_clip(|clip| {
        clip.gain = 0.5;
        clip.keyframes.set(AnimatedParameter::Gain, key(0, 1.0));
        clip.keyframes.set(AnimatedParameter::Gain, key(10, 0.0));
    });
    let audible = resolve_audio_tracks(&tracks, TimelineTime::ZERO, TimelineTime::from_millis(10));

    assert_eq!(audible[0].gain, 1.0);
    let ramp = audible[0].automation.expect("an envelope");
    assert_eq!(ramp.from, 1.0);
}

/// The whole point of a ramp: the block does not hold one value and step at the
/// boundary, which is the zipper noise that makes automation sound cheap.
#[test]
fn the_envelope_moves_across_the_block() {
    let tracks = track_with_clip(|clip| {
        clip.keyframes.set(AnimatedParameter::Gain, key(0, 1.0));
        clip.keyframes.set(AnimatedParameter::Gain, key(10, 0.0));
    });

    // A one-second block: a tenth of the way down the ramp.
    let audible = resolve_audio_tracks(&tracks, TimelineTime::ZERO, TimelineTime::from_seconds(1));
    let ramp = audible[0].automation.expect("an envelope");

    assert_eq!(ramp.from, 1.0);
    assert!(
        (ramp.to - 0.9).abs() < 1e-5,
        "the block ended at {}",
        ramp.to
    );
    assert_eq!(ramp.frames, 48_000, "a second at 48 kHz");
    assert!(
        (ramp.at(24_000) - 0.95).abs() < 1e-4,
        "the middle of the block"
    );
}

/// A block that runs past the clip's end measures the ramp over the part the
/// clip is actually in, or the envelope would be stretched and arrive late.
#[test]
fn a_ramp_stops_at_the_clips_own_end() {
    let tracks = track_with_clip(|clip| {
        clip.keyframes.set(AnimatedParameter::Gain, key(0, 1.0));
        clip.keyframes.set(AnimatedParameter::Gain, key(10, 0.0));
    });

    // Two seconds of block from nine seconds in: only one second is the clip's.
    let audible = resolve_audio_tracks(
        &tracks,
        TimelineTime::from_seconds(9),
        TimelineTime::from_seconds(2),
    );
    let ramp = audible[0].automation.expect("an envelope");

    assert_eq!(ramp.frames, 48_000);
    assert!((ramp.from - 0.1).abs() < 1e-5);
    assert!(
        ramp.to.abs() < 1e-5,
        "the ramp ran past the clip: {}",
        ramp.to
    );
}

/// §51: the envelope is anchored to source time, so a clip at double speed
/// travels through it twice as fast.
#[test]
fn speed_carries_the_envelope() {
    let tracks = track_with_clip(|clip| {
        clip.speed = Rational::new(2, 1).unwrap();
        clip.keyframes.set(AnimatedParameter::Gain, key(0, 1.0));
        clip.keyframes.set(AnimatedParameter::Gain, key(10, 0.0));
    });

    let audible = resolve_audio_tracks(&tracks, TimelineTime::ZERO, TimelineTime::from_seconds(1));
    let ramp = audible[0].automation.expect("an envelope");
    assert!(
        (ramp.to - 0.8).abs() < 1e-5,
        "a second at 2x covers two seconds of source, so the ramp should be at \
         0.8, not {}",
        ramp.to
    );
}
