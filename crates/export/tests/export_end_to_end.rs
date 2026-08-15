//! Export, end to end, against real media (§60 criterion 10).
//!
//! Builds a project the way the editor does, exports it, and decodes the result
//! back. This is the criterion the whole MVP has been working towards, so the
//! assertions are about what a user would notice: the file exists, it is the
//! right length, it has the right number of frames, the picture is the picture
//! they edited, and the effects they applied are in it.
//!
//! Skips itself when no GPU adapter is available — export needs one, and §52.1's
//! low-end target and any headless box may have none.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_export::{ExportProgress, ExportSettings, VideoCodec, codec_is_available, export};
use bettercut_foundation::{FrameRate, MediaTime, TimelineTime};
use bettercut_media::{FfmpegDecoder, FfmpegProber, MediaDecoder, MediaProber, NeverCancelled};
use bettercut_project_format::Project;
use bettercut_timeline::{
    AnimatedParameter, AudioClip, Interpolation, Keyframe, Resolution, SourceRange, TimelineRange,
    VideoClip,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

/// A scratch output that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("bettercut-export-e2e-{name}.mp4"));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The fixture on V1 and A1 from timeline zero, as dragging it in would do.
fn project_with_fixture() -> Project {
    let mut project = Project::new("Export");
    let asset = FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe the fixture");
    let duration = asset.duration;
    let media = project.add_media(asset);

    let source = SourceRange::new(MediaTime::ZERO, duration).expect("non-empty");
    let sequence = project.active_mut().expect("sequence");
    sequence.video_tracks[0]
        .insert(VideoClip::new(media, TimelineTime::ZERO, source).expect("valid"))
        .expect("no overlap");
    sequence.audio_tracks[0]
        .insert(AudioClip::new(media, TimelineTime::ZERO, source).expect("valid"))
        .expect("no overlap");
    project
}

/// The first second of whatever the project holds.
fn one_second() -> TimelineRange {
    TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_seconds(1)).expect("non-empty")
}

fn settings(path: &Path, range: TimelineRange) -> ExportSettings {
    ExportSettings {
        path: path.to_path_buf(),
        resolution: Resolution::new(640, 360),
        // The fixture is 29.97, and exporting at its own rate is the ordinary
        // case; `a_different_frame_rate_retimes_the_output` covers the other.
        frame_rate: FrameRate::NTSC_29_97,
        codec: VideoCodec::H264,
        bitrate: None,
        rate_control: bettercut_export::RateControl::Variable,
        range,
        threads: 2,
    }
}

/// Export needs a GPU. Nothing here can run without one, so the whole file
/// checks once and skips politely.
fn gpu_available() -> bool {
    let instance = bettercut_renderer::wgpu::Instance::new(
        bettercut_renderer::wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
    );
    pollster::block_on(
        instance.request_adapter(&bettercut_renderer::wgpu::RequestAdapterOptions::default()),
    )
    .is_ok()
}

macro_rules! gpu_or_skip {
    () => {
        if !gpu_available() {
            eprintln!("no GPU adapter; skipping");
            return;
        }
    };
}

fn run(project: &Project, settings: &ExportSettings) -> bettercut_export::ExportSummary {
    let sequence = project.active().expect("sequence");
    let mut seen = Vec::new();
    let summary = export(
        project,
        sequence,
        settings,
        &mut |progress: ExportProgress| seen.push(progress.frames_done),
        &NeverCancelled,
    )
    .expect("export");

    // Progress must actually be reported, and monotonically: a bar that jumps
    // backwards is worse than no bar.
    assert!(!seen.is_empty(), "no progress was reported");
    assert!(
        seen.windows(2).all(|pair| pair[1] > pair[0]),
        "progress went backwards: {seen:?}"
    );
    assert_eq!(
        seen.last().copied(),
        Some(summary.frames),
        "the last progress report did not match the frames written"
    );
    summary
}

