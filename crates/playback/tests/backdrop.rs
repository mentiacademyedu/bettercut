//! The blurred backdrop behind a clip that does not fill the frame (§36).
//!
//! Landscape footage in a vertical sequence is the everyday case of reframing,
//! and the choice is black bars or *something*. This is the something every
//! phone editor does: the same picture, blown up to cover and softened.
//!
//! It lives in `layer_requests` beside the transitions, for the same reason —
//! it is a decision about which layers are on screen, and preview and export
//! must not each make it (§46).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::layer_requests;
use bettercut_project_format::Project;
use bettercut_timeline::{
    Backdrop, Resolution, SourceRange, Transition, TransitionKind, VideoClip,
};

/// A sequence `output` wide and tall, with one clip of `source` shape on it.
fn project_with(source: (u32, u32), output: (u32, u32), backdrop: Backdrop) -> Project {
    let mut project = Project::new("Backdrop");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/wide.mp4",
        MediaTime::from_seconds(10),
    );
    asset.width = source.0;
    asset.height = source.1;
    let media = project.add_media(asset);

    let sequence = project.active_mut().expect("sequence");
    sequence.resolution = Resolution::new(output.0, output.1);
    let mut clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    clip.backdrop = backdrop;
    sequence.video_tracks[0].insert(clip).expect("empty track");
    project
}

fn layers(project: &Project) -> Vec<bettercut_playback::LayerRequest> {
    let sequence = project.active().expect("sequence");
    layer_requests(project, sequence, TimelineTime::from_seconds(1))
}

/// 16:9 footage in a 9:16 frame: the backdrop covers, the shot does not move.
#[test]
fn a_backdrop_fills_the_frame_behind_the_shot() {
    let project = project_with((1920, 1080), (1080, 1920), Backdrop::Blur);
    let layers = layers(&project);

    assert_eq!(layers.len(), 2, "expected a backdrop and the shot");

    let backdrop = &layers[0];
    let shot = &layers[1];

    // (16/9) / (9/16) is about 3.16, and the backdrop is drawn a little larger
    // still so the blur does not sample off its own edge.
    assert!(
        backdrop.look.transform.scale.x > 3.1,
        "the backdrop does not cover the frame: {}",
        backdrop.look.transform.scale.x
    );
    assert_eq!(
        backdrop.look.transform.scale.x, backdrop.look.transform.scale.y,
        "the backdrop was stretched rather than scaled"
    );
    assert!(backdrop.look.blur > 50.0, "the backdrop is not blurred");

    assert_eq!(shot.look.transform.scale.x, 1.0, "the shot was scaled");
    assert_eq!(shot.look.blur, 0.0, "the shot was blurred");
}

/// Drawn *first*, because §22 puts later layers over earlier ones and the shot
/// belongs in front of its own backdrop.
#[test]
fn the_backdrop_is_behind_the_shot() {
    let project = project_with((1920, 1080), (1080, 1920), Backdrop::Blur);
    let layers = layers(&project);
    assert!(
        layers[0].look.blur > layers[1].look.blur,
        "the blurred copy is in front of the shot"
    );
}

/// A clip that already covers the frame has nothing to fill: a backdrop there
/// is a second decode and a second draw of something nobody can see.
#[test]
fn a_clip_that_already_fills_the_frame_gets_no_backdrop() {
    let project = project_with((1920, 1080), (1920, 1080), Backdrop::Blur);
    assert_eq!(layers(&project).len(), 1);

    // And a shape near enough to the frame's — 16:9 against 1.77:1 — counts as
    // filling it rather than getting a backdrop nobody would see.
    let project = project_with((1920, 1080), (1280, 720), Backdrop::Blur);
    assert_eq!(layers(&project).len(), 1);
}

#[test]
fn no_backdrop_means_no_extra_layer() {
    let project = project_with((1920, 1080), (1080, 1920), Backdrop::None);
    assert_eq!(layers(&project).len(), 1, "a backdrop appeared uninvited");
}

/// The clip's own framing is not carried onto the backdrop: a shot pushed to
/// one side should still have a full frame behind it.
#[test]
fn the_backdrop_ignores_the_clips_own_transform() {
    let mut project = project_with((1920, 1080), (1080, 1920), Backdrop::Blur);
    let sequence = project.active_mut().expect("sequence");
    let clip = sequence.video_tracks[0].clips()[0].id;
    let clip = sequence.video_tracks[0].get_mut(clip).expect("clip");
    clip.transform.position.x = 0.3;
    clip.opacity = 0.5;

    let layers = layers(&project);
    assert_eq!(
        layers[0].look.transform.position.x, 0.0,
        "the backdrop moved"
    );
    assert_eq!(
        layers[1].look.transform.position.x, 0.3,
        "the shot did not move"
    );

    // Opacity *is* carried, and this assertion used to say the opposite — the
    // backdrop was forced solid so that a half-transparent shot still had a
    // full frame behind it. That reasoning only holds over black. Over another
    // track it meant a clip at any opacity at all blotted out everything
    // beneath it, and a clip faded to nothing still filled the frame.
    assert_eq!(
        layers[0].look.opacity, 0.5,
        "the backdrop ignored the shot's opacity and covered what is beneath"
    );
}

