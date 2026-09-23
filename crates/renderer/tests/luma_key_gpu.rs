//! The luma key on a real device: a frame half dark grey, half white, keyed
//! by brightness — the dark half goes (or the bright one), the other stays,
//! measured in pixels on an opaque backdrop. Skips without an adapter.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{
    BlendMode, ClipLook, ColorAdjust, LumaKey, MasterLook, Resolution, Transform,
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
                label: Some("luma key test"),
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
    height: 8,
};

/// Dark grey on the left, white on the right.
fn halves() -> VideoFrame {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..h {
        for x in 0..w {
            let v = if x < w / 2 { 40 } else { 255 };
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

fn keyed(device: &wgpu::Device, queue: &wgpu::Queue, key: Option<LumaKey>) -> Vec<u8> {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");
    let frame = halves();
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
            blur: 0.0,
            chroma_key: None,
            luma_key: key,
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

fn red_at(pixels: &[u8], x: u32) -> u8 {
    let y = OUTPUT.height / 2;
    pixels[((y * OUTPUT.width + x) * 4) as usize]
}

/// Dropping the dark leaves the backdrop (black) on the left and the white
/// on the right; dropping the bright does the opposite; no key keeps both.
#[test]
fn the_dark_or_the_bright_side_goes() {
    let (device, queue) = gpu_or_skip!();
    let (left, right) = (OUTPUT.width / 4, OUTPUT.width * 3 / 4);

    let plain = keyed(&device, &queue, None);
    assert!(
        red_at(&plain, left) > 20 && red_at(&plain, left) < 90,
        "{}",
        red_at(&plain, left)
    );
    assert!(red_at(&plain, right) > 240);

    let drop_dark = keyed(&device, &queue, Some(LumaKey::default()));
    assert!(
        red_at(&drop_dark, left) < 8,
        "the dark side stayed: {}",
        red_at(&drop_dark, left)
    );
    assert!(red_at(&drop_dark, right) > 240, "the bright side went");

    let drop_bright = keyed(
        &device,
        &queue,
        Some(LumaKey {
            threshold: 0.7,
            softness: 0.05,
            keep_bright: false,
        }),
    );
    assert!(
        red_at(&drop_bright, right) < 8,
        "the bright side stayed: {}",
        red_at(&drop_bright, right)
    );
    assert!(red_at(&drop_bright, left) > 20, "the dark side went");
}
