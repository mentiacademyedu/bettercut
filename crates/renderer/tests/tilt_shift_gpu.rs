//! Tilt-shift on the GPU: with a band, a bright line in the band stays a
//! line while one outside it spreads; without a band both spread. Read back
//! from a rendered frame. Skips without an adapter.

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
                label: Some("tilt-shift test"),
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
    height: 128,
};

/// Black, with a one-pixel white line at a quarter of the height and one
/// at half.
fn lined() -> VideoFrame {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let mut data = vec![0_u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let at = ((y * w + x) * 4) as usize;
            let on = y == h / 4 || y == h / 2;
            data[at..at + 4].copy_from_slice(
                &[if on { 255 } else { 0 }; 3]
                    .iter()
                    .chain([&255u8])
                    .copied()
                    .collect::<Vec<u8>>(),
            );
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

fn blurred(device: &wgpu::Device, queue: &wgpu::Queue, band: f32) -> Vec<u8> {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");
    let frame = lined();
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
            posterise: 0.0,
            tilt_band: band,
            tilt_centre: 0.5,
            blur: 60.0,
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

/// The red channel down the middle column.
fn column(pixels: &[u8]) -> Vec<u8> {
    let x = OUTPUT.width / 2;
    (0..OUTPUT.height)
        .map(|y| pixels[((y * OUTPUT.width + x) * 4) as usize])
        .collect()
}

#[test]
fn the_band_stays_sharp_while_the_rest_blurs() {
    let (device, queue) = gpu_or_skip!();
    let (quarter, half) = (OUTPUT.height as usize / 4, OUTPUT.height as usize / 2);

    let everywhere = column(&blurred(&device, &queue, 0.0));
    assert!(
        everywhere[half] < 200,
        "no band: the middle line did not blur: {}",
        everywhere[half]
    );
    assert!(
        everywhere[quarter] < 200,
        "no band: the upper line did not blur"
    );

    let banded = column(&blurred(&device, &queue, 0.2));
    assert!(
        banded[half] > 240,
        "the line in the band blurred: {}",
        banded[half]
    );
    assert!(
        banded[half - 1] < 60 && banded[half + 1] < 60,
        "the band's line spread: {:?}",
        &banded[half - 2..=half + 2]
    );
    assert!(
        banded[quarter] < 200,
        "the line outside the band stayed sharp: {}",
        banded[quarter]
    );
}
