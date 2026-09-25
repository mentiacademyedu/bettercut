//! Cinematic bars on the master draw, on a real device: solid black top and
//! bottom, the picture untouched between.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ClipLook, ColorAdjust, MasterLook, Resolution, Transform};

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
                label: Some("bars test"),
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
    height: 64,
};

fn flat(colour: [u8; 4]) -> VideoFrame {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    VideoFrame {
        timestamp: bettercut_foundation::MediaTime::ZERO,
        width: w,
        height: h,
        color: ColorMetadata::srgb(),
        storage: FrameStorage::System {
            data: colour.repeat((w * h) as usize),
            stride: w * 4,
        },
    }
}

/// A white layer over a red ground, with cinematic bars cutting the frame to
/// `bars`.
fn render(device: &wgpu::Device, queue: &wgpu::Queue, bars: f32) -> Vec<u8> {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");

    let ground = flat([255, 0, 0, 255]);
    let subject = flat([255, 255, 255, 255]);
    compositor
        .composite(
            &[
                Layer {
                    frame: &ground,

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
                        smooth_skin: 0.0,
                        vignette: 0.0,
                        reflection: bettercut_timeline::Reflection::None,
                        crop: bettercut_timeline::Crop::NONE,
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: None,
                        luma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
                Layer {
                    frame: &subject,

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
                        smooth_skin: 0.0,
                        vignette: 0.0,
                        reflection: bettercut_timeline::Reflection::None,
                        crop: bettercut_timeline::Crop::NONE,
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: None,
                        luma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
            ],
            MasterLook {
                bars,
                ..MasterLook::default()
            },
        )
        .expect("composite");

    read_back(device, queue, compositor.target())
}

/// The pixel at a point given in 0–1 across the frame, as RGB.
fn at(pixels: &[u8], x: f32, y: f32) -> [u8; 3] {
    let px = ((x * OUTPUT.width as f32) as u32).min(OUTPUT.width - 1);
    let py = ((y * OUTPUT.height as f32) as u32).min(OUTPUT.height - 1);
    let i = ((py * OUTPUT.width + px) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) < 40)
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

const WHITE: [u8; 3] = [255, 255, 255];

/// A square frame cut to 2:1 loses a quarter of its height top and bottom.
#[test]
fn bars_cover_the_top_and_bottom() {
    let (device, queue) = gpu_or_skip!();
    let pixels = render(&device, &queue, 2.0);
    for (x, y) in [(0.5, 0.05), (0.1, 0.2), (0.9, 0.8), (0.5, 0.95)] {
        assert!(near(at(&pixels, x, y), [0, 0, 0]), "no bar at {x},{y}");
    }
    for (x, y) in [(0.5, 0.3), (0.5, 0.5), (0.1, 0.7)] {
        assert!(near(at(&pixels, x, y), WHITE), "covered at {x},{y}");
    }
}

#[test]
fn no_bars_leave_the_frame_alone() {
    let (device, queue) = gpu_or_skip!();
    let pixels = render(&device, &queue, 0.0);
    for (x, y) in [(0.5, 0.05), (0.5, 0.5), (0.5, 0.95)] {
        assert!(near(at(&pixels, x, y), WHITE), "at {x},{y}");
    }
}
