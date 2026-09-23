//! Lens correction on the GPU: the middle stays where it is, the edges are
//! pushed out or pulled in, and what a push uncovers is see-through — read
//! back from a rendered frame, so it is the shader itself that is answering
//! (`VideoClip::lens`).
//!
//! Skips itself when no adapter can be found, like `blend_gpu.rs`.

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
                label: Some("lens test"),
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

/// `frame` drawn through `lens`, read back whole (tight RGBA rows).
fn bent(device: &wgpu::Device, queue: &wgpu::Queue, frame: &VideoFrame, lens: f32) -> Vec<u8> {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");

    let layer = Layer {
        frame,
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
            lens,
            tilt_band: 0.0,
            tilt_centre: 0.5,
            posterise: 0.0,
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

/// A frame with a bright vertical stripe two pixels in from the left edge,
/// on a mid grey. The stripe is what moves; the middle is what must not.
fn striped() -> VideoFrame {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let mut data = [90_u8, 90, 90, 255].repeat((w * h) as usize);
    for y in 0..h {
        for x in 2..4 {
            let at = ((y * w + x) * 4) as usize;
            data[at..at + 3].copy_from_slice(&[240, 240, 240]);
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

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * OUTPUT.width + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

fn is_bright(p: [u8; 4]) -> bool {
    p[0] > 180
}

#[test]
fn no_lens_leaves_the_picture_alone() {
    let (device, queue) = gpu_or_skip!();
    let frame = striped();
    let plain = bent(&device, &queue, &frame, 0.0);
    let middle = OUTPUT.height / 2;
    assert!(
        is_bright(pixel(&plain, 2, middle)),
        "the stripe is not where it was drawn"
    );
    assert!(!is_bright(pixel(&plain, 8, middle)));
}

/// Pushing the edges out (correcting barrel) widens the window the picture
/// is read through: the stripe near the edge moves inward, the corners now
/// read past the picture and are see-through, and the very middle reads the
/// same pixel as before.
#[test]
fn pushing_out_moves_the_edge_inward_and_uncovers_the_corner() {
    let (device, queue) = gpu_or_skip!();
    let frame = striped();
    let middle = OUTPUT.height / 2;
    let plain = bent(&device, &queue, &frame, 0.0);
    let pushed = bent(&device, &queue, &frame, 1.0);

    let found = (0..OUTPUT.width).find(|x| is_bright(pixel(&pushed, *x, middle)));
    assert!(
        matches!(found, Some(x) if x > 2),
        "the stripe did not move inward under a push: {found:?}"
    );
    // The export target sits on an opaque backdrop, so "see-through" reads
    // as the backdrop (black) rather than as the picture's grey smeared out
    // to the corner — which is the difference that matters.
    let corner = pixel(&pushed, 0, 0);
    assert!(
        corner[0] < 40,
        "the uncovered corner shows the picture's edge, not the backdrop: {corner:?}"
    );
    assert_eq!(
        pixel(&pushed, OUTPUT.width / 2, middle),
        pixel(&plain, OUTPUT.width / 2, middle),
        "the middle moved"
    );
}

/// Pulling the edges in (barrel) narrows the window: the picture's own edge
/// is pushed out of the frame, so the stripe is gone, and nothing empties.
#[test]
fn pulling_in_pushes_the_edge_out_of_frame_and_keeps_the_frame_full() {
    let (device, queue) = gpu_or_skip!();
    let frame = striped();
    let middle = OUTPUT.height / 2;
    let pulled = bent(&device, &queue, &frame, -1.0);

    let found = (0..OUTPUT.width).find(|x| is_bright(pixel(&pulled, *x, middle)));
    assert!(
        found.is_none(),
        "the edge stayed in frame under a pull: {found:?}"
    );
    assert_eq!(pixel(&pulled, 0, 0)[3], 255, "a pull emptied a corner");
    assert_eq!(
        pixel(&pulled, 0, 0)[0],
        90,
        "the corner is not the grey it reads inward to"
    );
}