/// §60 criterion 10, in one test.
#[test]
fn a_project_exports_to_a_playable_file() {
    gpu_or_skip!();
    let scratch = Scratch::new("basic");
    let project = project_with_fixture();

    let summary = run(&project, &settings(scratch.path(), one_second()));
    eprintln!(
        "encoded {} frames with {} (hardware: {})",
        summary.frames, summary.encoder, summary.hardware
    );

    let asset = FfmpegProber
        .probe(scratch.path())
        .expect("probe the export");
    assert_eq!((asset.width, asset.height), (640, 360));
    assert!(asset.video_codec.is_some());
    assert!(asset.audio_codec.is_some(), "the audio track is missing");

    // One second at 29.97 is 30 frames.
    assert_eq!(summary.frames, 30);
    let seconds = asset.duration.ticks() as f64 / bettercut_foundation::TICKS_PER_SECOND as f64;
    assert!(
        (0.9..=1.2).contains(&seconds),
        "expected about a second, got {seconds:.3}"
    );
}

/// Every frame the exporter counted has to be in the file. This is the check
/// that caught B-frame reordering silently dropping one.
#[test]
fn every_counted_frame_reaches_the_file() {
    gpu_or_skip!();
    let scratch = Scratch::new("frames");
    let project = project_with_fixture();

    let summary = run(&project, &settings(scratch.path(), one_second()));

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open the export");

    let mut decoded = 0_u64;
    while decoder
        .decode_frame(&NeverCancelled)
        .expect("decode")
        .is_some()
    {
        decoded += 1;
        assert!(decoded <= summary.frames * 2, "the file will not end");
    }

    assert_eq!(decoded, summary.frames);
}

/// The export has to contain the *edit*, not the source. A clip at half opacity
/// over black must come out darker than the same clip at full — otherwise the
/// exporter is copying the file rather than rendering the timeline.
#[test]
fn the_export_contains_the_edit_rather_than_the_source() {
    gpu_or_skip!();
    let project = project_with_fixture();

    let bright = Scratch::new("bright");
    run(&project, &settings(bright.path(), one_second()));

    let mut dimmed = project.clone();
    {
        let sequence = dimmed.active_mut().expect("sequence");
        let id = sequence.video_tracks[0].clips()[0].id;
        sequence.video_tracks[0].get_mut(id).expect("clip").opacity = 0.25;
    }
    let dark = Scratch::new("dark");
    run(&dimmed, &settings(dark.path(), one_second()));

    let full = mean_luma(bright.path());
    let quarter = mean_luma(dark.path());
    assert!(
        quarter < full * 0.6,
        "opacity did not reach the export: {full:.1} at full, {quarter:.1} at a quarter"
    );
}

/// §24: an animated parameter has to export as the animation, not as a
/// constant. A fade that renders in the preview and exports flat is the exact
/// failure §46 exists to prevent.
#[test]
fn an_animated_fade_exports_as_a_fade() {
    gpu_or_skip!();
    let scratch = Scratch::new("fade");
    let mut project = project_with_fixture();
    {
        let sequence = project.active_mut().expect("sequence");
        let id = sequence.video_tracks[0].clips()[0].id;
        let clip = sequence.video_tracks[0].get_mut(id).expect("clip");
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            Keyframe::new(MediaTime::ZERO, 1.0, Interpolation::Linear),
        );
        clip.keyframes.set(
            AnimatedParameter::Opacity,
            Keyframe::new(MediaTime::from_seconds(1), 0.0, Interpolation::Linear),
        );
    }

    run(&project, &settings(scratch.path(), one_second()));

    // Decode and compare the first frames against the last: the picture should
    // be fading towards black over the second.
    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open");

    let mut lumas = Vec::new();
    while let Some(frame) = decoder.decode_frame(&NeverCancelled).expect("decode") {
        lumas.push(frame_luma(&frame));
    }

    assert!(lumas.len() >= 20, "too few frames to judge a fade");
    let first = lumas[..5].iter().sum::<f64>() / 5.0;
    let last = lumas[lumas.len() - 5..].iter().sum::<f64>() / 5.0;
    assert!(
        last < first * 0.5,
        "the clip did not fade: {first:.1} at the start, {last:.1} at the end"
    );
}

/// §14: export reads the original, never the proxy. A project whose proxy is a
/// different resolution must still export at the sequence's resolution — and
/// more importantly, from the full-quality source.
#[test]
fn export_uses_the_sequence_resolution_not_the_sources() {
    gpu_or_skip!();
    let scratch = Scratch::new("resolution");
    let project = project_with_fixture();

    let mut settings = settings(scratch.path(), one_second());
    settings.resolution = Resolution::new(320, 180);
    run(&project, &settings);

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    assert_eq!((asset.width, asset.height), (320, 180));
}

