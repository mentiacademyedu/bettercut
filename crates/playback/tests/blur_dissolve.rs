//! The blur dissolve (§25): both shots go soft, change over, come back sharp.
//!
//! The last of §25's seven, and the one that needed no new compositing at all —
//! blur is already an amount per layer, so the transition is a crossfade with
//! the sharpness taken out of both sides. What is worth pinning down is that it
//! really is *both* sides, and that the softness peaks where the change-over
//! happens rather than somewhere either side of it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::{MAX_TRANSITION_BLUR, layer_requests, transition_blur};
use bettercut_project_format::Project;
use bettercut_timeline::{Resolution, SourceRange, Transition, TransitionKind, VideoClip};

/// Two clips meeting at 4 s with a two-second blur dissolve, each trimmed away
/// from the ends of the file so both have handles to spare (§25).
fn project_with_dissolve() -> Project {
    let mut project = Project::new("Dissolve");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    );
    asset.width = 1920;
    asset.height = 1080;
    let media = project.add_media(asset);

    let sequence = project.active_mut().expect("sequence");
    sequence.resolution = Resolution::new(1920, 1080);
    let source =
        SourceRange::new(MediaTime::from_seconds(10), MediaTime::from_seconds(14)).unwrap();

    let mut first = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    first.transition_out = Some(Transition::new(
        TransitionKind::Blur,
        TimelineTime::from_seconds(2),
    ));
    sequence.video_tracks[0].insert(first).expect("empty track");

    let second = VideoClip::new(media, TimelineTime::from_seconds(4), source).unwrap();
    sequence.video_tracks[0].insert(second).expect("room");
    project
}

fn layers_at(project: &Project, ms: i64) -> Vec<bettercut_playback::LayerRequest> {
    let sequence = project.active().expect("sequence");
    layer_requests(project, sequence, TimelineTime::from_ticks(ms * 960))
}

/// Sharp at either end, softest exactly at the cut.
#[test]
fn the_softness_peaks_at_the_change_over() {
    assert_eq!(transition_blur(0.0), 0.0);
    assert_eq!(transition_blur(1.0), 0.0);
    assert_eq!(transition_blur(0.5), MAX_TRANSITION_BLUR);
    assert!(transition_blur(0.25) < transition_blur(0.5));
    assert!(transition_blur(0.75) < transition_blur(0.5));
}

/// Short of the maximum on purpose: the point is to lose the detail that makes
/// a cut visible, not to turn the frame to fog.
///
/// Both sides are constants, so this is settled when the crate is compiled
/// rather than when the test runs — which is the right time for it: a value
/// out of range should not get as far as a test run.
#[test]
fn a_dissolve_does_not_blur_the_frame_away_entirely() {
    const { assert!(MAX_TRANSITION_BLUR < bettercut_timeline::MAX_BLUR * 0.75) };
    const { assert!(MAX_TRANSITION_BLUR > 0.0) };
}

/// Both shots, not one. Softening only the outgoing clip would read as the
/// picture going wrong rather than as a change-over.
#[test]
fn both_shots_are_softened_together() {
    let layers = layers_at(&project_with_dissolve(), 4_000);
    assert_eq!(layers.len(), 2, "expected both shots on screen");
    assert_eq!(
        layers[0].look.blur, layers[1].look.blur,
        "only one side of the dissolve was softened"
    );
    assert_eq!(layers[0].look.blur, MAX_TRANSITION_BLUR);
}

/// And the crossfade underneath is untouched: the incoming shot still comes up
/// over the outgoing one, which is what actually changes the picture.
#[test]
fn the_incoming_shot_still_fades_up_across_the_window() {
    let project = project_with_dissolve();
    let opacity = |ms| layers_at(&project, ms)[1].look.opacity;

    assert!(
        opacity(3_100) < 0.2,
        "the incoming shot started up too high"
    );
    assert!((opacity(4_000) - 0.5).abs() < 0.02, "not half at the cut");
    assert!(opacity(4_900) > 0.8, "the incoming shot never arrived");
}

/// Sharp again once it is over, both before the window and after it.
#[test]
fn the_picture_is_sharp_either_side_of_the_dissolve() {
    let project = project_with_dissolve();
    for ms in [2_000, 5_500] {
        for layer in layers_at(&project, ms) {
            assert_eq!(layer.look.blur, 0.0, "still soft at {ms} ms");
        }
    }
}

/// A clip the user has already softened is not *sharpened* by a dissolve, and
/// the two amounts are not added — that could ask for more blur than the scale
/// has.
#[test]
fn a_clips_own_blur_is_not_undone_by_the_dissolve() {
    let mut project = project_with_dissolve();
    let sequence = project.active_mut().expect("sequence");
    let id = sequence.video_tracks[0].clips()[0].id;
    sequence.video_tracks[0].get_mut(id).expect("clip").blur = 90.0;

    // Near the start of the window, where the dissolve's own softness is small.
    let layers = layers_at(&project, 3_100);
    assert_eq!(
        layers[0].look.blur, 90.0,
        "the dissolve sharpened a clip the user had blurred"
    );
    assert!(layers[0].look.blur <= bettercut_timeline::MAX_BLUR);
}

/// §25: both shots are on screen at once, so both read outside their own
/// ranges — the same requirement a crossfade has.
#[test]
fn a_blur_dissolve_needs_handles() {
    assert!(TransitionKind::Blur.needs_handles());
}
