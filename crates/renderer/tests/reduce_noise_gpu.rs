//! Reduce noise on the GPU: grain over a dark and a bright patch comes out
//! smoother, while the edge between them stays where it was and as sharp —
//! read back from a rendered frame, so it is the shader answering
//! (`VideoClip::reduce_noise`).
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
                label: Some("reduce noise test"),
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

/// `frame` drawn with `noise` reduction, read back whole (tight RGBA rows).
fn held(device: &wgpu::Device, queue: &wgpu::Queue, frame: &VideoFrame, noise: f32) -> Vec<u8> {
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
            smooth_skin: 0.0,
            reduce_noise: noise,
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

/// Dark grey on the left, light on the right, each with a fine checker of
/// grain, and a hard edge between them.
fn patches() -> VideoFrame {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let grain: i32 = if (x + y) % 2 == 0 { 10 } else { -10 };
            let base = if x < w / 2 { 60 } else { 200 };
            let level = (base + grain).clamp(0, 255) as u8;
            data.extend_from_slice(&[level, level, level, 255]);
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

fn red(pixels: &[u8], x: u32) -> i32 {
    let y = OUTPUT.height / 2;
    i32::from(pixels[((y * OUTPUT.width + x) * 4) as usize])
}

/// How much the red channel jumps from pixel to pixel along the middle row,
/// over `from..to`: the grain left.
fn roughness(pixels: &[u8], from: u32, to: u32) -> u32 {
    (from..to - 1)
        .map(|x| red(pixels, x).abs_diff(red(pixels, x + 1)))
        .sum()
}

#[test]
fn grain_is_smoothed_and_the_edge_kept() {
    let (device, queue) = gpu_or_skip!();
    let frame = patches();
    let before = held(&device, &queue, &frame, 0.0);
    let after = held(&device, &queue, &frame, 1.0);
    let (dark, light) = ((4, 26), (38, 60));

    for (name, (from, to)) in [("dark", dark), ("light", light)] {
        assert!(
            roughness(&after, from, to) * 2 < roughness(&before, from, to),
            "the {name} side kept its grain: {} then {}",
            roughness(&before, from, to),
            roughness(&after, from, to)
        );
    }
    // Either side of the edge, each pixel is still its own side's grey: the
    // two were not blurred into each other.
    let (left, right) = (OUTPUT.width / 2 - 1, OUTPUT.width / 2);
    assert!(
        red(&after, left) < 90,
        "the dark side's edge brightened to {}",
        red(&after, left)
    );
    assert!(
        red(&after, right) > 170,
        "the light side's edge darkened to {}",
        red(&after, right)
    );
}

#[test]
fn none_leaves_the_picture_as_it_was() {
    let (device, queue) = gpu_or_skip!();
    let frame = patches();
    let FrameStorage::System { data, .. } = &frame.storage else {
        unreachable!()
    };
    let drawn = held(&device, &queue, &frame, 0.0);
    // Within a level of the source: drawn as is.
    for (a, b) in drawn.iter().zip(data) {
        assert!(a.abs_diff(*b) <= 1, "{a} drawn for {b}");
    }
}