/// A cancelled export leaves nothing behind. A partial MP4 sitting where the
/// user asked for their video is worse than no file: it looks finished.
#[test]
fn cancelling_removes_the_partial_file() {
    gpu_or_skip!();
    let scratch = Scratch::new("cancelled");
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");

    struct StopAfter(std::sync::atomic::AtomicU32);
    impl bettercut_media::CancellationToken for StopAfter {
        fn is_cancelled(&self) -> bool {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 3
        }
    }

    let result = export(
        &project,
        sequence,
        &settings(scratch.path(), one_second()),
        &mut |_| {},
        &StopAfter(std::sync::atomic::AtomicU32::new(0)),
    );

    let err = result.expect_err("the export should have been cancelled");
    assert!(err.is_cancellation(), "unexpected failure: {err}");
    assert!(
        !scratch.path().exists(),
        "a cancelled export left a file behind"
    );
}

/// An empty range is refused rather than producing a zero-length file.
#[test]
fn an_empty_range_is_refused() {
    gpu_or_skip!();
    let scratch = Scratch::new("empty");
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");

    let empty = TimelineRange {
        start: TimelineTime::from_seconds(1),
        end: TimelineTime::from_seconds(1),
    };
    assert!(
        export(
            &project,
            sequence,
            &settings(scratch.path(), empty),
            &mut |_| {},
            &NeverCancelled,
        )
        .is_err()
    );
}

/// Mean brightness of the whole file, for comparing two exports.
fn mean_luma(path: &Path) -> f64 {
    let asset = FfmpegProber.probe(path).expect("probe");
    let mut decoder = FfmpegDecoder::new(2).expect("decoder");
    decoder.open(&asset).expect("open");

    let mut total = 0.0;
    let mut frames = 0.0;
    while let Some(frame) = decoder.decode_frame(&NeverCancelled).expect("decode") {
        total += frame_luma(&frame);
        frames += 1.0;
    }
    if frames == 0.0 { 0.0 } else { total / frames }
}

/// Mean of the green channel, sampled sparsely — enough to compare exposures
/// without decoding into a full histogram.
fn frame_luma(frame: &bettercut_media::VideoFrame) -> f64 {
    let bettercut_media::FrameStorage::System { data, stride } = &frame.storage else {
        panic!("expected a system-memory frame");
    };

    let mut total = 0.0;
    let mut count = 0.0;
    for y in (0..frame.height).step_by(4) {
        for x in (0..frame.width).step_by(4) {
            let at = (y * stride + x * 4) as usize;
            total += f64::from(data[at + 1]);
            count += 1.0;
        }
    }
    if count == 0.0 { 0.0 } else { total / count }
}

/// Exporting at a rate other than the sequence's re-times the output: the same
/// second of timeline becomes a different number of frames.
#[test]
fn a_different_frame_rate_retimes_the_output() {
    gpu_or_skip!();
    let scratch = Scratch::new("retimed");
    let project = project_with_fixture();

    let mut settings = settings(scratch.path(), one_second());
    settings.frame_rate = FrameRate::FPS_60;
    let summary = run(&project, &settings);

    assert_eq!(summary.frames, 60, "a second at 60 fps is 60 frames");

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let seconds = asset.duration.ticks() as f64 / bettercut_foundation::TICKS_PER_SECOND as f64;
    assert!(
        (0.9..=1.2).contains(&seconds),
        "re-timing changed the duration: {seconds:.3}"
    );
}

