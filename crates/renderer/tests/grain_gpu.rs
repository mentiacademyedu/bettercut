//! Film grain, drawn (`composite.wgsl`'s `add_grain`).
//!
//! Grain has claims that a lazy version fails one at a time: it must actually
//! texture the picture, without shifting its brightness; it must leave pure
//! black black, or the shadows turn to grey haze; it must be the same on the
//! same frame — preview and export, before and after a reload — and different
//! on the next frame, or it reads as dirt on the lens rather than film.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Grade, Layer, RenderConfig};
use bettercut_timeline::{
    AdjustmentLook, ClipLook, ColorAdjust, Crop, MasterLook, Resolution, Transform,
};

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
                label: Some("grain test"),
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

const SIZE: Resolution = Resolution {
    width: 96,
    height: 48,
};

fn flat(value: u8) -> VideoFrame {
    let (w, h) = (SIZE.width, SIZE.height);
    let data: Vec<u8> = (0..w * h)
        .flat_map(|_| [value, value, value, 255])
        .collect();
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

fn layer(frame: &VideoFrame) -> Layer<'_> {
    Layer {
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
            blur: 0.0,
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            pixelate: 0.0,
            zoom_blur: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            chroma_key: None,
            mask: None,
            blend: bettercut_timeline::BlendMode::Normal,
        },
    }
}

/// Every pixel's red channel, in linear light.
fn pixels(compositor: &Compositor) -> Vec<f32> {
    (0..SIZE.height)
        .flat_map(|y| (0..SIZE.width).map(move |x| (x, y)))
        .map(|(x, y)| compositor.read_pixel(x, y).expect("in frame")[0])
        .collect()
}

fn render(compositor: &mut Compositor, value: u8, grain: f32, seed: u32) -> Vec<f32> {
    let frame = flat(value);
    compositor.set_grain_seed(seed);
    compositor
        .composite(
            &[layer(&frame)],
            MasterLook {
                grain,
                ..MasterLook::default()
            },
        )
        .expect("composite");
    pixels(compositor)
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}

fn spread(values: &[f32]) -> f32 {
    let m = mean(values);
    (values.iter().map(|v| (v - m).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
}

fn compositor(device: &wgpu::Device, queue: &wgpu::Queue) -> Compositor {
    Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(SIZE),
    )
    .expect("compositor")
}

/// Grain textures a mid-grey — visibly — without making it lighter or darker
/// on the whole; and none draws exactly the flat picture there was.
#[test]
fn grain_textures_the_picture_without_shifting_it() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    let clean = render(&mut compositor, 128, 0.0, 7);
    assert!(
        spread(&clean) < 1e-4,
        "a flat frame is not flat without grain"
    );

    let grained = render(&mut compositor, 128, 1.0, 7);
    assert!(
        spread(&grained) > 0.01,
        "full grain barely changed the picture: spread {}",
        spread(&grained)
    );
    assert!(
        (mean(&grained) - mean(&clean)).abs() < 0.01,
        "grain shifted the brightness: {} -> {}",
        mean(&clean),
        mean(&grained)
    );
}

/// Pure black stays black: grain lives in the mid-tones, as film's does.
#[test]
fn black_stays_black() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let grained = render(&mut compositor, 0, 1.0, 3);
    let brightest = grained.iter().copied().fold(0.0, f32::max);
    assert!(brightest < 0.002, "grain lifted black to {brightest}");
}

/// The same frame always gets the same grain; the next frame gets different
/// grain.
#[test]
fn grain_is_fixed_per_frame_and_moves_between_frames() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    let first = render(&mut compositor, 128, 1.0, 41);
    let again = render(&mut compositor, 128, 1.0, 41);
    let next = render(&mut compositor, 128, 1.0, 42);

    assert_eq!(first, again, "the same frame drew different grain");
    let changed = first
        .iter()
        .zip(&next)
        .filter(|(a, b)| (*a - *b).abs() > 1e-3)
        .count();
    assert!(
        changed > first.len() / 2,
        "only {changed} of {} pixels changed between frames",
        first.len()
    );
}

/// An adjustment carries grain too, for one scene "on film".
#[test]
fn an_adjustment_grains_what_it_covers() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = flat(128);
    compositor.set_grain_seed(5);
    compositor
        .composite_graded(
            &[layer(&frame)],
            &[Grade {
                beneath: 1,
                look: AdjustmentLook {
                    grain: 1.0,
                    ..AdjustmentLook::default()
                },
            }],
            MasterLook::default(),
        )
        .expect("composite");
    assert!(
        spread(&pixels(&compositor)) > 0.01,
        "the adjustment's grain did not draw"
    );
}
