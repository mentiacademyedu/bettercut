//! The colour wheels on the GPU: lift moves the darks and not the brights,
//! gain the brights and not the darks, gamma the middle — read back from a
//! rendered frame, so it is the shader itself that is answering
//! (`bettercut_timeline::ColorWheels`).
//!
//! Skips itself when no adapter can be found, like `blend_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{
    BlendMode, ClipLook, ColorAdjust, ColorWheels, MasterLook, Resolution, Transform,
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
                label: Some("wheels test"),
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

/// A flat `colour` drawn through `wheels`, read back as one pixel.
fn graded(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    colour: [u8; 4],
    wheels: ColorWheels,
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
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform::default(),
            opacity: 1.0,
            color: ColorAdjust {
                wheels,
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

const DARK: [u8; 4] = [30, 30, 30, 255];
const MID: [u8; 4] = [128, 128, 128, 255];
const LIGHT: [u8; 4] = [230, 230, 230, 255];

fn wheels(lift: [f32; 3], gamma: [f32; 3], gain: [f32; 3]) -> ColorWheels {
    ColorWheels { lift, gamma, gain }
}

/// Wheels at rest change nothing, to the byte: the grade is skipped, not
/// merely small.
#[test]
fn wheels_at_rest_leave_the_picture_alone() {
    let (device, queue) = gpu_or_skip!();
    for colour in [DARK, MID, LIGHT] {
        let out = graded(&device, &queue, colour, ColorWheels::IDENTITY);
        assert_eq!(
            out,
            [colour[0], colour[1], colour[2]],
            "{colour:?} moved at rest"
        );
    }
}

/// A red lift reddens the darks and leaves the brights alone: the shadows'
/// colour is not painted onto the highlights.
#[test]
fn lift_moves_the_darks_and_not_the_brights() {
    let (device, queue) = gpu_or_skip!();
    let red_lift = wheels([0.4, 0.0, 0.0], [0.0; 3], [0.0; 3]);

    let dark = graded(&device, &queue, DARK, red_lift);
    let light = graded(&device, &queue, LIGHT, red_lift);
    let dark_rise = i32::from(dark[0]) - i32::from(DARK[0]);
    let light_rise = i32::from(light[0]) - i32::from(LIGHT[0]);
    assert!(
        dark_rise > 20 && dark[1] <= DARK[1] + 2,
        "the darks were not lifted red: {dark:?}"
    );
    // The lift fades out towards white, so a light grey takes a little of it
    // and a dark one takes most: the shadow's colour is not painted on the
    // highlights.
    assert!(
        light_rise * 2 < dark_rise,
        "the brights took the shadow's colour: {light:?} rose {light_rise} against {dark_rise}"
    );
}

/// A blue gain brightens the brights far more than the darks: a multiply is
/// nothing on nothing.
#[test]
fn gain_moves_the_brights_more_than_the_darks() {
    let (device, queue) = gpu_or_skip!();
    let blue_gain = wheels([0.0; 3], [0.0; 3], [0.0, 0.0, 0.6]);

    let dark = graded(&device, &queue, DARK, blue_gain);
    let light = graded(&device, &queue, LIGHT, blue_gain);
    let dark_rise = i32::from(dark[2]) - i32::from(DARK[2]);
    let light_rise = i32::from(light[2]) - i32::from(LIGHT[2]);
    assert!(light_rise > 10, "the brights did not gain blue: {light:?}");
    assert!(
        dark_rise < light_rise,
        "the darks gained as much as the brights: {dark_rise} vs {light_rise}"
    );
    assert_eq!(light[0], LIGHT[0], "gain on blue touched red: {light:?}");
}

/// Gamma bends the middle: a positive gamma lifts mid-grey and a negative one
/// sinks it, and neither moves black or white.
#[test]
fn gamma_bends_the_middle_and_pins_the_ends() {
    let (device, queue) = gpu_or_skip!();
    let up = wheels([0.0; 3], [0.6; 3], [0.0; 3]);
    let down = wheels([0.0; 3], [-0.6; 3], [0.0; 3]);

    let mid_up = graded(&device, &queue, MID, up);
    let mid_down = graded(&device, &queue, MID, down);
    assert!(
        mid_up[1] > MID[1] + 15,
        "gamma up did not lift the middle: {mid_up:?}"
    );
    assert!(
        mid_down[1] < MID[1] - 15,
        "gamma down did not sink the middle: {mid_down:?}"
    );

    let black = graded(&device, &queue, [0, 0, 0, 255], up);
    let white = graded(&device, &queue, [255, 255, 255, 255], down);
    assert_eq!(black, [0, 0, 0], "gamma moved black");
    assert_eq!(white, [255, 255, 255], "gamma moved white");
}
