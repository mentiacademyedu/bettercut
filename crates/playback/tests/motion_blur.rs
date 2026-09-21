//! Motion blur, the cheap way (§45's "Motion blur → Expensive").
//!
//! Done properly it is a velocity buffer and a directional pass over the whole
//! frame. This is what the compositor can already do without either: the same
//! picture drawn at the places it passed through, each fainter than the last —
//! which is what a shutter records anyway.
//!
//! The two things worth holding to: a shot that is not moving must pay nothing,
//! and a smeared shot must not be brighter than an unsmeared one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::{SMEAR_SAMPLES, layer_requests, smear};
use bettercut_project_format::Project;
use bettercut_timeline::{
    ClipMotion, Motion, MotionKind, Resolution, SourceRange, Transform, Vec2, VideoClip,
};

fn moved(x: f32) -> Transform {
    Transform {
        position: Vec2::new(x, 0.0),
        ..Transform::default()
    }
}

/// A still shot pays nothing: one copy, at full strength.
#[test]
fn a_shot_that_has_not_moved_is_drawn_once() {
    let at_rest = smear(moved(0.2), moved(0.2), 1.0);
    assert_eq!(at_rest.len(), 1, "a still shot was smeared");
    assert_eq!(at_rest[0], (moved(0.2), 1.0));
}

/// And a moving one is drawn along the way it came.
#[test]
fn a_moving_shot_is_drawn_along_its_path() {
    let trail = smear(moved(0.4), moved(0.0), 1.0);
    assert_eq!(trail.len(), SMEAR_SAMPLES);

    // Ordered from where it was to where it is, and the last copy is at the
    // true position — the trail goes behind the picture, not in front of it.
    let xs: Vec<f32> = trail.iter().map(|(at, _)| at.position.x).collect();
    assert_eq!(xs.first().copied(), Some(0.0));
    assert_eq!(xs.last().copied(), Some(0.4));
    assert!(
        xs.windows(2).all(|pair| pair[0] < pair[1]),
        "the copies are not in order along the path: {xs:?}"
    );
}

/// The whole stack carries the opacity the layer would have had alone.
/// Otherwise turning motion blur on looks like turning the exposure up.
#[test]
fn a_smear_is_no_brighter_than_the_shot_it_replaces() {
    for opacity in [1.0, 0.5, 0.25] {
        let total: f32 = smear(moved(0.3), moved(0.0), opacity)
            .iter()
            .map(|(_, share)| share)
            .sum();
        assert!(
            (total - opacity).abs() < 1e-6,
            "a smear at {opacity} adds up to {total}"
        );
    }
}

/// Turning, growing and moving all count as movement — a shot that spins on
/// the spot is moving as much as one that crosses the frame.
#[test]
fn a_turn_or_a_zoom_smears_as_much_as_a_move() {
    let turned = Transform {
        rotation_degrees: 10.0,
        ..Transform::default()
    };
    assert_eq!(
        smear(turned, Transform::default(), 1.0).len(),
        SMEAR_SAMPLES
    );

    let grown = Transform {
        scale: Vec2::new(1.2, 1.2),
        ..Transform::default()
    };
    assert_eq!(smear(grown, Transform::default(), 1.0).len(), SMEAR_SAMPLES);
}

/// One 4-second clip sliding in, with motion blur on or off.
fn project_with(blur: bool) -> Project {
    let mut project = Project::new("Smear");
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
    let mut clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    clip.motion_blur = blur;
    clip.motion = ClipMotion {
        intro: Some(Motion::new(
            MotionKind::SlideRight,
            TimelineTime::from_seconds(1),
        )),
        outro: None,
    };
    sequence.video_tracks[0].insert(clip).expect("empty track");
    project
}

fn layers_at(project: &Project, ms: i64) -> Vec<bettercut_playback::LayerRequest> {
    let sequence = project.active().expect("sequence");
    layer_requests(project, sequence, TimelineTime::from_ticks(ms * 960))
}

/// The wire: a clip with the setting on, mid-entrance, is drawn several times.
#[test]
fn a_clip_arriving_with_motion_blur_is_drawn_more_than_once() {
    assert_eq!(
        layers_at(&project_with(false), 500).len(),
        1,
        "a clip without the setting was smeared"
    );
    assert_eq!(
        layers_at(&project_with(true), 500).len(),
        SMEAR_SAMPLES,
        "the entrance did not smear"
    );
}

/// And once it has arrived and stopped, the cost goes away on its own.
#[test]
fn the_smear_stops_when_the_shot_does() {
    assert_eq!(
        layers_at(&project_with(true), 3_000).len(),
        1,
        "a settled shot was still being smeared"
    );
}

