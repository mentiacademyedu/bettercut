//! The flash transition (§25): white over the cut rather than black under it.
//!
//! The one transition that needed something the engine did not have — a layer
//! with no source file behind it. Everything else composites pictures that were
//! decoded from somewhere; this one generates its own.
//!
//! Both halves are worth guarding. The curve, because a flash that is not full
//! white exactly at the cut is a smear rather than a flash; and the geometry,
//! because the generated frame is one pixel and the compositor fits layers to
//! the frame by aspect — a square of white in the middle of a wide frame is the
//! obvious way for this to go wrong.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::{LayerSource, flash_alpha, layer_requests, layer_transform, solid_frame};
use bettercut_project_format::Project;
use bettercut_timeline::{Resolution, SourceRange, Transition, TransitionKind, VideoClip};

/// Two four-second clips meeting at 4 s, with a two-second flash over the cut.
fn project_with_flash(output: (u32, u32)) -> Project {
    let mut project = Project::new("Flash");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    );
    asset.width = 1920;
    asset.height = 1080;
    let media = project.add_media(asset);

    let sequence = project.active_mut().expect("sequence");
    sequence.resolution = Resolution::new(output.0, output.1);
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();

    let mut first = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    first.transition_out = Some(Transition::new(
        TransitionKind::Flash,
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

fn white_of(layers: &[bettercut_playback::LayerRequest]) -> Option<f32> {
    layers
        .iter()
        .find(|layer| matches!(layer.source, LayerSource::Solid { .. }))
        .map(|layer| layer.look.opacity)
}

/// Nothing at either end, full white at the cut.
#[test]
fn the_white_peaks_exactly_at_the_cut() {
    assert_eq!(flash_alpha(0.0), 0.0);
    assert_eq!(flash_alpha(0.5), 1.0, "the cut was not fully white");
    assert_eq!(flash_alpha(1.0), 0.0);
    assert!((flash_alpha(0.25) - 0.5).abs() < 1e-6);
    assert!((flash_alpha(0.75) - 0.5).abs() < 1e-6);
}

/// The shot stays at full opacity underneath. Fading it out as well would dip
/// through black on the way to white, which is the opposite of the effect.
#[test]
fn the_shot_underneath_is_never_faded() {
    let project = project_with_flash((1920, 1080));

    // The window runs 3 s to 5 s: a second either side of the cut at 4 s.
    for ms in [3_000, 3_500, 4_000, 4_500, 4_999] {
        let layers = layers_at(&project, ms);
        let shot = layers
            .iter()
            .find(|layer| matches!(layer.source, LayerSource::Media(_)))
            .unwrap_or_else(|| panic!("no picture at {ms} ms"));
        assert_eq!(shot.look.opacity, 1.0, "the shot was faded at {ms} ms");
    }
}

#[test]
fn the_white_rises_and_falls_across_the_window() {
    let project = project_with_flash((1920, 1080));

    assert_eq!(white_of(&layers_at(&project, 2_999)), None, "early white");
    assert_eq!(white_of(&layers_at(&project, 3_000)), Some(0.0));
    let quarter = white_of(&layers_at(&project, 3_500)).expect("white");
    assert!(
        (quarter - 0.5).abs() < 1e-6,
        "at a quarter it was {quarter}"
    );
    assert_eq!(white_of(&layers_at(&project, 4_000)), Some(1.0));
    assert_eq!(white_of(&layers_at(&project, 5_001)), None, "late white");
}

/// Over the top, not underneath — §22 draws later layers over earlier ones, and
/// white behind the picture would do nothing at all.
#[test]
fn the_white_is_drawn_over_the_shot() {
    let layers = layers_at(&project_with_flash((1920, 1080)), 4_000);
    let solid = layers
        .iter()
        .position(|layer| matches!(layer.source, LayerSource::Solid { .. }))
        .expect("no white");
    let shot = layers
        .iter()
        .position(|layer| matches!(layer.source, LayerSource::Media(_)))
        .expect("no shot");
    assert!(solid > shot, "the white was drawn behind the picture");
}

/// The generated frame is one pixel and the uniform fits layers to the frame by
/// aspect, so without correction a wide frame would get a white *square* in the
/// middle of it. The transform has to undo that fit exactly, for any shape.
#[test]
fn the_white_covers_the_whole_frame_whatever_its_shape() {
    for output in [(1920, 1080), (1080, 1920), (1000, 1000)] {
        let project = project_with_flash(output);
        let layers = layers_at(&project, 4_000);
        let white = layers
            .iter()
            .find(|layer| matches!(layer.source, LayerSource::Solid { .. }))
            .expect("no white");

        let frame = solid_frame([255, 255, 255]);
        let transform = layer_transform(white, &frame, Resolution::new(output.0, output.1));

        // What the uniform does: fit by aspect, then apply this scale. The two
        // together have to come out at exactly 1, or an edge shows.
        let (fit_x, fit_y) = bettercut_timeline::fit_scale(
            frame.width as f32 / frame.height as f32,
            output.0 as f32 / output.1 as f32,
        );
        let covered = (fit_x * transform.scale.x, fit_y * transform.scale.y);
        assert!(
            (covered.0 - 1.0).abs() < 1e-5 && (covered.1 - 1.0).abs() < 1e-5,
            "white covers {covered:?} of a {output:?} frame, leaving an edge"
        );
    }
}

/// It is white, and opaque. A generated layer with a stray alpha would be a
/// flash nobody can see through to debug.
#[test]
fn the_generated_frame_is_the_colour_it_was_asked_for() {
    let frame = solid_frame([255, 255, 255]);
    let bettercut_media::FrameStorage::System { data, stride } = &frame.storage else {
        panic!("a generated frame should not claim to be on the GPU");
    };
    assert_eq!(data.as_slice(), &[255, 255, 255, 255]);
    assert_eq!(*stride as usize * frame.height as usize, data.len());
}

/// §25: a flash reads nothing outside either clip's own range, so it can sit
/// against the very start or end of a file — the same as a fade through black.
#[test]
fn a_flash_needs_no_handles() {
    assert!(!TransitionKind::Flash.needs_handles());
}