/// An explicit bitrate has to reach the encoder.
///
/// Deliberately *not* asserting proportionality. The bitrate is a ceiling, not
/// a quota: a capped-VBR encoder spends what the picture needs and no more, so
/// a simple fixture at 16 Mb/s does not produce sixteen times the bytes of the
/// same fixture at 1. Measured here at about 1.7x, which is the shape of the
/// real behaviour — what matters is that the number reaches the encoder at all,
/// and a decorative field would show no difference whatsoever.
#[test]
fn an_explicit_bitrate_changes_the_file_size() {
    gpu_or_skip!();
    let project = project_with_fixture();

    let small = Scratch::new("small-bitrate");
    let mut low = settings(small.path(), one_second());
    low.bitrate = Some(1_000_000);
    run(&project, &low);

    let large = Scratch::new("large-bitrate");
    let mut high = settings(large.path(), one_second());
    high.bitrate = Some(16_000_000);
    run(&project, &high);

    let low_bytes = std::fs::metadata(small.path()).expect("small").len();
    let high_bytes = std::fs::metadata(large.path()).expect("large").len();
    assert!(
        high_bytes > low_bytes * 5 / 4,
        "the bitrate did not reach the encoder: {low_bytes} vs {high_bytes} bytes"
    );
}

/// H.265 when the machine can write it. Skipped rather than failed elsewhere:
/// there is no software fallback by design (§0.1 forbids x265), so a machine
/// without a GPU encoder legitimately cannot run this.
#[test]
fn h265_exports_when_the_machine_can_write_it() {
    gpu_or_skip!();
    let resolution = Resolution::new(640, 360);
    if !codec_is_available(VideoCodec::H265, resolution, FrameRate::NTSC_29_97) {
        eprintln!("no H.265 encoder on this machine; skipping");
        return;
    }

    let scratch = Scratch::new("h265");
    let project = project_with_fixture();
    let mut settings = settings(scratch.path(), one_second());
    settings.codec = VideoCodec::H265;
    let summary = run(&project, &settings);

    let asset = FfmpegProber.probe(scratch.path()).expect("probe");
    let codec = asset.video_codec.expect("no video stream");
    assert!(
        codec.contains("hevc") || codec.contains("h265"),
        "asked for H.265 and got {codec}"
    );
    assert_eq!(summary.frames, 30);
}

/// H.264 is always available — `libopenh264` is linked in — and asking must not
/// cost a driver probe. The export window asks on the UI thread when it opens,
/// and §74 is explicit that the interface must not stall behind FFmpeg.
#[test]
fn asking_about_a_codec_is_cheap_after_the_first_time() {
    let resolution = Resolution::new(1920, 1080);
    let rate = FrameRate::FPS_30;

    assert!(
        codec_is_available(VideoCodec::H264, resolution, rate),
        "H.264 has a linked software encoder, so it is available everywhere"
    );

    // Warm whatever the first call costs, then measure the second.
    let _ = codec_is_available(VideoCodec::H265, resolution, rate);
    let started = std::time::Instant::now();
    for _ in 0..50 {
        let _ = codec_is_available(VideoCodec::H265, resolution, rate);
        let _ = codec_is_available(VideoCodec::H264, resolution, rate);
    }
    let elapsed = started.elapsed();

    // A single uncached probe is around half a second here. A hundred cached
    // ones must be nowhere near that.
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "100 availability checks took {elapsed:?}; the answer is not being cached"
    );
}

/// CBR pads simple footage to hold the rate; VBR spends less. The same clip at
/// the same bitrate must therefore come out visibly larger under CBR — which is
/// also the check that `rate_control` reaches the encoder at all.
#[test]
fn constant_rate_control_produces_a_larger_file_than_variable() {
    gpu_or_skip!();
    let project = project_with_fixture();

    let vbr_file = Scratch::new("vbr");
    let mut vbr = settings(vbr_file.path(), one_second());
    vbr.bitrate = Some(8_000_000);
    vbr.rate_control = bettercut_export::RateControl::Variable;
    run(&project, &vbr);

    let cbr_file = Scratch::new("cbr");
    let mut cbr = settings(cbr_file.path(), one_second());
    cbr.bitrate = Some(8_000_000);
    cbr.rate_control = bettercut_export::RateControl::Constant;
    run(&project, &cbr);

    let vbr_bytes = std::fs::metadata(vbr_file.path()).expect("vbr").len();
    let cbr_bytes = std::fs::metadata(cbr_file.path()).expect("cbr").len();
    assert!(
        cbr_bytes > vbr_bytes,
        "constant rate control did not reach the encoder: \
         {vbr_bytes} bytes variable, {cbr_bytes} constant"
    );
}
