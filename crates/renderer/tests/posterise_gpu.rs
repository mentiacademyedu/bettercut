//! Posterise on the GPU: a smooth ramp comes out as steps, black and white
//! stay black and white, and off leaves the ramp smooth — read back from a
//! rendered frame, so it is the shader answering (`VideoClip::posterise`).
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
                label: Some("posterise test"),
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
    height: 4,
};

/// `frame` drawn with `posterise` levels, read back whole (tight RGBA rows).
fn held(device: &wgpu::Device, queue: &wgpu::Queue, frame: &VideoFrame, posterise: f32) -> Vec<u8> {
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
            posterise,
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

/// A grey ramp, black at the left edge to white at the right.
fn ramp() -> VideoFrame {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..h {
        for x in 0..w {
            let v = (x * 255 / (w - 1)) as u8;
            data.extend_from_slice(&[v, v, v, 255]);
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

/// The red channel across the middle row.
fn row(pixels: &[u8]) -> Vec<u8> {
    let y = OUTPUT.height / 2;
    (0..OUTPUT.width)
        .map(|x| pixels[((y * OUTPUT.width + x) * 4) as usize])
        .collect()
}

fn distinct(values: &[u8]) -> usize {
    let mut seen: Vec<u8> = values.to_vec();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

#[test]
fn off_leaves_the_ramp_smooth() {
    let (device, queue) = gpu_or_skip!();
    let smooth = row(&held(&device, &queue, &ramp(), 0.0));
    assert!(
        distinct(&smooth) > 40,
        "the ramp lost its steps: {smooth:?}"
    );
    assert!(
        smooth.windows(2).all(|p| p[1] >= p[0]),
        "not rising: {smooth:?}"
    );
}

/// Two levels is black or white and nothing else; four is four steps, the
/// ends still black and white.
#[test]
fn levels_hold_the_ramp_to_that_many_greys() {
    let (device, queue) = gpu_or_skip!();
    let frame = ramp();
    let two = row(&held(&device, &queue, &frame, 2.0));
    assert!(
        two.iter().all(|v| *v < 8 || *v > 247),
        "two levels left a mid grey: {two:?}"
    );
    assert!(
        two[0] < 8 && two[OUTPUT.width as usize - 1] > 247,
        "{two:?}"
    );

    let four = row(&held(&device, &queue, &frame, 4.0));
    assert_eq!(distinct(&four), 4, "four levels: {four:?}");
    assert!(
        four[0] < 8 && four[OUTPUT.width as usize - 1] > 247,
        "{four:?}"
    );
    assert!(
        four.windows(2).all(|p| p[1] >= p[0]),
        "not rising: {four:?}"
    );
}
