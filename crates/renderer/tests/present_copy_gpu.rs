//! The frame handed to egui on a GPU that cannot view one texture in two
//! formats (WebGL2): drawn into a plain texture from the sRGB target, it must
//! hold the target's bytes exactly as a second view of them would.
//!
//! Its own test binary: the switch (`use_present_copy`) is for the whole
//! process.
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
                label: Some("present copy test"),
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

/// Every byte value, row after row, in each channel: a ramp across the frame.
fn ramp() -> VideoFrame {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = ((y * w + x) % 256) as u8;
            data.extend_from_slice(&[v, 255 - v, v / 2, 255]);
        }
    }
    VideoFrame {
        timestamp: bettercut_foundation::MediaTime::ZERO,
        width: w,
        height: h,
        color: ColorMetadata::srgb(),
        storage: FrameStorage::System { data, stride: w * 4 },
    }
}

#[test]
fn the_copy_egui_is_handed_holds_the_targets_bytes() {
    let (device, queue) = gpu_or_skip!();
    bettercut_renderer::compositor::use_present_copy(true);
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");
    let frame = ramp();
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
            }],
            MasterLook::default(),
        )
        .expect("composite");
    assert!(
        !std::ptr::eq(compositor.present_texture(), compositor.target()),
        "the copy is a texture of its own"
    );
    let target = read_back(&device, &queue, compositor.target());
    let present = read_back(&device, &queue, compositor.present_texture());
    let worst = target
        .iter()
        .zip(&present)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(worst <= 1, "the copy is off by up to {worst} from the target");
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
