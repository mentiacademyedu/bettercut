//! Blur, on a real device, checked against the picture it produces.
//!
//! Everything else about blur is tested on the CPU side — the plan, the byte
//! layout, the shader's well-formedness. None of that would notice a kernel
//! that blurs by the wrong amount, loses brightness, or is silently bypassed.
//!
//! This is the first test in the tree that renders and reads back pixels, so
//! it is also the shape §51.1's golden-frame comparison will take once export
//! exists: build a frame, composite it under two [`RenderConfig`]s, and compare
//! the results.
//!
//! It skips itself when no adapter can be found. §52.1's low-end target and any
//! headless CI box may have no usable GPU, and a test that cannot run there
//! must not fail there.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, QualityTier, RenderConfig};
use bettercut_timeline::{ColorAdjust, MasterLook, Resolution, Transform};

/// Square, so the composite's letterboxing maps the source onto the target one
/// texel to one texel and the readback can be compared with the input.
const SIZE: u32 = 512;
/// A white square in the middle, small enough that even the widest blur here
/// stays clear of the edges — `ClampToEdge` would otherwise pull in border
/// texels and add brightness the energy check is meant to detect.
const SQUARE: u32 = 64;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("blur test"),
        ..Default::default()
    }))
    .ok()
}

/// An opaque black frame with a white square in the centre.
///
/// Opaque matters. A decoded video frame always is — §21a.1's working space is
/// RGBA only because the GPU wants four channels, and the alpha FFmpeg writes
/// is 255 everywhere. Leaving the background transparent here instead makes the
/// composite multiply the blurred colour by the blurred alpha, which reads as
/// blur that both loses brightness and spreads less far than it should: a
/// convincing shader bug that is really a fixture bug.
///
/// (The flip side is a real constraint: this blur assumes straight, opaque
/// alpha. Blurring a genuinely translucent layer would need premultiplication
/// first, or colour bleeds out of the transparent regions. Nothing produces one
/// today, and text and masks are Phase 2.)
fn test_frame() -> VideoFrame {
    let stride = SIZE * 4;
    let mut data = vec![0_u8; (stride * SIZE) as usize];
    for pixel in data.chunks_exact_mut(4) {
        pixel[3] = 255;
    }

    let low = (SIZE - SQUARE) / 2;
    let high = low + SQUARE;
    for y in low..high {
        for x in low..high {
            let at = (y * stride + x * 4) as usize;
            data[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }

    VideoFrame {
        timestamp: bettercut_foundation::MediaTime::ZERO,
        width: SIZE,
        height: SIZE,
        color: ColorMetadata::bt709_limited_8bit(),
        storage: FrameStorage::System { data, stride },
    }
}

/// Composite one layer and read the result back as linear-light red values.
///
/// Linear, not the stored bytes: the target is sRGB-encoded, and averaging is
/// what blur does, so any claim about brightness is only meaningful in the
/// space the averaging happened in.
fn render(device: &wgpu::Device, queue: &wgpu::Queue, blur: f32, tier: QualityTier) -> Vec<f32> {
    let resolution = Resolution::new(SIZE, SIZE);
    let config = match tier {
        QualityTier::Preview => RenderConfig::preview(resolution),
        QualityTier::Full => RenderConfig::export_to_texture(resolution),
    };

    let mut compositor =
        Compositor::new(device.clone(), queue.clone(), config).expect("compositor");
    let frame = test_frame();
    compositor
        .composite(
            &[Layer {
                frame: &frame,
                transform: Transform::default(),
                opacity: 1.0,
                color: ColorAdjust::default(),
                blur,
            }],
            MasterLook::default(),
        )
        .expect("composite");

    read_back(device, queue, compositor.target())
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<f32> {
    // wgpu requires copy rows to be a multiple of 256 bytes.
    let unpadded = SIZE * 4;
    let padded = unpadded.div_ceil(256) * 256;

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("readback encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");

    let mapped = slice.get_mapped_range().expect("mapped");
    let mut out = Vec::with_capacity((SIZE * SIZE) as usize);
    for y in 0..SIZE {
        let row = (y * padded) as usize;
        for x in 0..SIZE {
            out.push(srgb_to_linear(mapped[row + (x * 4) as usize]));
        }
    }
    drop(mapped);
    buffer.unmap();
    out
}

/// The inverse of what the sRGB-aware target applied on write (§21a.1).
fn srgb_to_linear(byte: u8) -> f32 {
    let value = f32::from(byte) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn sum(pixels: &[f32]) -> f32 {
    pixels.iter().sum()
}

fn at(pixels: &[f32], x: u32, y: u32) -> f32 {
    pixels[(y * SIZE + x) as usize]
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

/// Zero blur must be bit-for-bit the old path, not a one-tap kernel that
/// happens to look similar.
#[test]
fn no_blur_leaves_a_hard_edge() {
    let (device, queue) = gpu_or_skip!();
    let sharp = render(&device, &queue, 0.0, QualityTier::Full);

    let low = (SIZE - SQUARE) / 2;
    assert!(at(&sharp, SIZE / 2, SIZE / 2) > 0.99, "centre is not white");
    assert!(
        at(&sharp, low - 1, SIZE / 2) < 0.01,
        "the texel outside the square is not black: {}",
        at(&sharp, low - 1, SIZE / 2)
    );
}

/// The claim `blur.wgsl` makes when it divides by the summed weights rather
/// than the analytic constant: blurring moves light around, it does not create
/// or destroy it.
///
/// Getting this wrong is the classic Gaussian bug — a truncated kernel whose
/// weights sum to less than one darkens every blurred clip, and darkens it
/// more the wider the blur, so it looks like a vignette rather than a mistake.
#[test]
fn blur_preserves_total_brightness() {
    let (device, queue) = gpu_or_skip!();

    let sharp = sum(&render(&device, &queue, 0.0, QualityTier::Full));
    for amount in [25.0, 60.0, 100.0] {
        let blurred = sum(&render(&device, &queue, amount, QualityTier::Full));
        let error = (blurred - sharp).abs() / sharp;
        // Measured at 0.2% on Vulkan; the margin is for 8-bit rounding
        // differing between drivers, not for slack in the maths.
        assert!(
            error < 0.005,
            "blur {amount} changed total brightness by {:.1}% ({sharp} -> {blurred})",
            error * 100.0
        );
    }
}

/// Blur has to actually spread light, and spread more of it the higher the
/// slider goes — otherwise the control is decorative.
#[test]
fn blur_spreads_light_and_more_of_it_at_higher_amounts() {
    let (device, queue) = gpu_or_skip!();

    let low = (SIZE - SQUARE) / 2;
    // A texel well outside the square: only blur can put light here.
    let outside = (low - 12, SIZE / 2);

    let sharp = render(&device, &queue, 0.0, QualityTier::Full);
    let some = render(&device, &queue, 40.0, QualityTier::Full);
    let lots = render(&device, &queue, 100.0, QualityTier::Full);

    assert!(at(&sharp, outside.0, outside.1) < 0.001);
    assert!(
        at(&some, outside.0, outside.1) > 0.001,
        "40% blur did not reach 12 texels out"
    );
    assert!(
        at(&lots, outside.0, outside.1) > at(&some, outside.0, outside.1),
        "100% blur did not spread further than 40%"
    );

    // And the centre gives up light as it spreads.
    assert!(at(&lots, SIZE / 2, SIZE / 2) < at(&some, SIZE / 2, SIZE / 2));
}

/// §46's central claim, made testable.
///
/// Preview spends 16 taps a side and export spends 40, so the kernels are
/// sampled quite differently — but they cover the same radius, and the picture
/// a user approves in the preview has to be the picture that gets exported.
#[test]
fn the_preview_and_export_tiers_agree() {
    let (device, queue) = gpu_or_skip!();

    let preview = render(&device, &queue, 100.0, QualityTier::Preview);
    let full = render(&device, &queue, 100.0, QualityTier::Full);

    let peak = full.iter().copied().fold(0.0_f32, f32::max);
    let worst = preview
        .iter()
        .zip(&full)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    let mean = preview
        .iter()
        .zip(&full)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / preview.len() as f32;

    assert!(
        mean < 0.005 * peak,
        "preview and export differ by {mean:.5} on average (peak {peak:.3})"
    );
    assert!(
        worst < 0.08 * peak,
        "preview and export differ by {worst:.5} at the worst texel (peak {peak:.3})"
    );
}
