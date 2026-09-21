//! Exporting with a transparent background (`ExportSettings::transparent`).
//!
//! The end-to-end test opens a GPU and skips itself without one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_export::{ExportSettings, export, unpremultiply};
use bettercut_foundation::{FrameRate, MediaTime, TimelineTime};
use bettercut_media::{FfmpegProber, Generated, MediaAsset, MediaProber, NeverCancelled};
use bettercut_project_format::Project;
use bettercut_timeline::{Resolution, SourceRange, TimelineRange, Vec2, VideoClip};

fn gpu_available() -> bool {
    let instance = bettercut_renderer::wgpu::Instance::new(
        bettercut_renderer::wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
    );
    pollster::block_on(
        instance.request_adapter(&bettercut_renderer::wgpu::RequestAdapterOptions::default()),
    )
    .is_ok()
}

#[test]
fn straight_alpha_undoes_the_premultiply_in_linear_light() {
    // Half-covered white: premultiplied in linear light is 0.5, which the
    // sRGB target stores as 188.
    let mut pixels = vec![188, 188, 188, 128, 10, 20, 30, 0, 50, 60, 70, 255];
    unpremultiply(&mut pixels);
    assert!(pixels[0] >= 253, "{pixels:?}");
    assert_eq!(&pixels[4..8], &[0, 0, 0, 0], "fully clear is black");
    assert_eq!(&pixels[8..12], &[50, 60, 70, 255], "opaque is untouched");
}

#[test]
fn a_transparent_export_is_vp9_webm_with_alpha() {
    if !gpu_available() {
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let mut project = Project::new("Sticker");
    let media = project.add_media(MediaAsset::generated(
        Generated::solid([255, 0, 0]),
        160,
        90,
    ));
    let sequence = project.active_mut().unwrap();
    sequence.resolution = Resolution {
        width: 160,
        height: 90,
    };
    sequence.frame_rate = FrameRate::FPS_30;
    let mut clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(1)).unwrap(),
    )
    .unwrap();
    clip.transform.scale = Vec2::new(0.5, 0.5);
    sequence.video_tracks[0].insert(clip).unwrap();
    let sequence = project.active().unwrap().clone();

    let out = std::env::temp_dir().join(format!("bettercut-alpha-{}.webm", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let mut settings = ExportSettings::for_sequence(out.clone(), &sequence);
    settings.transparent = true;
    settings.frame_rate = FrameRate::new(10, 1).unwrap();
    settings.range = TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_seconds(1)).unwrap();

    let summary = export(&project, &sequence, &settings, &mut |_| {}, &NeverCancelled)
        .expect("export with a transparent background");
    assert_eq!(summary.frames, 10);

    let probed = FfmpegProber.probe(&out).expect("probe the webm");
    assert_eq!(probed.video_codec.as_deref(), Some("vp9"));
    assert!(
        probed.audio_codec.is_none(),
        "a transparent export carries no sound"
    );
    // WebM marks a stream with an alpha plane with this tag.
    let bytes = std::fs::read(&out).unwrap();
    let marked = bytes
        .windows(10)
        .any(|w| w.eq_ignore_ascii_case(b"alpha_mode"));
    assert!(marked, "the file does not say it has alpha");
    let _ = std::fs::remove_file(&out);
}
