//! Colour match end to end (`bettercut_export::colour_match`): two encoded
//! shots of one scene, one dim and cool, one bright and warm; the dim one
//! matched to the other through real decoding and a real render.
//!
//! GPU tests take one lock, as in `still.rs`. Skips itself without a GPU.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_export::{VideoCodec, auto_level, colour_match, render_still};
use bettercut_foundation::{ClipId, FrameRate, MediaTime, TimelineTime};
use bettercut_media::{FfmpegProber, MediaProber, NeverCancelled};
use bettercut_project_format::Project;
use bettercut_timeline::colour_match::{FrameSample, FrameStats};
use bettercut_timeline::{Resolution, SourceRange, VideoClip};

static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn gpu_available() -> bool {
    let instance = bettercut_renderer::wgpu::Instance::new(
        bettercut_renderer::wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
    );
    pollster::block_on(
        instance.request_adapter(&bettercut_renderer::wgpu::RequestAdapterOptions::default()),
    )
    .is_ok()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("bettercut-match-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

const SIZE: Resolution = Resolution {
    width: 320,
    height: 180,
};

/// One second of a still scene — a diagonal ramp through several colours —
/// with each channel scaled by `gain`, as a camera exposed and balanced
/// differently would record it.
fn scene(path: &Path, gain: [f32; 3]) {
    use bettercut_media::{ExportFormat, VideoWriter};

    let (width, height) = (SIZE.width, SIZE.height);
    let mut writer = VideoWriter::create(
        path,
        ExportFormat {
            transparent: false,
            prores: false,
            width,
            height,
            frame_rate: FrameRate::FPS_30,
            codec: VideoCodec::H264,
            bitrate: Some(4_000_000),
            rate_control: bettercut_export::RateControl::Variable,
            channels: 0,
            audio_bitrate: None,
            threads: 2,
        },
    )
    .expect("open the generated source");
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let t = (x + y) as f32 / (width + height) as f32;
            let base = [
                0.15 + 0.7 * t,
                0.2 + 0.6 * t * (1.0 - y as f32 / height as f32),
                0.1 + 0.6 * (1.0 - t),
            ];
            let at = ((y * width + x) * 4) as usize;
            for k in 0..3 {
                rgba[at + k] = (base[k] * gain[k] * 255.0).clamp(0.0, 255.0) as u8;
            }
            rgba[at + 3] = 255;
        }
    }
    for _ in 0..30 {
        writer.push_frame(&rgba).expect("write a frame");
    }
    writer.finish().expect("finish the generated source");
}

/// The dim shot at 0–1 s and the bright one at 1–2 s on V1.
fn project_with(dim: &Path, bright: &Path) -> (Project, ClipId) {
    let mut project = Project::new("Match");
    let mut ids = Vec::new();
    for (start, path) in [(0, dim), (1, bright)] {
        let asset = FfmpegProber.probe(path).expect("probe");
        let duration = asset.duration.min(MediaTime::from_seconds(1));
        let media = project.add_media(asset);
        let clip = VideoClip::new(
            media,
            TimelineTime::from_seconds(start),
            SourceRange::new(MediaTime::ZERO, duration).unwrap(),
        )
        .unwrap();
        ids.push(clip.id);
        let sequence = project.active_mut().expect("sequence");
        sequence.resolution = SIZE;
        sequence.frame_rate = FrameRate::FPS_30;
        sequence.video_tracks[0].insert(clip).unwrap();
    }
    (project, ids[0])
}

fn stats_at(project: &Project, millis: i64) -> FrameStats {
    let (size, rgba) = render_still(
        project,
        project.active().unwrap(),
        TimelineTime::from_millis(millis),
        &NeverCancelled,
    )
    .unwrap();
    FrameSample::from_rgba(&rgba, size.width, size.height)
        .unwrap()
        .stats()
}

#[test]
fn a_dim_cool_shot_is_matched_to_a_bright_warm_one() {
    let _one = serial();
    if !gpu_available() {
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let (dim, bright) = (Scratch::new("dim.mp4"), Scratch::new("bright.mp4"));
    scene(&dim.0, [0.85, 0.88, 0.95]);
    scene(&bright.0, [1.0, 0.95, 0.9]);
    let (mut project, clip) = project_with(&dim.0, &bright.0);
    let sequence = project.active().unwrap().id;

    let reference = stats_at(&project, 1_500);
    let before = stats_at(&project, 500);

    let grade = colour_match(
        &project,
        sequence,
        clip,
        TimelineTime::from_millis(1_500),
        &NeverCancelled,
    )
    .unwrap();
    assert!(
        grade.brightness > 1.0,
        "a dim shot was not brightened: {grade:?}"
    );
    assert!(
        grade.temperature > 0.0,
        "a cool shot was not warmed: {grade:?}"
    );

    project.active_mut().unwrap().video_tracks[0]
        .get_mut(clip)
        .unwrap()
        .color = grade;
    let after = stats_at(&project, 500);
    assert!(
        (after.luma - reference.luma).abs() < 0.25 * (before.luma - reference.luma).abs(),
        "brightness did not come close: before {before:?}, after {after:?}, reference {reference:?}"
    );
    assert!(
        (after.warmth - reference.warmth).abs() < 0.25 * (before.warmth - reference.warmth).abs(),
        "the cast did not come close: before {before:?}, after {after:?}, reference {reference:?}"
    );
}

/// Auto level needs no reference and no GPU: the dim, cool shot comes up
/// and warms towards neutral from its own frame.
#[test]
fn auto_level_brings_a_dim_cool_shot_up_from_its_own_frame() {
    let (dim, bright) = (Scratch::new("dim3.mp4"), Scratch::new("bright3.mp4"));
    scene(&dim.0, [0.85, 0.88, 0.95]);
    scene(&bright.0, [1.0, 0.95, 0.9]);
    let (project, clip) = project_with(&dim.0, &bright.0);
    let sequence = project.active().unwrap().id;
    let grade = auto_level(&project, sequence, clip, &NeverCancelled).unwrap();
    assert!(
        grade.brightness > 1.0,
        "a dim shot was not brightened: {grade:?}"
    );
    // Aimed at neutral, and never at an extreme: the scene is warm by
    // design, so its balance moves but stays within the auto's reach.
    assert!(
        grade.temperature.abs() <= 0.5 && grade.tint.abs() <= 0.5 && grade.contrast <= 1.6,
        "the auto grade went to an extreme: {grade:?}"
    );
    assert!(
        auto_level(&project, sequence, ClipId::new(), &NeverCancelled).is_err(),
        "a clip that is not there was graded"
    );
}

/// A playhead resting on the clip itself, with nothing beneath it, has no
/// reference: the clip is left out of the reference, which is then black.
#[test]
fn nothing_under_the_playhead_is_refused() {
    let _one = serial();
    if !gpu_available() {
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let (dim, bright) = (Scratch::new("dim2.mp4"), Scratch::new("bright2.mp4"));
    scene(&dim.0, [0.85, 0.88, 0.95]);
    scene(&bright.0, [1.0, 0.95, 0.9]);
    let (project, clip) = project_with(&dim.0, &bright.0);
    let sequence = project.active().unwrap().id;
    let err = colour_match(
        &project,
        sequence,
        clip,
        TimelineTime::from_millis(500),
        &NeverCancelled,
    )
    .unwrap_err();
    assert!(err.contains("no picture under the playhead"), "{err}");
}
