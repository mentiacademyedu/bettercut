//! Smooth skin on the GPU: a noisy skin-coloured patch comes out smoother,
//! a noisy blue one does not — read back from a rendered frame, so it is
//! the shader answering (`VideoClip::smooth_skin`).
//!
//! Skips itself when no adapter can be found, like `blend_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{BlendMode, ClipLook, ColorAdjust, MasterLook, Resolution, Transform};

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
                label: Some("smooth skin test"),
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

const OUTPUT: Resolution = Resolution {
    width: 64,
    height: 16,
};

/// `frame` drawn with `smooth` skin smoothing, read back whole (tight RGBA rows).
fn held(device: &wgpu::Device, queue: &wgpu::Queue, frame: &VideoFrame, smooth: f32) -> Vec<u8> {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");

    let layer = Layer {
        frame,
        look: ClipLook {
            corner_pin: Default::default(),
            old_film: 0.0,
            glow: 0.0,
            shadow: Default::default(),
            border: Default::default(),
            sharpen: 0.0,
            lut: None,
            rgb_split: 0.0,
            glitch: 0.0,
            pixelate: 0.0,
            zoom_blur: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform::default(),
            opacity: 1.0,
            color: ColorAdjust::default(),
            lens: 0.0,
            tilt_band: 0.0,
            tilt_centre: 0.5,
            posterise: 0.0,
            smooth_skin: smooth,
            blur: 0.0,
            chroma_key: None,
            luma_key: None,
            mask: None,
            blend: BlendMode::Normal,
        },
    };
    compositor
        .composite(&[layer], MasterLook::default())
        .expect("composite");

    read_back(device, queue, compositor.target())
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let padded = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("read back"),
        size: u64::from(padded * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    let mapped = slice.get_mapped_range().expect("mapped");
    let mut tight = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h {
        let start = (row * padded) as usize;
        tight.extend_from_slice(&mapped[start..start + (w * 4) as usize]);
    }
    drop(mapped);
    buffer.unmap();
    tight
}

/// Skin on the left, blue on the right, each with a fine checker of noise.
fn patches() -> VideoFrame {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let bump: i32 = if (x + y) % 2 == 0 { 14 } else { -14 };
            let base: [i32; 3] = if x < w / 2 {
                [205, 150, 120]
            } else {
                [60, 90, 200]
            };
            let px = base.map(|c| (c + bump).clamp(0, 255) as u8);
            data.extend_from_slice(&[px[0], px[1], px[2], 255]);
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

/// How much the red channel jumps from pixel to pixel along the middle row,
/// over `from..to`: the noise left.
fn roughness(pixels: &[u8], from: u32, to: u32) -> u32 {
    let y = OUTPUT.height / 2;
    let red = |x: u32| i32::from(pixels[((y * OUTPUT.width + x) * 4) as usize]);
    (from..to - 1).map(|x| red(x).abs_diff(red(x + 1))).sum()
}

#[test]
fn skin_is_smoothed_and_blue_is_left_alone() {
    let (device, queue) = gpu_or_skip!();
    let frame = patches();
    let before = held(&device, &queue, &frame, 0.0);
    let after = held(&device, &queue, &frame, 1.0);
    let (skin, blue) = ((4, 28), (36, 60));

    assert!(
        roughness(&after, skin.0, skin.1) * 2 < roughness(&before, skin.0, skin.1),
        "the skin kept its noise: {} then {}",
        roughness(&before, skin.0, skin.1),
        roughness(&after, skin.0, skin.1)
    );
    assert_eq!(
        roughness(&after, blue.0, blue.1),
        roughness(&before, blue.0, blue.1),
        "the blue was smoothed too"
    );
}
