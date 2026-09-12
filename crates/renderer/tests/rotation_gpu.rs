//! Rotation, on a real device, measured.
//!
//! A rotation preserves area and shape. The existing test checks that one point
//! at 90° on a square frame lands on the right side, which is true of a correct
//! rotation and also of several wrong ones — a shear, a mirror, a squash. These
//! render a solid square and measure what came out: how much of it there is,
//! and whether its extent is still square.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ColorAdjust, MasterLook, Resolution, Transform, Vec2};

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rotation test"),
        ..Default::default()
    }))
    .ok()
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

/// A solid white source, opaque.
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

/// What a rotated source looks like once composited: how many pixels it
/// covers, and the width and height of the box around them.
struct Coverage {
    pixels: usize,
    width: u32,
    height: u32,
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: Resolution,
    source: (u32, u32),
    degrees: f32,
) -> Coverage {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(output),
    )
    .expect("compositor");

    let frame = white(source.0, source.1);
    compositor
        .composite(
            &[Layer {
                frame: &frame,
                // Half size, so a 45° turn stays well inside the frame and
                // nothing is lost off the edge to confuse the count.
                transform: Transform {
                    scale: Vec2::new(0.4, 0.4),
                    rotation_degrees: degrees,
                    ..Transform::default()
                },
                opacity: 1.0,
                color: ColorAdjust::default(),
                blur: 0.0,
            }],
            MasterLook::default(),
        )
        .expect("composite");

    let pixels = read_back(device, queue, compositor.target(), output);
    let (w, h) = (output.width, output.height);
    let (mut count, mut left, mut top, mut right, mut bottom) = (0, u32::MAX, u32::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if pixels[i] > 128 {
                count += 1;
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    Coverage {
        pixels: count,
        width: if count > 0 { right - left + 1 } else { 0 },
        height: if count > 0 { bottom - top + 1 } else { 0 },
    }
}

fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    output: Resolution,
) -> Vec<u8> {
    let (w, h) = (output.width, output.height);
    let padded = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(h),
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

/// A rotation preserves area. A clip that shrinks, grows or vanishes as it
/// turns is being sheared, not rotated.
#[test]
fn rotating_a_clip_keeps_its_area() {
    let (device, queue) = gpu_or_skip!();
    let square = Resolution::new(512, 512);

    let upright = render(&device, &queue, square, (256, 256), 0.0);
    assert!(upright.pixels > 1000, "setup: nothing was drawn upright");

    for degrees in [30.0, 45.0, 60.0, 90.0] {
        let turned = render(&device, &queue, square, (256, 256), degrees);
        let ratio = turned.pixels as f64 / upright.pixels as f64;
        assert!(
            (0.95..=1.05).contains(&ratio),
            "at {degrees}° the clip covers {} pixels against {} upright — a \
             ratio of {ratio:.2}, which is not a rotation",
            turned.pixels,
            upright.pixels
        );
    }
}

/// A square turned 45° is a diamond with equal width and height. A shear makes
/// it lopsided; a squash from rotating in the frame's non-square clip space
/// makes it wider than it is tall.
#[test]
fn a_rotated_square_stays_square_on_a_wide_frame() {
    let (device, queue) = gpu_or_skip!();
    let wide = Resolution::new(640, 360);

    for degrees in [0.0, 45.0, 90.0] {
        let turned = render(&device, &queue, wide, (256, 256), degrees);
        assert!(turned.pixels > 500, "nothing drawn at {degrees}°");
        let aspect = turned.width as f64 / turned.height as f64;
        assert!(
            (0.93..=1.07).contains(&aspect),
            "at {degrees}° on a 16:9 frame a square came out {}×{} — an aspect \
             of {aspect:.2}, so the rotation is not happening in pixel space",
            turned.width,
            turned.height
        );
    }
}
