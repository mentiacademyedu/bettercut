//! Masks, on a real device.
//!
//! A mask decides what of a layer exists at all, so it is measured the same way
//! the chroma key is: composite over a ground of another colour and look at
//! which one came through where.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{
    ClipLook, ColorAdjust, Mask, MaskShape, MasterLook, Resolution, Transform,
};

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
                label: Some("mask test"),
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

/// A white layer, masked, over a red ground.
///
/// Wherever the mask kept the layer the pixel is white; wherever it did not,
/// the red ground shows through.
fn render(device: &wgpu::Device, queue: &wgpu::Queue, mask: Option<Mask>) -> Vec<u8> {
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
                        sharpen: 0.0,
                        lut: None,
                        rgb_split: 0.0,
                        glitch: 0.0,
                        reflection: bettercut_timeline::Reflection::None,
                        crop: bettercut_timeline::Crop::NONE,
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
                Layer {
                    frame: &subject,

                    look: ClipLook {
                        sharpen: 0.0,
                        lut: None,
                        rgb_split: 0.0,
                        glitch: 0.0,
                        reflection: bettercut_timeline::Reflection::None,
                        crop: bettercut_timeline::Crop::NONE,
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: None,
                        mask,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
            ],
            MasterLook::default(),
        )
        .expect("composite");

    read_back(device, queue, compositor.target())
}

