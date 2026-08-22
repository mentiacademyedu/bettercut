//! Text overlays, resolved and rasterized (§26).
//!
//! §26.1's rule is that *one* implementation shapes and rasterizes text, and
//! that preview and export use it identically. These cover the half of that
//! which does not need a GPU: that a title becomes a layer at the right
//! instants, that it composites over the video rather than under it, and that
//! the bitmap it produces is the same one every time it is asked for.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{FfmpegProber, MediaProber};
use bettercut_playback::{LayerSource, TextFrames, layer_requests};
use bettercut_project_format::Project;
use bettercut_text::{Rgba, TextStyle};
use bettercut_timeline::{SourceRange, TextClip, VideoClip};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A video clip from 0 to 2 s, with a title over the first second of it.
fn project_with_a_title() -> (Project, bettercut_foundation::ClipId) {
    let mut project = Project::new("Text");
    let asset = FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    let media = project.add_media(asset);
    let sequence = project.active_mut().expect("sequence");

    sequence.video_tracks[0]
        .insert(
            VideoClip::new(
                media,
                TimelineTime::ZERO,
                SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).expect("range"),
            )
            .expect("valid"),
        )
        .expect("empty track");

    let title = TextClip::with_duration("Hello", TimelineTime::ZERO, seconds(1)).expect("valid");
    let id = title.id;
    sequence.text_tracks[0].insert(title).expect("empty track");

    (project, id)
}

#[test]
fn a_title_becomes_a_layer_while_it_is_on_screen() {
    let (project, title) = project_with_a_title();
    let sequence = project.active().expect("sequence");

    let during = layer_requests(&project, sequence, TimelineTime::from_millis(500));
    assert_eq!(during.len(), 2, "expected the clip and the title");
    assert!(
        during.iter().any(|l| l.source == LayerSource::Text(title)),
        "the title is not among the layers"
    );

    let after = layer_requests(&project, sequence, TimelineTime::from_millis(1500));
    assert_eq!(after.len(), 1, "the title outlived its own clip");
}

/// §22 draws later layers over earlier ones, and §26 puts text above the
/// picture. Reversed, every title would be invisible behind the shot.
#[test]
fn a_title_composites_over_the_video() {
    let (project, title) = project_with_a_title();
    let sequence = project.active().expect("sequence");

    let layers = layer_requests(&project, sequence, TimelineTime::from_millis(500));
    assert_eq!(
        layers.last().map(|l| l.source),
        Some(LayerSource::Text(title)),
        "the title was not drawn last"
    );
}

/// A title with nothing typed in it yet is the normal state right after it is
/// added. It must not become a layer, and it must not be an error either.
#[test]
fn a_blank_title_is_not_a_layer() {
    let (mut project, title) = project_with_a_title();
    project
        .active_mut()
        .expect("sequence")
        .text_clip_mut(title)
        .expect("clip")
        .text = "   ".to_string();

    let sequence = project.active().expect("sequence");
    let layers = layer_requests(&project, sequence, TimelineTime::from_millis(500));
    assert_eq!(layers.len(), 1, "a blank title was drawn");
}

#[test]
fn a_hidden_text_track_contributes_nothing() {
    let (mut project, _) = project_with_a_title();
    project.active_mut().expect("sequence").text_tracks[0].enabled = false;

    let sequence = project.active().expect("sequence");
    let layers = layer_requests(&project, sequence, TimelineTime::from_millis(500));
    assert_eq!(layers.len(), 1);
}

/// The clip's own opacity reaches the layer, so a title can be faded like
/// anything else.
#[test]
fn the_titles_opacity_is_carried_through() {
    let (mut project, title) = project_with_a_title();
    project
        .active_mut()
        .expect("sequence")
        .text_clip_mut(title)
        .expect("clip")
        .opacity = 0.25;

    let sequence = project.active().expect("sequence");
    let layers = layer_requests(&project, sequence, TimelineTime::from_millis(500));
    let text = layers
        .iter()
        .find(|l| l.source == LayerSource::Text(title))
        .expect("title");
    assert_eq!(text.look.opacity, 0.25);
}

