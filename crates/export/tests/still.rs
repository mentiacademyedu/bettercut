//! Saving a frame as a PNG (`bettercut_export::still`).
//!
//! The claims a user would notice: the picture is the frame under the
//! playhead — not the first frame of the clip, not a nearby one — at the
//! sequence's full size, and the file holds exactly what was rendered.
//!
//! Every test here opens its own GPU device, and opening several at once
//! deadlocks the driver on this machine (see `pixel_read.rs` in the renderer),
//! so they take one lock and run one at a time. Skips itself without a GPU.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_export::{VideoCodec, render_still, save_still, write_png};
use bettercut_foundation::{FrameRate, MediaTime, TimelineTime};
use bettercut_media::{FfmpegProber, MediaProber, NeverCancelled};
use bettercut_project_format::Project;
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

macro_rules! gpu_or_skip {
    () => {
        if !gpu_available() {
            eprintln!("no GPU adapter; skipping");
            return;
        }
    };
}

/// A file in the temp folder that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("bettercut-still-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

const SOURCE: Resolution = Resolution {
    width: 320,
    height: 180,
};
const FRAMES: u32 = 60;
const BAR: u32 = SOURCE.width / 5;

/// Two seconds of a white bar sweeping left to right across black, so every
/// frame is unmistakably a different frame: the bar's position says which.
fn sweeping_bar(path: &Path) {
    use bettercut_media::{ExportFormat, VideoWriter};

    let (width, height) = (SOURCE.width, SOURCE.height);
    let mut writer = VideoWriter::create(
        path,
        ExportFormat {
            transparent: false,
            width,
            height,
            frame_rate: FrameRate::FPS_30,
            codec: VideoCodec::H264,
            bitrate: Some(2_000_000),
            rate_control: bettercut_export::RateControl::Variable,
            channels: 0,
            threads: 2,
        },
    )
    .expect("open the generated source");
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for frame in 0..FRAMES {
        rgba.fill(0);
        let left = bar_left(frame);
        for y in 0..height {
            for x in left..left + BAR {
                let at = ((y * width + x) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&[235, 235, 235, 255]);
            }
        }
        writer.push_frame(&rgba).expect("write a frame");
    }
    writer.finish().expect("finish the generated source");
}

fn bar_left(frame: u32) -> u32 {
    (frame * (SOURCE.width - BAR)) / (FRAMES - 1)
}

/// The bar source on V1 of a sequence the same size, so a source column is an
/// output column.
fn project_with(source: &Path) -> Project {
    let mut project = Project::new("Still");
    let asset = FfmpegProber.probe(source).expect("probe");
    let duration = asset.duration;
    let media = project.add_media(asset);
    let sequence = project.active_mut().expect("sequence");
    sequence.resolution = SOURCE;
    sequence.frame_rate = FrameRate::FPS_30;
    sequence.video_tracks[0]
        .insert(
            VideoClip::new(
                media,
                TimelineTime::ZERO,
                SourceRange::new(MediaTime::ZERO, duration).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    project
}

/// The middle column of the brightest run along the middle row.
fn bar_centre(rgba: &[u8], width: u32, height: u32) -> u32 {
    let row = height / 2;
    let bright: Vec<u32> = (0..width)
        .filter(|x| rgba[((row * width + x) * 4) as usize] > 128)
        .collect();
    assert!(!bright.is_empty(), "no bar in the frame");
    (bright[0] + bright[bright.len() - 1]) / 2
}

/// The frame under the playhead, not the first frame of the clip: the still
/// taken two thirds of the way in shows the bar two thirds of the way across.
#[test]
fn the_still_is_the_frame_under_the_playhead() {
    let _one = serial();
    gpu_or_skip!();
    let source = Scratch::new("bar.mp4");
    sweeping_bar(&source.0);
    let project = project_with(&source.0);
    let sequence = project.active().unwrap();

    for frame in [0_u32, 40] {
        let at =
            TimelineTime::from_ticks(i64::from(frame) * TimelineTime::from_seconds(1).ticks() / 30);
        let (size, rgba) = render_still(&project, sequence, at, &NeverCancelled).unwrap();
        assert_eq!(size, SOURCE);
        let expected = bar_left(frame) + BAR / 2;
        let found = bar_centre(&rgba, size.width, size.height);
        assert!(
            found.abs_diff(expected) <= 3,
            "frame {frame}: the bar is at {found}, it belongs at {expected}"
        );
    }
}

/// The PNG is the rendered frame to the bit — lossless, full size, opaque.
#[test]
fn the_png_holds_exactly_the_rendered_frame() {
    let _one = serial();
    gpu_or_skip!();
    let source = Scratch::new("bar2.mp4");
    sweeping_bar(&source.0);
    let project = project_with(&source.0);
    let sequence = project.active().unwrap();
    let at = TimelineTime::from_seconds(1);
    let out = Scratch::new("frame.png");

    let (_, rendered) = render_still(&project, sequence, at, &NeverCancelled).unwrap();
    let size = save_still(&project, sequence, at, &out.0, &NeverCancelled).unwrap();

    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&out.0).unwrap(),
    ));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (size.width, size.height));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(
        &pixels[..info.buffer_size()],
        &rendered[..],
        "the PNG is not the rendered frame"
    );
    assert!(
        rendered.chunks_exact(4).all(|px| px[3] == 255),
        "the composite should be opaque"
    );
}