/// The end of that: a clip faded out has to actually go.
#[test]
fn a_clip_faded_to_nothing_leaves_no_backdrop_behind() {
    let mut project = project_with((1920, 1080), (1080, 1920), Backdrop::Blur);
    let sequence = project.active_mut().expect("sequence");
    let clip = sequence.video_tracks[0].clips()[0].id;
    sequence.video_tracks[0]
        .get_mut(clip)
        .expect("clip")
        .opacity = 0.0;

    for layer in layers(&project) {
        assert_eq!(
            layer.look.opacity, 0.0,
            "an invisible clip still had something on screen"
        );
    }
}

/// An animation is the shot *arriving*, and the backdrop arrives with it.
/// A slide that left the backdrop where it was would read as the picture
/// travelling across a blurred frame that had been there all along.
#[test]
fn the_backdrop_travels_with_the_clips_animation() {
    use bettercut_timeline::{ClipMotion, Motion, MotionKind};

    let mut project = project_with((1920, 1080), (1080, 1920), Backdrop::Blur);
    let sequence = project.active_mut().expect("sequence");
    let clip = sequence.video_tracks[0].clips()[0].id;
    let clip = sequence.video_tracks[0].get_mut(clip).expect("clip");
    clip.transform.position.x = 0.3;
    clip.motion = ClipMotion {
        intro: Some(Motion::new(
            MotionKind::SlideRight,
            TimelineTime::from_seconds(2),
        )),
        outro: None,
    };

    // Half a second into a two-second entrance: both still on their way in.
    let sequence = project.active().expect("sequence");
    let entering =
        bettercut_playback::layer_requests(&project, sequence, TimelineTime::from_ticks(480_000));
    let (backdrop, shot) = (&entering[0], &entering[1]);
    assert!(
        backdrop.look.transform.position.x < 0.0,
        "the backdrop stayed put while the shot slid in: {}",
        backdrop.look.transform.position.x
    );
    assert_eq!(
        backdrop.look.transform.position.x,
        shot.look.transform.position.x - 0.3,
        "the two did not travel together"
    );

    // And once the entrance is over — past its two seconds, which is why this
    // does not use the one-second `layers` — the backdrop is a plain full
    // frame again, the clip's own framing still not carried.
    let settled =
        bettercut_playback::layer_requests(&project, sequence, TimelineTime::from_seconds(3));
    assert_eq!(settled[0].look.transform.position.x, 0.0);
    assert_eq!(settled[1].look.transform.position.x, 0.3, "the shot moved");
}

/// Not during a transition: both clips are moving then, and a backdrop would
/// be revealed at the edges as they slide.
#[test]
fn a_transition_has_no_backdrop() {
    let mut project = project_with((1920, 1080), (1080, 1920), Backdrop::Blur);
    let media = project.media[0].id;
    let sequence = project.active_mut().expect("sequence");

    let mut second = VideoClip::new(
        media,
        TimelineTime::from_seconds(10),
        SourceRange::new(MediaTime::from_seconds(10), MediaTime::from_seconds(20)).unwrap(),
    )
    .unwrap();
    second.backdrop = Backdrop::Blur;
    sequence.video_tracks[0].insert(second).expect("no overlap");

    let first = sequence.video_tracks[0].clips()[0].id;
    sequence.video_tracks[0]
        .get_mut(first)
        .expect("clip")
        .transition_out = Some(Transition::new(
        TransitionKind::Slide,
        TimelineTime::from_seconds(1),
    ));

    let sequence = project.active().expect("sequence");
    let during = layer_requests(
        project_ref(&project),
        sequence,
        TimelineTime::from_millis(9_800),
    );
    assert_eq!(during.len(), 2, "a backdrop was drawn under a transition");
    assert!(during.iter().all(|layer| layer.look.blur == 0.0));
}

/// Borrow helper: `layer_requests` takes the project by reference and the
/// sequence separately, and both come from the same value.
fn project_ref(project: &Project) -> &Project {
    project
}

/// A clip whose media has gone (§66) leaves a gap rather than a backdrop of
/// nothing.
#[test]
fn missing_media_draws_no_backdrop() {
    let mut project = project_with((1920, 1080), (1080, 1920), Backdrop::Blur);
    project.media.clear();
    assert!(layers(&project).is_empty());
    let _ = MediaId::new();
}