/// The source time is measured within the title's own span, not from the start
/// of the timeline — which is what §24's keyframes anchor to.
#[test]
fn the_source_time_is_relative_to_the_title() {
    let (mut project, _) = project_with_a_title();
    let sequence = project.active_mut().expect("sequence");
    let moved = TextClip::with_duration("Late", seconds(4), seconds(2)).expect("valid");
    let id = moved.id;
    sequence.text_tracks[0].insert(moved).expect("no overlap");

    let sequence = project.active().expect("sequence");
    let layers = layer_requests(&project, sequence, TimelineTime::from_millis(4500));
    let text = layers
        .iter()
        .find(|l| l.source == LayerSource::Text(id))
        .expect("title");
    assert_eq!(
        text.source_time,
        MediaTime::from_millis(500),
        "read from the timeline rather than from within the clip"
    );
}

/// The picture itself: a title rasterizes to a frame the compositor can upload.
#[test]
fn a_title_rasterizes_to_a_frame() {
    let mut titles = TextFrames::new();
    let clip = TextClip::new("Hello", TimelineTime::ZERO).expect("valid");

    let frame = titles.frame_for(&clip).expect("no frame");
    assert!(frame.width > 0 && frame.height > 0);

    let bettercut_media::FrameStorage::System { data, stride } = &frame.storage else {
        panic!("text produced a frame the compositor cannot upload");
    };
    assert_eq!(*stride, frame.width * 4, "the rows are not tightly packed");
    assert_eq!(data.len(), (frame.width * frame.height * 4) as usize);
    assert!(data.chunks_exact(4).any(|p| p[3] > 0), "nothing was drawn");
}

/// §21a: the bitmap is already in the working space, so the upload boundary has
/// nothing to convert. Tagged as limited-range BT.709 it would come out washed.
#[test]
fn a_title_is_tagged_as_already_converted() {
    let mut titles = TextFrames::new();
    let clip = TextClip::new("Hello", TimelineTime::ZERO).expect("valid");
    let frame = titles.frame_for(&clip).expect("no frame");

    assert_eq!(frame.color, bettercut_media::ColorMetadata::srgb());
    assert!(!frame.color.range.needs_expansion());
}

/// The same clip asked for twice is shaped once. A title sits on screen for
/// dozens of frames, and re-shaping it for each of them would be the most
/// expensive thing in the preview.
#[test]
fn the_same_title_is_only_rasterized_once() {
    let mut titles = TextFrames::new();
    let clip = TextClip::new("Hello", TimelineTime::ZERO).expect("valid");

    let first = titles.frame_for(&clip).expect("no frame");
    let second = titles.frame_for(&clip).expect("no frame");

    assert_eq!(titles.cached(), 1, "the same title was shaped twice");
    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "the second ask produced a different allocation"
    );
}

/// And a change to the text or the style produces a different picture, or an
/// edit would appear to do nothing.
#[test]
fn changing_the_text_or_the_style_reshapes() {
    let mut titles = TextFrames::new();
    let mut clip = TextClip::new("Hello", TimelineTime::ZERO).expect("valid");
    let original = titles.frame_for(&clip).expect("no frame");

    clip.text = "Goodbye".to_string();
    let retyped = titles.frame_for(&clip).expect("no frame");
    assert!(
        !std::sync::Arc::ptr_eq(&original, &retyped),
        "changing the text reused the old picture"
    );

    clip.style.color = Rgba::opaque(255, 0, 0);
    let recoloured = titles.frame_for(&clip).expect("no frame");
    assert!(
        !std::sync::Arc::ptr_eq(&retyped, &recoloured),
        "changing the colour reused the old picture"
    );
}