/// Motion blur makes one clip several layers, and three places in the engine
/// take the layer they just pushed and rewrite it — the backdrop, the blur
/// dissolve and the moving transitions. Each of those wants *one* layer with a
/// particular shape, not a trail.
mod alongside_other_effects {
    use super::*;
    use bettercut_timeline::{Backdrop, Transition, TransitionKind};

    /// A backdrop is a full-frame fill behind the shot. A smear of backdrops
    /// is neither a fill nor a smear — and the copies the backdrop path does
    /// not rewrite are the shot itself, drawn behind where it should be.
    #[test]
    fn a_backdrop_is_still_one_layer_behind_a_smeared_shot() {
        let mut project = project_with(true);
        {
            let sequence = project.active_mut().expect("sequence");
            sequence.resolution = Resolution::new(1080, 1920);
            let id = sequence.video_tracks[0].clips()[0].id;
            sequence.video_tracks[0].get_mut(id).expect("clip").backdrop = Backdrop::Blur;
        }

        // One backdrop plus the shot's own trail, and nothing else. Counting
        // *blurred* layers would say 1 even when the path leaves strays
        // behind, because only the one it rewrote carries the blur.
        let layers = layers_at(&project, 500);
        assert_eq!(
            layers.len(),
            1 + SMEAR_SAMPLES,
            "expected a backdrop and a {SMEAR_SAMPLES}-copy trail, got {} layers",
            layers.len()
        );
        let backdrops = layers.iter().filter(|layer| layer.look.blur > 0.0).count();
        assert_eq!(backdrops, 1, "the backdrop was smeared into several");
    }

    /// A moving transition places both shots by hand. Copies it did not place
    /// sit at the clip's untransformed position, so the outgoing shot appears
    /// to slide and stand still at the same time.
    #[test]
    fn a_moving_transition_places_every_layer_it_draws() {
        let mut project = project_with(true);
        {
            let sequence = project.active_mut().expect("sequence");
            let id = sequence.video_tracks[0].clips()[0].id;
            let media = sequence.video_tracks[0].get(id).expect("clip").media_id;
            sequence.video_tracks[0]
                .get_mut(id)
                .expect("clip")
                .transition_out = Some(Transition::new(
                TransitionKind::Push,
                TimelineTime::from_seconds(1),
            ));

            let mut second = VideoClip::new(
                media,
                TimelineTime::from_seconds(4),
                SourceRange::new(MediaTime::from_seconds(10), MediaTime::from_seconds(14)).unwrap(),
            )
            .unwrap();
            second.motion_blur = true;
            second.motion = ClipMotion {
                intro: Some(Motion::new(
                    MotionKind::SlideRight,
                    TimelineTime::from_seconds(1),
                )),
                outro: None,
            };
            sequence.video_tracks[0].insert(second).expect("room");
        }

        // Mid-push, and mid-entrance: the second shot is still sliding in, so
        // its look really is moving and the smear really would fire if the
        // transition let it. Exactly two layers, one per shot, each where the
        // transition put it.
        let layers = layers_at(&project, 4_200);
        assert_eq!(
            layers.len(),
            2,
            "a push drew {} layers instead of two",
            layers.len()
        );
    }
}

/// A title is the thing most likely to be moving fast — a spin or a pop is over
/// in a third of a second — so it smears the same way a shot does. The
/// asymmetry was the bug: a spinning clip blurred and a spinning title did not,
/// for no reason a user could see.
mod titles {
    use super::*;
    use bettercut_timeline::{TextAnimation, TextClip};

    fn project_with_title(blur: bool) -> Project {
        let mut project = Project::new("Titles");
        let sequence = project.active_mut().expect("sequence");
        sequence.resolution = Resolution::new(1920, 1080);

        let mut title =
            TextClip::with_duration("Hello", TimelineTime::ZERO, TimelineTime::from_seconds(4))
                .expect("valid title");
        title.motion_blur = blur;
        title.animation = TextAnimation {
            scroll: None,
            intro: Some(Motion::new(MotionKind::Spin, TimelineTime::from_seconds(1))),
            outro: None,
            looping: None,
        };
        sequence.text_tracks[0].insert(title).expect("empty track");
        project
    }

    #[test]
    fn a_spinning_title_smears_like_a_spinning_shot() {
        assert_eq!(
            layers_at(&project_with_title(false), 500).len(),
            1,
            "a title without the setting was smeared"
        );
        assert_eq!(
            layers_at(&project_with_title(true), 500).len(),
            SMEAR_SAMPLES,
            "the title's spin did not smear"
        );
    }

    /// And a title sitting still costs nothing, exactly as a shot does.
    #[test]
    fn a_settled_title_is_drawn_once() {
        assert_eq!(layers_at(&project_with_title(true), 3_000).len(), 1);
    }
}
