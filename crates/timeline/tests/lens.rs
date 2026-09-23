//! The lens as data: older projects say nothing about it and load with none,
//! and a set lens survives the round trip and reaches the look.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_timeline::{SourceRange, VideoClip};

#[test]
fn a_clip_written_before_the_lens_loads_with_none() {
    let clip = VideoClip::new(
        MediaId::new(),
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap(),
    )
    .unwrap();
    let mut json: serde_json::Value = serde_json::to_value(&clip).unwrap();
    json.as_object_mut().unwrap().remove("lens");
    let back: VideoClip = serde_json::from_value(json).unwrap();
    assert_eq!(back.lens, 0.0);
    assert_eq!(back.look_at(MediaTime::ZERO).lens, 0.0);
}

#[test]
fn a_set_lens_survives_the_round_trip_and_reaches_the_look() {
    let mut clip = VideoClip::new(
        MediaId::new(),
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap(),
    )
    .unwrap();
    clip.lens = -0.25;
    let back: VideoClip = serde_json::from_str(&serde_json::to_string(&clip).unwrap()).unwrap();
    assert_eq!(back.lens, -0.25);
    assert_eq!(back.look_at(MediaTime::from_seconds(1)).lens, -0.25);
}
