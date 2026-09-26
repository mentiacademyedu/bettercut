//! The lens flare: soft ovals screened over the shot, in front of it, with
//! the ghosts on the line from the source through the centre of the frame.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::{layer_requests, lens_flare_at};
use bettercut_project_format::Project;
use bettercut_timeline::{BlendMode, SourceRange, VideoClip};

fn project_with(flare: f32) -> Project {
    let mut project = Project::new("Flare");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/sun.mp4",
        MediaTime::from_seconds(10),
    );
    asset.width = 1920;
    asset.height = 1080;
    let media = project.add_media(asset);
    let sequence = project.active_mut().expect("sequence");
    let mut clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    clip.lens_flare = flare;
    sequence.video_tracks[0].insert(clip).expect("empty track");
    project
}

fn layers(project: &Project) -> Vec<bettercut_playback::LayerRequest> {
    let sequence = project.active().expect("sequence");
    layer_requests(project, sequence, TimelineTime::from_seconds(2))
}

#[test]
fn no_flare_adds_no_layers() {
    assert_eq!(layers(&project_with(0.0)).len(), 1);
    assert!(lens_flare_at(0.0, 1.0).is_empty());
}

#[test]
fn a_flare_is_screened_over_the_shot() {
    let layers = layers(&project_with(60.0));
    assert_eq!(layers.len(), 7, "the shot and six flare pieces");
    assert_eq!(
        layers[0].look.blend,
        BlendMode::Normal,
        "the shot is not first"
    );
    for piece in &layers[1..] {
        assert_eq!(piece.look.blend, BlendMode::Screen, "a piece would darken");
        assert!(piece.look.opacity > 0.0 && piece.look.opacity <= 1.0);
        assert!(piece.look.mask.is_some(), "a piece fills the whole frame");
    }
}

#[test]
fn the_ghosts_lie_on_the_line_through_the_centre() {
    let pieces = lens_flare_at(80.0, 3.7);
    let source = pieces[0].centre;
    for ghost in &pieces[3..] {
        // Cross product of (ghost - source) and (centre - source) is zero on
        // the line.
        let a = [ghost.centre[0] - source[0], ghost.centre[1] - source[1]];
        let b = [0.5 - source[0], 0.5 - source[1]];
        assert!(
            (a[0] * b[1] - a[1] * b[0]).abs() < 1e-5,
            "a ghost is off the line"
        );
    }
    // And the flare moves as the clip plays.
    assert_ne!(lens_flare_at(80.0, 0.0)[0].centre, source);
}
