//! The secondary on the GPU: the picked colour moves and every other colour
//! stays, read back from a rendered frame so it is the shader itself that is
//! answering (`bettercut_timeline::HslSecondary`).
//!
//! Skips itself when no adapter can be found, like `blend_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{
    BlendMode, ClipLook, ColorAdjust, HslSecondary, MasterLook, Resolution, Transform,
};

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
                label: Some("secondary test"),
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
    width: 32,
    height: 32,
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

/// A flat `colour` drawn through `pick`, read back as one pixel.
fn graded(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    colour: [u8; 4],
    pick: HslSecondary,
) -> [u8; 3] {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");

    let frame = flat(colour);
    let layer = Layer {
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
            smooth_skin: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform::default(),
            opacity: 1.0,
            color: ColorAdjust {
                secondary: pick,
                ..ColorAdjust::default()
            },
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

    let pixels = read_back(device, queue, compositor.target());
    let middle = ((OUTPUT.height / 2 * OUTPUT.width + OUTPUT.width / 2) * 4) as usize;
    [pixels[middle], pixels[middle + 1], pixels[middle + 2]]
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

const RED: [u8; 4] = [200, 40, 40, 255];
const BLUE: [u8; 4] = [40, 40, 200, 255];
const GREY: [u8; 4] = [120, 120, 120, 255];

/// A pick on red, turned a third of the circle: the reds go green.
fn reds_to_green() -> HslSecondary {
    HslSecondary {
        hue: 0.0,
        width: 0.08,
        // A third of a turn is 0.667 of the half turn the shift spans.
        hue_shift: 0.667,
        saturation: 0.0,
        luminance: 0.0,
    }
}

/// A pick that does nothing changes nothing, to the byte, wherever it sits.
#[test]
fn a_pick_with_no_shift_leaves_the_picture_alone() {
    let (device, queue) = gpu_or_skip!();
    let mut picked = HslSecondary::IDENTITY;
    picked.hue = 0.0;
    picked.width = HslSecondary::MAX_WIDTH;
    for colour in [RED, BLUE, GREY] {
        let out = graded(&device, &queue, colour, picked);
        assert_eq!(out, [colour[0], colour[1], colour[2]], "{colour:?} moved");
    }
}

/// The whole point: the picked colour moves and the others do not.
#[test]
fn only_the_picked_colour_moves() {
    let (device, queue) = gpu_or_skip!();
    let red = graded(&device, &queue, RED, reds_to_green());
    assert!(
        red[1] > red[0] + 60 && red[1] > red[2] + 60,
        "the reds did not turn green: {red:?}"
    );
    let blue = graded(&device, &queue, BLUE, reds_to_green());
    assert_eq!(
        blue,
        [BLUE[0], BLUE[1], BLUE[2]],
        "the blues moved with the reds"
    );
}

/// Grey has no hue: it is never picked, whatever the pick and however wide.
#[test]
fn grey_is_never_picked() {
    let (device, queue) = gpu_or_skip!();
    let mut every_hue = reds_to_green();
    every_hue.width = HslSecondary::MAX_WIDTH;
    every_hue.luminance = 1.0;
    let grey = graded(&device, &queue, GREY, every_hue);
    assert_eq!(
        grey,
        [GREY[0], GREY[1], GREY[2]],
        "grey was picked: {grey:?}"
    );
}

/// Saturation and brightness shifts do what they say on the picked colour,
/// and only there.
#[test]
fn saturation_and_brightness_move_the_pick() {
    let (device, queue) = gpu_or_skip!();
    let duller = HslSecondary {
        saturation: -1.0,
        ..reds_to_green()
    };
    let out = graded(
        &device,
        &queue,
        RED,
        HslSecondary {
            hue_shift: 0.0,
            ..duller
        },
    );
    assert!(
        out[0] - out[1] < (RED[0] - RED[1]) / 2,
        "the reds were not made duller: {out:?}"
    );
    let brighter = HslSecondary {
        hue_shift: 0.0,
        luminance: 0.8,
        ..reds_to_green()
    };
    let out = graded(&device, &queue, RED, brighter);
    assert!(
        out[0] > RED[0] + 20,
        "the reds were not made brighter: {out:?}"
    );
    let blue = graded(&device, &queue, BLUE, brighter);
    assert_eq!(
        blue,
        [BLUE[0], BLUE[1], BLUE[2]],
        "brightness leaked onto the blues"
    );
}
