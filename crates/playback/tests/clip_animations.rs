//! A clip's entrance and exit, on the way to the screen.
//!
//! The movement itself is decided in the timeline crate and tested there. What
//! these check is that it is decided in `layer_requests` — the one place both
//! the preview and the export ask what is on screen (§46). An animation the
//! preview applied and the export did not would be the worst kind of bug in an
//! editor: the exported film is not the one that was cut.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::layer_requests;
use bettercut_project_format::Project;
use bettercut_timeline::{
    ClipMotion, Motion, MotionKind, Resolution, SourceRange, Transform, VideoClip,
};

/// One 4-second clip filling a 1920×1080 frame, with `motion` on it.
fn project_with(motion: ClipMotion, transform: Transform, opacity: f32) -> Project {
    let mut project = Project::new("Animated");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/shot.mp4",
        MediaTime::from_seconds(10),
    );
    asset.width = 1920;
    asset.height = 1080;
    let media = project.add_media(asset);

    let sequence = project.active_mut().expect("sequence");
    sequence.resolution = Resolution::new(1920, 1080);
    let mut clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    clip.motion = motion;
    clip.transform = transform;
    clip.opacity = opacity;
    sequence.video_tracks[0].insert(clip).expect("empty track");
    project
}

fn look_at(project: &Project, ms: i64) -> bettercut_timeline::ClipLook {
    let sequence = project.active().expect("sequence");
    let layers = layer_requests(project, sequence, TimelineTime::from_ticks(ms * 960));
    assert_eq!(layers.len(), 1, "expected exactly the one clip");
    layers[0].look
}

fn fade_in() -> ClipMotion {
    ClipMotion {
        intro: Some(Motion::new(MotionKind::Fade, TimelineTime::from_seconds(1))),
        outro: None,
    }
}

#[test]
fn an_entrance_reaches_the_layer_both_the_preview_and_the_export_read() {
    let project = project_with(fade_in(), Transform::default(), 1.0);

    assert_eq!(
        look_at(&project, 0).opacity,
        0.0,
        "visible at its first frame"
    );
    let half = look_at(&project, 500).opacity;
    assert!(
        (half - 0.5).abs() < 1e-6,
        "halfway through a one-second fade the clip was at {half}"
    );
    assert_eq!(look_at(&project, 2000).opacity, 1.0, "never arrived");
}

/// A shot placed in a corner slides in to that corner. The animation is applied
/// on top of the clip's own transform, not instead of it.
#[test]
fn an_animation_carries_the_clips_own_placement_with_it() {
    let placed = Transform {
        position: bettercut_timeline::Vec2::new(0.25, 0.0),
        scale: bettercut_timeline::Vec2::new(0.5, 0.5),
        ..Transform::default()
    };
    let slide = ClipMotion {
        intro: Some(Motion::new(
            MotionKind::SlideRight,
            TimelineTime::from_seconds(1),
        )),
        outro: None,
    };
    let project = project_with(slide, placed, 1.0);

    let settled = look_at(&project, 2000).transform;
    assert_eq!(settled, placed, "the clip did not settle where it was put");

    let entering = look_at(&project, 0).transform;
    assert!(
        entering.position.x < placed.position.x,
        "the entrance did not start off to the left: {}",
        entering.position.x
    );
    assert_eq!(
        entering.scale, placed.scale,
        "the entrance rescaled the shot"
    );
}

/// A clip with no animation is untouched — the ordinary case, and the one that
/// would be most obviously wrong if the settled value were not exactly the
/// clip's own.
#[test]
fn a_clip_without_an_animation_is_left_alone() {
    let project = project_with(ClipMotion::default(), Transform::default(), 1.0);
    let look = look_at(&project, 0);
    assert_eq!(look.opacity, 1.0);
    assert_eq!(look.transform, Transform::default());
}

/// Clip animations and §24's keyframes are two movements over the same clip,
/// and the animation runs on top of whatever the keyframes decided — so a clip
/// keyed to 50% fades in *to* 50%, not to full.
#[test]
fn an_entrance_multiplies_an_opacity_the_keyframes_chose() {
    let project = project_with(fade_in(), Transform::default(), 0.5);

    assert_eq!(
        look_at(&project, 2000).opacity,
        0.5,
        "settled at the wrong level"
    );
    let half = look_at(&project, 500).opacity;
    assert!(
        (half - 0.25).abs() < 1e-6,
        "halfway through the fade a half-opacity clip was at {half}"
    );
}