/// How white a pixel is, 0–255, at a point given in 0–1 across the frame.
fn at(pixels: &[u8], x: f32, y: f32) -> u8 {
    let px = ((x * OUTPUT.width as f32) as u32).min(OUTPUT.width - 1);
    let py = ((y * OUTPUT.height as f32) as u32).min(OUTPUT.height - 1);
    // Green is the telling channel: white has it, the red ground does not.
    pixels[((py * OUTPUT.width + px) * 4 + 1) as usize]
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
fn no_mask_keeps_the_whole_layer() {
    let (device, queue) = gpu_or_skip!();
    let pixels = render(&device, &queue, None);
    for (x, y) in [(0.1, 0.1), (0.5, 0.5), (0.9, 0.9)] {
        assert!(
            at(&pixels, x, y) > 200,
            "the layer was masked without a mask"
        );
    }
}

/// A rectangle keeps its inside and nothing else.
#[test]
fn a_rectangle_keeps_its_inside() {
    let (device, queue) = gpu_or_skip!();
    let pixels = render(
        &device,
        &queue,
        Some(Mask {
            shape: MaskShape::Rectangle,
            center: [0.5, 0.5],
            size: [0.25, 0.25],
            feather: 0.0,
            ..Mask::default()
        }),
    );

    assert!(at(&pixels, 0.5, 0.5) > 200, "the middle was removed");
    // Just inside each edge, and well outside.
    assert!(
        at(&pixels, 0.3, 0.5) > 200,
        "the left of the box was removed"
    );
    assert!(
        at(&pixels, 0.7, 0.5) > 200,
        "the right of the box was removed"
    );
    for (x, y) in [(0.05, 0.5), (0.95, 0.5), (0.5, 0.05), (0.5, 0.95)] {
        assert!(
            at(&pixels, x, y) < 60,
            "outside the box survived at {x},{y}"
        );
    }
    // The corners of the frame are outside a centred box on both axes.
    assert!(at(&pixels, 0.05, 0.05) < 60);
}

/// An ellipse is round: its corners are outside it where a rectangle's are in.
#[test]
fn an_ellipse_is_not_a_rectangle() {
    let (device, queue) = gpu_or_skip!();
    let mask = Mask {
        shape: MaskShape::Ellipse,
        center: [0.5, 0.5],
        size: [0.4, 0.4],
        feather: 0.0,
        ..Mask::default()
    };
    let round = render(&device, &queue, Some(mask));
    let square = render(
        &device,
        &queue,
        Some(Mask {
            shape: MaskShape::Rectangle,
            ..mask
        }),
    );

    // Straight out from the centre, both keep the pixel.
    assert!(at(&round, 0.85, 0.5) > 200, "the ellipse lost its own axis");
    assert!(at(&square, 0.85, 0.5) > 200);

    // Diagonally, the rectangle keeps it and the ellipse does not: that corner
    // is 0.35 * sqrt(2) from the centre, past a radius of 0.4.
    assert!(
        at(&square, 0.85, 0.85) > 200,
        "the rectangle lost its corner"
    );
    assert!(
        at(&round, 0.85, 0.85) < 60,
        "the ellipse kept a corner, so it is a rectangle"
    );
}

/// A linear mask keeps one side of a straight edge: what a split screen and a
/// reveal are made of.
#[test]
fn a_linear_mask_keeps_one_side() {
    let (device, queue) = gpu_or_skip!();
    let pixels = render(
        &device,
        &queue,
        Some(Mask {
            shape: MaskShape::Linear,
            center: [0.5, 0.5],
            feather: 0.0,
            ..Mask::default()
        }),
    );

    assert!(at(&pixels, 0.5, 0.2) > 200, "the top was removed");
    assert!(at(&pixels, 0.5, 0.8) < 60, "the bottom survived");
}

/// Turning it turns the edge with it.
#[test]
fn rotation_turns_a_linear_mask() {
    let (device, queue) = gpu_or_skip!();
    let pixels = render(
        &device,
        &queue,
        Some(Mask {
            shape: MaskShape::Linear,
            center: [0.5, 0.5],
            feather: 0.0,
            rotation_degrees: 90.0,
            ..Mask::default()
        }),
    );

    // A quarter turn puts the edge down the middle instead of across it.
    let (left, right) = (at(&pixels, 0.2, 0.5), at(&pixels, 0.8, 0.5));
    assert!(
        left.abs_diff(right) > 150,
        "the edge did not turn: {left} against {right}"
    );
}

/// Inverting keeps exactly what it kept before, and nothing that it did.
#[test]
fn inverting_swaps_what_is_kept() {
    let (device, queue) = gpu_or_skip!();
    let mask = Mask {
        shape: MaskShape::Ellipse,
        center: [0.5, 0.5],
        size: [0.3, 0.3],
        feather: 0.0,
        ..Mask::default()
    };
    let plain = render(&device, &queue, Some(mask));
    let inverted = render(
        &device,
        &queue,
        Some(Mask {
            invert: true,
            ..mask
        }),
    );

    assert!(at(&plain, 0.5, 0.5) > 200 && at(&inverted, 0.5, 0.5) < 60);
    assert!(at(&plain, 0.05, 0.05) < 60 && at(&inverted, 0.05, 0.05) > 200);
}

/// Feathering softens the edge without moving it: the middle and the outside
/// are unchanged, and the boundary is no longer a step.
#[test]
fn feathering_softens_the_edge_without_moving_it() {
    let (device, queue) = gpu_or_skip!();
    let hard = Mask {
        shape: MaskShape::Rectangle,
        center: [0.5, 0.5],
        size: [0.25, 0.25],
        feather: 0.0,
        ..Mask::default()
    };
    let soft = Mask {
        feather: 0.25,
        ..hard
    };

    let hard_pixels = render(&device, &queue, Some(hard));
    let soft_pixels = render(&device, &queue, Some(soft));

    // Dead centre and far outside: the same either way.
    assert!(at(&hard_pixels, 0.5, 0.5) > 200 && at(&soft_pixels, 0.5, 0.5) > 200);
    assert!(at(&hard_pixels, 0.95, 0.5) < 60 && at(&soft_pixels, 0.95, 0.5) < 60);

    // On the edge itself, the hard mask is one or the other and the soft one
    // is in between.
    let middling = at(&soft_pixels, 0.78, 0.5);
    assert!(
        (60..200).contains(&middling),
        "the feathered edge is still a step: {middling}"
    );
}