/// A folder that does not exist is an error naming the file, and leaves
/// nothing behind that looks like a picture.
#[test]
fn an_unwritable_path_is_an_error_naming_it() {
    let path = std::env::temp_dir()
        .join("bettercut-no-such-folder-for-stills")
        .join("frame.png");
    let rgba = vec![0_u8; 4 * 4 * 4];

    let err = write_png(&path, Resolution::new(4, 4), &rgba).unwrap_err();

    assert!(err.to_string().contains("frame.png"), "{err}");
    assert!(!path.exists());
}

/// A clip's LUT reaches the rendered picture: the export side reads the
/// project's `.cube` file itself (the renderer reads none), so without that
/// load the clip would render ungraded. An inverting table turns the white bar
/// dark and the black background light.
#[test]
fn a_clips_lut_is_applied() {
    let _one = serial();
    gpu_or_skip!();
    let source = Scratch::new("bar-lut.mp4");
    sweeping_bar(&source.0);
    let cube = Scratch::new("invert.cube");
    let mut text = String::from("LUT_3D_SIZE 2\n");
    for [r, g, b] in &bettercut_timeline::CubeLut::identity(2).table {
        text.push_str(&format!("{} {} {}\n", 1.0 - r, 1.0 - g, 1.0 - b));
    }
    std::fs::write(&cube.0, text).unwrap();

    let mut project = project_with(&source.0);
    let lut = bettercut_foundation::LutId::new();
    project.luts.push(bettercut_project_format::LutAsset {
        id: lut,
        name: "Invert".to_owned(),
        path: cube.0.clone(),
    });
    let sequence = project.active_mut().unwrap();
    let clip = sequence.video_tracks[0].clips()[0].id;
    sequence.video_tracks[0].get_mut(clip).unwrap().lut =
        Some(bettercut_timeline::ClipLut::new(lut));
    let sequence = project.active().unwrap();

    let (size, rgba) =
        render_still(&project, sequence, TimelineTime::ZERO, &NeverCancelled).unwrap();

    let row = size.height / 2;
    let at = |x: u32| rgba[((row * size.width + x) * 4) as usize];
    let bar = BAR / 2; // frame 0: the bar starts at the left edge
    let background = size.width - 10;
    assert!(at(bar) < 60, "the white bar was not inverted: {}", at(bar));
    assert!(
        at(background) > 200,
        "the black background was not inverted: {}",
        at(background)
    );
}

/// A reversed clip's first instant is the material's last frame: through the
/// export path the bar sits where the source's final frame puts it, and a
/// little later it has moved back towards the start.
#[test]
fn a_reversed_clip_plays_its_material_backwards() {
    let _one = serial();
    gpu_or_skip!();
    let source = Scratch::new("bar-reversed.mp4");
    sweeping_bar(&source.0);
    let mut project = project_with(&source.0);
    let sequence = project.active_mut().unwrap();
    let clip = sequence.video_tracks[0].clips()[0].id;
    sequence.video_tracks[0].get_mut(clip).unwrap().reversed = true;
    let sequence = project.active().unwrap();

    let frame = |n: i64| TimelineTime::from_ticks(n * TimelineTime::from_seconds(1).ticks() / 30);
    let (size, first) = render_still(&project, sequence, frame(0), &NeverCancelled).unwrap();
    let (_, later) = render_still(&project, sequence, frame(20), &NeverCancelled).unwrap();

    let at_first = bar_centre(&first, size.width, size.height);
    let at_later = bar_centre(&later, size.width, size.height);
    let last = bar_left(FRAMES - 1) + BAR / 2;
    let twenty_back = bar_left(FRAMES - 1 - 20) + BAR / 2;
    assert!(
        at_first.abs_diff(last) <= 3,
        "the first instant shows {at_first}, not the last frame at {last}"
    );
    assert!(
        at_later.abs_diff(twenty_back) <= 3,
        "twenty frames in shows {at_later}, expected {twenty_back}"
    );
}
