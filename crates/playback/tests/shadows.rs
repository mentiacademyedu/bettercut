//! Drop shadows in the frame plan (`engine::add_shadows`): a layer of their
//! own beneath the picture, offset, carrying none of the picture's grade.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{FrameRate, MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_project_format::Project;
use bettercut_timeline::{Shadow, SourceRange, VideoClip};

fn shadowed(shadow: Shadow) -> Project {
    let mut project = Project::new("Shadow");
    let media = project.add_media(
        MediaAsset::new(
            MediaKind::Video,
            "C:/media/pip.mp4",
            MediaTime::from_seconds(10),
        )
        .with_video(1920, 1080, FrameRate::new(30, 1).unwrap()),
    );
    let mut clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(5)).unwrap(),
    )
    .unwrap();
    clip.shadow = shadow;
    clip.opacity = 0.8;
    clip.blur = 12.0;
    project.active_mut().unwrap().video_tracks[0]
        .insert(clip)
        .unwrap();
    project
}

#[test]
fn the_shadow_is_drawn_first_offset_and_ungraded() {
    let shadow = Shadow {
        opacity: 0.5,
        distance: 0.05,
        angle_degrees: 90.0,
        ..Shadow::NONE
    };
    let project = shadowed(shadow);
    let sequence = project.active().unwrap();
    let requests =
        bettercut_playback::layer_requests(&project, sequence, TimelineTime::from_seconds(1));
    assert_eq!(requests.len(), 2, "no shadow layer");

    let (under, picture) = (&requests[0], &requests[1]);
    assert!(under.look.shadow.is_visible());
    assert!(
        !picture.look.shadow.is_visible(),
        "the picture is a shadow too"
    );
    assert_eq!(under.clip, picture.clip);
    // Straight down, by the distance in frame heights.
    let (a, b) = (
        under.look.transform.position,
        picture.look.transform.position,
    );
    assert!((a.x - b.x).abs() < 1e-5 && (a.y - b.y - 0.05).abs() < 1e-5);
    assert!((under.look.opacity - 0.4).abs() < 1e-5);
    assert_eq!(under.look.blur, 0.0, "the shadow was blurred like the shot");
    assert_eq!(picture.look.blur, 12.0);
}

#[test]
fn no_shadow_adds_no_layer() {
    let project = shadowed(Shadow::NONE);
    let sequence = project.active().unwrap();
    let requests =
        bettercut_playback::layer_requests(&project, sequence, TimelineTime::from_seconds(1));
    assert_eq!(requests.len(), 1);
}

/// Sideways, the distance is measured in frame heights, so the same shadow
/// falls the same number of pixels whichever way it points.
#[test]
fn a_sideways_shadow_is_as_far_in_pixels() {
    let shadow = Shadow {
        distance: 0.1,
        angle_degrees: 0.0,
        ..Shadow::NONE
    };
    let (dx, dy) = shadow.offset(1920, 1080);
    assert!((dx * 1920.0 - 0.1 * 1080.0).abs() < 0.01, "{dx}");
    assert!(dy.abs() < 1e-6);
}
