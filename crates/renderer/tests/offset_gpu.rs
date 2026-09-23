//! What a layer's position actually means, on a real device.
//!
//! §25's moving transitions are built on one assumption: that an offset of 1.0
//! moves a layer by exactly one frame width, so a slide can start with the
//! incoming shot just off the right edge and end with it square. Everything
//! about those transitions is arithmetic on that number, and the arithmetic is
//! tested where it is written — but if the compositor's idea of position were
//! anything else (clip space, pixels, half-widths), every one of those tests
//! would still pass and every slide would be wrong.
//!
//! So this measures it: render a solid layer at a known offset and find where
//! its edge landed.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ClipLook, ColorAdjust, MasterLook, Resolution, Transform, Vec2};

/// One GPU device for the whole binary, shared by every test in it.
///
/// libtest runs tests on parallel threads, and a device per test meant several
/// being created at once — which deadlocks this machine's driver and hung the
/// whole workspace run with no output. `pixel_read.rs` has the full account.
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
                label: Some("offset test"),
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
    width: 320,
    height: 180,
};

fn white(width: u32, height: u32) -> VideoFrame {
    let stride = width * 4;
    VideoFrame {
        timestamp: bettercut_foundation::MediaTime::ZERO,
        width,
        height,
        color: ColorMetadata::srgb(),
        storage: FrameStorage::System {
            data: vec![255; (stride * height) as usize],
            stride,
        },
    }
}

/// The columns a full-frame white layer covers when moved by `offset_x`.
///
/// Returns how many of the output's columns are lit, and the leftmost lit one.
fn columns_lit(device: &wgpu::Device, queue: &wgpu::Queue, offset_x: f32) -> (u32, Option<u32>) {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");

    let frame = white(OUTPUT.width, OUTPUT.height);
    compositor
        .composite(
            &[Layer {
                frame: &frame,

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
                    lens: 0.0,
                    tilt_band: 0.0,
                    tilt_centre: 0.5,
                    posterise: 0.0,
                    vignette: 0.0,
                    reflection: bettercut_timeline::Reflection::None,
                    crop: bettercut_timeline::Crop::NONE,
                    transform: Transform {
                        position: Vec2::new(offset_x, 0.0),
                        ..Transform::default()
                    },
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                },
            }],
            MasterLook::default(),
        )
        .expect("composite");

    let pixels = read_back(device, queue, compositor.target());
    let middle = OUTPUT.height / 2;
    let mut lit = 0;
    let mut first = None;
    for x in 0..OUTPUT.width {
        let index = ((middle * OUTPUT.width + x) * 4) as usize;
        if pixels[index] > 128 {
            lit += 1;
            first.get_or_insert(x);
        }
    }
    (lit, first)
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

#[test]
fn an_unmoved_layer_fills_the_frame() {
    let (device, queue) = gpu_or_skip!();
    let (lit, first) = columns_lit(&device, &queue, 0.0);
    assert_eq!(lit, OUTPUT.width, "a layer at rest does not fill the frame");
    assert_eq!(first, Some(0));
}

/// The number the transitions are built on: one whole frame width.
#[test]
fn an_offset_of_one_moves_the_layer_entirely_off_the_frame() {
    let (device, queue) = gpu_or_skip!();
    let (lit, _) = columns_lit(&device, &queue, 1.0);
    assert!(
        lit <= 1,
        "an offset of 1.0 left {lit} columns on screen, so it is not one frame \
         width — every slide built on that number would be wrong"
    );
}

/// Half of that is half the frame, from the middle rightwards.
#[test]
fn half_an_offset_is_half_the_frame() {
    let (device, queue) = gpu_or_skip!();
    let (lit, first) = columns_lit(&device, &queue, 0.5);

    let half = OUTPUT.width / 2;
    assert!(
        lit.abs_diff(half) <= 2,
        "expected about {half} columns lit, got {lit}"
    );
    assert!(
        first.is_some_and(|x| x.abs_diff(half) <= 2),
        "the layer's left edge is at {first:?}, not the middle"
    );
}

/// And it moves the other way too — a push sends the outgoing shot left.
#[test]
fn a_negative_offset_moves_the_layer_left() {
    let (device, queue) = gpu_or_skip!();
    let (lit, first) = columns_lit(&device, &queue, -0.5);

    let half = OUTPUT.width / 2;
    assert!(
        lit.abs_diff(half) <= 2,
        "expected about {half} columns lit, got {lit}"
    );
    assert_eq!(first, Some(0), "it did not move left");
}
