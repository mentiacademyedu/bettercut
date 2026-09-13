//! §22's blend modes, on a real device.
//!
//! Each mode is a claim about what happens to the picture beneath: screen never
//! darkens, multiply never lightens, add only brightens. Those are measurable,
//! so they are measured — a blend state is four enum values in a pipeline
//! descriptor and nothing about reading it tells you it is right.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{BlendMode, ClipLook, ColorAdjust, MasterLook, Resolution, Transform};

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
                label: Some("blend test"),
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

/// `over` composited onto `under` with `mode`, read back as one pixel.
fn blended(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    under: [u8; 4],
    over: [u8; 4],
    mode: BlendMode,
    opacity: f32,
) -> [u8; 3] {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");

    let bottom = flat(under);
    let top = flat(over);
    fn layer(frame: &VideoFrame, mode: BlendMode, opacity: f32) -> Layer<'_> {
        Layer {
            frame,

            look: ClipLook {
                sharpen: 0.0,
                lut: None,
                rgb_split: 0.0,
                glitch: 0.0,
                reflection: bettercut_timeline::Reflection::None,
                crop: bettercut_timeline::Crop::NONE,
                transform: Transform::default(),
                opacity,
                color: ColorAdjust::default(),
                blur: 0.0,
                chroma_key: None,
                mask: None,
                blend: mode,
            },
        }
    }
    compositor
        .composite(
            &[
                layer(&bottom, BlendMode::Normal, 1.0),
                layer(&top, mode, opacity),
            ],
            MasterLook::default(),
        )
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

const MID_GREY: [u8; 4] = [128, 128, 128, 255];
const DARK: [u8; 4] = [40, 40, 40, 255];
const LIGHT: [u8; 4] = [200, 200, 200, 255];

/// Normal covers what is beneath it, whatever that was.
#[test]
fn normal_covers_what_is_under_it() {
    let (device, queue) = gpu_or_skip!();
    let out = blended(&device, &queue, MID_GREY, DARK, BlendMode::Normal, 1.0);
    assert!(
        out[0].abs_diff(DARK[0]) <= 2,
        "the layer did not cover: {out:?}"
    );
}

/// Screen never darkens: black in the layer disappears, which is how a light
/// leak or a glow is laid on.
#[test]
fn screen_never_darkens() {
    let (device, queue) = gpu_or_skip!();

    let lit = blended(&device, &queue, MID_GREY, DARK, BlendMode::Screen, 1.0);
    assert!(
        lit[0] >= MID_GREY[0],
        "screening a dark layer made it darker: {lit:?}"
    );

    // Black changes nothing at all — the whole point of shooting an overlay on
    // black.
    let black = blended(
        &device,
        &queue,
        MID_GREY,
        [0, 0, 0, 255],
        BlendMode::Screen,
        1.0,
    );
    assert!(
        black[0].abs_diff(MID_GREY[0]) <= 2,
        "black did not disappear under screen: {black:?}"
    );
}

/// Multiply never lightens, and white disappears.
#[test]
fn multiply_never_lightens() {
    let (device, queue) = gpu_or_skip!();

    let darkened = blended(&device, &queue, MID_GREY, LIGHT, BlendMode::Multiply, 1.0);
    assert!(
        darkened[0] <= MID_GREY[0] + 2,
        "multiplying made it lighter: {darkened:?}"
    );

    let white = blended(
        &device,
        &queue,
        MID_GREY,
        [255, 255, 255, 255],
        BlendMode::Multiply,
        1.0,
    );
    assert!(
        white[0].abs_diff(MID_GREY[0]) <= 2,
        "white did not disappear under multiply: {white:?}"
    );
}

/// Add is brighter than screen for the same layer, and clips sooner.
#[test]
fn add_is_the_brightest() {
    let (device, queue) = gpu_or_skip!();
    let screened = blended(&device, &queue, MID_GREY, DARK, BlendMode::Screen, 1.0);
    let added = blended(&device, &queue, MID_GREY, DARK, BlendMode::Add, 1.0);

    assert!(
        added[0] > screened[0],
        "add ({added:?}) is not brighter than screen ({screened:?})"
    );
}

/// Opacity still means opacity: a half-strength overlay screens half as much.
///
/// This is what the premultiplied shader output is *for* — with straight alpha
/// the blend factors that express screen have nowhere to apply the opacity, and
/// a 50% overlay would screen at full strength.
#[test]
fn opacity_scales_a_screened_layer() {
    let (device, queue) = gpu_or_skip!();
    let full = blended(&device, &queue, DARK, LIGHT, BlendMode::Screen, 1.0);
    let half = blended(&device, &queue, DARK, LIGHT, BlendMode::Screen, 0.5);

    assert!(
        half[0] < full[0],
        "a half-opacity overlay screened as hard as a full one: {half:?} against {full:?}"
    );
    assert!(half[0] > DARK[0], "it screened nothing at all: {half:?}");
}

/// Every mode leaves the picture opaque: §21a's output has no alpha to speak
/// of, and a multiply that punched a hole in it would show as black.
#[test]
fn no_mode_makes_the_picture_transparent() {
    let (device, queue) = gpu_or_skip!();
    for mode in BlendMode::ALL {
        let mut compositor = Compositor::new(
            device.clone(),
            queue.clone(),
            RenderConfig::export_to_texture(OUTPUT),
        )
        .expect("compositor");
        let bottom = flat(MID_GREY);
        let top = flat(DARK);
        compositor
            .composite(
                &[
                    Layer {
                        frame: &bottom,

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
                            blend: BlendMode::Normal,
                        },
                    },
                    Layer {
                        frame: &top,

                        look: ClipLook {
                            sharpen: 0.0,
                            lut: None,
                            rgb_split: 0.0,
                            glitch: 0.0,
                            reflection: bettercut_timeline::Reflection::None,
                            crop: bettercut_timeline::Crop::NONE,
                            transform: Transform::default(),
                            opacity: 0.6,
                            color: ColorAdjust::default(),
                            blur: 0.0,
                            chroma_key: None,
                            mask: None,
                            blend: mode,
                        },
                    },
                ],
                MasterLook::default(),
            )
            .expect("composite");

        let pixels = read_back(&device, &queue, compositor.target());
        let middle = ((OUTPUT.height / 2 * OUTPUT.width + OUTPUT.width / 2) * 4) as usize;
        assert_eq!(
            pixels[middle + 3],
            255,
            "{} left the picture partly transparent",
            mode.label()
        );
    }
}