/// §17: the cache is bounded. A long project full of titles must not hold every
/// bitmap it has ever drawn.
#[test]
fn the_cache_is_bounded() {
    let mut titles = TextFrames::new();
    for n in 0..40 {
        let clip = TextClip::new(format!("Title {n}"), TimelineTime::ZERO).expect("valid");
        titles.frame_for(&clip);
    }
    assert!(
        titles.cached() <= 16,
        "the cache grew to {} entries",
        titles.cached()
    );
}

/// §46: two independently built rasterizers — which is exactly what preview and
/// export are — must produce identical bytes from identical parameters.
#[test]
fn preview_and_export_rasterize_the_same_bytes() {
    let clip = TextClip {
        style: TextStyle {
            shadow: Some(bettercut_text::Shadow::default()),
            background: Some(bettercut_text::Background::default()),
            ..TextStyle::default()
        },
        ..TextClip::new("Same both ways", TimelineTime::ZERO).expect("valid")
    };

    let preview = TextFrames::new().frame_for(&clip).expect("no frame");
    let export = TextFrames::new().frame_for(&clip).expect("no frame");

    assert_eq!(
        (preview.width, preview.height),
        (export.width, export.height)
    );
    let (
        bettercut_media::FrameStorage::System { data: a, .. },
        bettercut_media::FrameStorage::System { data: b, .. },
    ) = (&preview.storage, &export.storage)
    else {
        panic!("unexpected storage");
    };
    assert_eq!(a, b, "preview and export drew different pictures");
}

/// The preview's own path, not just the helper it calls: a title resolved
/// through the engine comes back with the correction already applied.
///
/// Without it every title is stretched to fill the frame, and a longer sentence
/// comes out *smaller* — the opposite of a size control.
#[test]
fn the_preview_path_sizes_a_title_naturally() {
    use bettercut_playback::PlaybackEngine;

    let mut project = Project::new("Text only");
    let title = TextClip::with_duration("Hello", TimelineTime::ZERO, seconds(2)).expect("valid");
    let id = title.id;
    let sequence = project.active_mut().expect("sequence");
    sequence.text_tracks[0].insert(title).expect("empty track");

    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(8 * 1024 * 1024, 1);
    let layers = engine.resolve_video(&project, sequence, TimelineTime::from_millis(500));

    assert_eq!(layers.len(), 1, "the title did not resolve to a layer");
    assert_eq!(layers[0].clip, id);

    let expected = bettercut_timeline::natural_size_transform(
        bettercut_timeline::Transform::default(),
        layers[0].frame.width,
        layers[0].frame.height,
        sequence.resolution.width,
        sequence.resolution.height,
    );
    assert!(
        (layers[0].transform.scale.x - expected.scale.x).abs() < 1e-5,
        "resolved at scale {} rather than its natural {}",
        layers[0].transform.scale.x,
        expected.scale.x
    );
    assert!(
        layers[0].transform.scale.x < 1.0,
        "a title smaller than the frame did not shrink at all — it is being \
         fitted to the canvas"
    );
}

/// A media layer must keep the fitting it has always had. The correction
/// applies to generated pictures only, and getting that backwards would
/// letterbox every clip.
#[test]
fn a_media_layer_is_left_fitted() {
    let (project, _) = project_with_a_title();
    let sequence = project.active().expect("sequence");
    let layers = layer_requests(&project, sequence, TimelineTime::from_millis(500));
    let media = layers
        .iter()
        .find(|l| matches!(l.source, LayerSource::Media(_)))
        .expect("no media layer");

    let frame = bettercut_media::VideoFrame {
        timestamp: MediaTime::ZERO,
        width: 640,
        height: 360,
        color: bettercut_media::ColorMetadata::srgb(),
        storage: bettercut_media::FrameStorage::System {
            data: vec![0; 640 * 360 * 4],
            stride: 640 * 4,
        },
    };
    let transform = bettercut_playback::layer_transform(media, &frame, sequence.resolution);
    assert_eq!(
        transform, media.look.transform,
        "a media layer was rescaled"
    );
}
