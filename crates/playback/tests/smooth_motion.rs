//! Smooth slow motion (`engine::smooth_motion_blend`): a slowed clip laid over
//! by its next frame as far as the playhead is between the two.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{FrameRate, MediaTime, Rational, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::engine::smooth_motion_blend;
use bettercut_project_format::Project;
use bettercut_timeline::{SourceRange, VideoClip};

/// A 10 fps file (so a frame is a tenth of a second), a clip of it at half
/// speed with smooth motion on.
fn slowed() -> (Project, VideoClip) {
    let mut project = Project::new("Smooth");
    let asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/run.mp4",
        MediaTime::from_seconds(10),
    )
    .with_video(320, 180, FrameRate::new(10, 1).unwrap());
    let media = project.add_media(asset);
    let mut clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    clip.speed = Rational::new(1, 2).unwrap();
    clip.smooth_motion = true;
    (project, clip)
}

#[test]
fn between_two_frames_the_next_is_laid_over_by_how_far() {
    let (project, clip) = slowed();
    // A quarter of the way from the frame at 0.2 s to the one at 0.3 s.
    let (base, next, phase) =
        smooth_motion_blend(&clip, &project, MediaTime::from_millis(225)).unwrap();
    assert_eq!(base, MediaTime::from_millis(200));
    assert_eq!(next, MediaTime::from_millis(300));
    assert!((phase - 0.25).abs() < 1e-3, "phase {phase}");

    // Exactly on a frame there is nothing to blend.
    assert!(smooth_motion_blend(&clip, &project, MediaTime::from_millis(300)).is_none());
}

#[test]
fn nothing_blends_at_full_speed_off_or_frozen() {
    let (project, mut clip) = slowed();
    let at = MediaTime::from_millis(225);

    clip.smooth_motion = false;
    assert!(smooth_motion_blend(&clip, &project, at).is_none(), "off");

    clip.smooth_motion = true;
    clip.speed = Rational::ONE;
    assert!(
        smooth_motion_blend(&clip, &project, at).is_none(),
        "full speed"
    );

    clip.speed = Rational::new(1, 2).unwrap();
    clip.frozen = true;
    assert!(smooth_motion_blend(&clip, &project, at).is_none(), "a hold");
}

/// Backwards, the frame laid over is the earlier one.
#[test]
fn a_reversed_clip_blends_towards_the_earlier_frame() {
    let (project, mut clip) = slowed();
    clip.reversed = true;
    let (base, next, phase) =
        smooth_motion_blend(&clip, &project, MediaTime::from_millis(225)).unwrap();
    assert_eq!(base, MediaTime::from_millis(300));
    assert_eq!(next, MediaTime::from_millis(200));
    assert!((phase - 0.75).abs() < 1e-3, "phase {phase}");
}

/// The frame plan draws the clip twice, the second layer at the phase's
/// opacity, when it falls between frames.
#[test]
fn the_plan_draws_the_blend_as_a_second_layer() {
    let (mut project, clip) = slowed();
    project.active_mut().unwrap().video_tracks[0]
        .insert(clip)
        .unwrap();
    let sequence = project.active().unwrap();
    // 0.45 s into a half-speed clip reads 0.225 s of source.
    let requests =
        bettercut_playback::layer_requests(&project, sequence, TimelineTime::from_millis(450));
    assert_eq!(requests.len(), 2, "no blend layer");
    assert_eq!(requests[0].source_time, MediaTime::from_millis(200));
    assert_eq!(requests[1].source_time, MediaTime::from_millis(300));
    assert!((requests[1].look.opacity - 0.25).abs() < 1e-3);
}
