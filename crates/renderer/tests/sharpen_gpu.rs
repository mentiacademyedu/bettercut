//! Sharpen, drawn (`crate::sharpen`).
//!
//! A sharpen has two claims that pull in opposite directions, and each alone is
//! satisfied by something that is not a sharpen: an edge must gain contrast (a
//! contrast boost does that too), and a flat patch must not change (doing
//! nothing does that too). Both are measured.
//!
//! And one claim about the graph it is part of: it is the second node, so a
//! clip with a blur and a sharpen must reach the sharpen with the blur's output
//! rather than the original frame — the first time the graph has had two nodes
//! to hand a picture between.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ClipLook, ColorAdjust, Crop, MasterLook, Resolution, Transform};

/// One GPU device for the whole binary; `pixel_read.rs` has the account of why.
fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    static SHARED: std::sync::OnceLock<Option<(wgpu::Device, wgpu::Queue)>> =
        std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            let instance = wgpu::Instance::new(
                wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
            );
            let adapter = pollster::block_on(
                instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
            )
            .ok()?;
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("sharpen test"),
                ..Default::default()
            }))
            .ok()
        })
        .clone()
}

macro_rules! gpu_or_skip {
    () => {
        match device() {
            Some(pair) => pair,
            None => {
                eprintln!("no GPU adapter; skipping");
                return;
            }
        }
    };
}

/// Source and frame the same size, so one source texel is one output texel and
/// a one-texel sharpen is measurable where it happens.
const SIZE: Resolution = Resolution {
    width: 128,
    height: 64,
};

/// A vertical step between two greys, neither at the ends of the range, so a
/// sharpen can push the dark side darker and the bright side brighter without
/// either being clipped away.
fn step() -> VideoFrame {
    let (w, h) = (SIZE.width, SIZE.height);
    let mut data = vec![0_u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let v = if x < w / 2 { 70 } else { 170 };
            let at = ((y * w + x) * 4) as usize;
            data[at..at + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    VideoFrame {
        timestamp: bettercut_foundation::MediaTime::ZERO,
        width: w,
        height: h,
        color: ColorMetadata::srgb(),
        storage: FrameStorage::System {
            data,
            stride: w * 4,
        },
    }
}

fn render(compositor: &mut Compositor, frame: &VideoFrame, blur: f32, sharpen: f32) {
    compositor
        .composite(
            &[Layer {
                frame,
                look: ClipLook {
                    corner_pin: Default::default(),
                    old_film: 0.0,
                    glow: 0.0,
                    shadow: Default::default(),
                    border: Default::default(),
                    crop: Crop::NONE,
                    transform: Transform::default(),
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur,
                    sharpen,
                    lut: None,
                    rgb_split: 0.0,
                    glitch: 0.0,
                    pixelate: 0.0,
                    zoom_blur: 0.0,
                    lens: 0.0,
                    tilt_band: 0.0,
                    tilt_centre: 0.5,
                    posterise: 0.0,
                    smooth_skin: 0.0,
                    vignette: 0.0,
                    reflection: bettercut_timeline::Reflection::None,
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                },
            }],
            MasterLook::default(),
        )
        .expect("composite");
}

fn compositor(device: &wgpu::Device, queue: &wgpu::Queue) -> Compositor {
    Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(SIZE),
    )
    .expect("compositor")
}

/// The texel either side of the step, and one far from it on the dark side.
fn probes(compositor: &Compositor) -> (f32, f32, f32) {
    let row = SIZE.height / 2;
    let dark_edge = compositor
        .read_pixel(SIZE.width / 2 - 1, row)
        .expect("in frame")[0];
    let bright_edge = compositor
        .read_pixel(SIZE.width / 2, row)
        .expect("in frame")[0];
    let flat = compositor.read_pixel(8, row).expect("in frame")[0];
    (dark_edge, bright_edge, flat)
}

#[test]
fn a_sharpen_widens_an_edge_and_leaves_flat_areas_alone() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = step();

    render(&mut compositor, &frame, 0.0, 0.0);
    let (dark_before, bright_before, flat_before) = probes(&compositor);

    render(&mut compositor, &frame, 0.0, 100.0);
    let (dark_after, bright_after, flat_after) = probes(&compositor);

    assert!(
        dark_after < dark_before - 0.02,
        "the dark side of the edge did not get darker: {dark_before} -> {dark_after}"
    );
    assert!(
        bright_after > bright_before + 0.02,
        "the bright side of the edge did not get brighter: {bright_before} -> {bright_after}"
    );
    assert!(
        (flat_after - flat_before).abs() < 0.01,
        "a flat patch changed, so this is a contrast boost, not a sharpen: \
         {flat_before} -> {flat_after}"
    );
}

/// No sharpen draws exactly what no sharpen used to — the node declines the
/// pass rather than running it at zero.
#[test]
fn no_sharpen_changes_nothing() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = step();

    render(&mut compositor, &frame, 0.0, 0.0);
    let untouched = probes(&compositor);
    assert_eq!(
        compositor.intermediate_count(),
        0,
        "a clip with no effects took an intermediate texture"
    );
    render(&mut compositor, &frame, 0.0, 0.0);
    assert_eq!(probes(&compositor), untouched);
}

/// **The graph is a graph.** The sharpen is the second node, so on a blurred and
/// sharpened clip it must read the blur's output, not the original frame.
///
/// Measured *away* from the edge, deliberately. A strong blur lifts the dark
/// side several texels out from the step; the sharpen only reaches one texel,
/// so out there it cannot undo that. If the sharpen reads the blurred picture,
/// the lift survives. If it were handed the original frame, that texel would be
/// the flat original grey again — the sharpen leaves flat areas alone — and the
/// blur would simply be gone.
///
/// Not measured at the edge itself: a sharpen at double gain overshoots, and
/// "brighter than the unblurred edge" is then true of a correct chain and an
/// incorrect one alike. That was this test's first version, and it failed on a
/// chain that was working.
#[test]
fn a_blurred_clip_is_sharpened_after_it_is_blurred() {
    let (device, queue) = gpu_or_skip!();
    // Taller, so a blur reaches well past the sharpen's one texel.
    let size = Resolution::new(256, 128);
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(size),
    )
    .expect("compositor");
    let frame = {
        let (w, h) = (size.width, size.height);
        let mut data = vec![0_u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let v = if x < w / 2 { 70 } else { 170 };
                let at = ((y * w + x) * 4) as usize;
                data[at..at + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        VideoFrame {
            timestamp: bettercut_foundation::MediaTime::ZERO,
            width: w,
            height: h,
            color: ColorMetadata::srgb(),
            storage: FrameStorage::System {
                data,
                stride: w * 4,
            },
        }
    };
    // Four texels out on the dark side: beyond the sharpen, within the blur.
    let out_there = (size.width / 2 - 4, size.height / 2);
    let mut read = |blur: f32, sharpen: f32| {
        render(&mut compositor, &frame, blur, sharpen);
        compositor
            .read_pixel(out_there.0, out_there.1)
            .expect("in frame")[0]
    };

    let original = read(0.0, 0.0);
    let blurred = read(100.0, 0.0);
    let both = read(100.0, 100.0);

    assert!(
        blurred > original + 0.02,
        "the blur did not reach the probe, so the test proves nothing: \
         original {original}, blurred {blurred}"
    );
    assert!(
        both > original + 0.02,
        "the blur vanished once sharpened, so the sharpen read the original frame: \
         original {original}, blurred {blurred}, blurred and sharpened {both}"
    );
}
