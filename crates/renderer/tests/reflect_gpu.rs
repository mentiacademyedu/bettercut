//! Reflections, drawn (`crate::reflect`), held to their reference.
//!
//! `Reflection::source_uv` says where each texel reads from; the shader is a
//! transcription of it. This paints a picture whose colour *is* its position —
//! red across, green down — so a pixel's colour says which source position the
//! shader actually read, and that is compared with what the reference says
//! it should have read.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{
    ClipLook, ColorAdjust, Crop, MasterLook, Reflection, Resolution, Transform,
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
                label: Some("reflect test"),
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

const SIZE: Resolution = Resolution {
    width: 256,
    height: 112,
};

fn frame(pixel: impl Fn(u32, u32) -> [u8; 3]) -> VideoFrame {
    let (w, h) = (SIZE.width, SIZE.height);
    let mut data = vec![0_u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = pixel(x, y);
            let at = ((y * w + x) * 4) as usize;
            data[at..at + 4].copy_from_slice(&[r, g, b, 255]);
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

/// sRGB-encoded 0..1 back to the byte it was written from, 0..1 — what
/// `read_pixel` returns is encoded, so this compares like with like.
fn position_picture() -> VideoFrame {
    frame(|x, y| {
        [
            ((x as f32 + 0.5) / SIZE.width as f32 * 255.0).round() as u8,
            ((y as f32 + 0.5) / SIZE.height as f32 * 255.0).round() as u8,
            0,
        ]
    })
}

fn render(
    compositor: &mut Compositor,
    frame: &VideoFrame,
    reflection: Reflection,
    pixels: &[(u32, u32)],
) -> Vec<[f32; 3]> {
    compositor
        .composite(
            &[Layer {
                frame,
                look: ClipLook {
                    corner_pin: Default::default(),
                    old_film: 0.0,
                    glow: 0.0,
                    shadow: Default::default(),
                    border: Default::default(),
                    crop: Crop::NONE,
                    transform: Transform::default(),
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    sharpen: 0.0,
                    lut: None,
                    rgb_split: 0.0,
                    glitch: 0.0,
                    pixelate: 0.0,
                    zoom_blur: 0.0,
                    vignette: 0.0,
                    reflection,
                    chroma_key: None,
                    mask: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                },
            }],
            MasterLook::default(),
        )
        .expect("composite");
    pixels
        .iter()
        .map(|&(x, y)| compositor.read_pixel(x, y).expect("pixel"))
        .collect()
}

/// Points spread over the frame, kept a few pixels off every fold line and
/// wedge edge, where a bilinear read legitimately blends both sides.
fn sample_points() -> Vec<(u32, u32)> {
    let mut points = Vec::new();
    for j in 0..6 {
        for i in 0..9 {
            points.push((13 + i * 29, 7 + j * 19));
        }
    }
    points
}

#[test]
fn every_reflection_reads_where_the_reference_says() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor =
        Compositor::new(device, queue, RenderConfig::export_to_texture(SIZE)).unwrap();
    let picture = position_picture();
    let aspect = SIZE.width as f32 / SIZE.height as f32;
    let points = sample_points();

    for kind in Reflection::ALL {
        let drawn = render(&mut compositor, &picture, kind, &points);
        let mut worst = 0.0_f32;
        let mut checked = 0;
        for (&(x, y), [r, g, _]) in points.iter().zip(&drawn) {
            let uv = [
                (x as f32 + 0.5) / SIZE.width as f32,
                (y as f32 + 0.5) / SIZE.height as f32,
            ];
            let [su, sv] = kind.source_uv(uv, aspect);
            // Near a fold or a wedge edge the neighbouring reads differ, and a
            // bilinear sample mixes them: skip points whose reference moves
            // sharply within a pixel.
            let step = [1.0 / SIZE.width as f32, 1.0 / SIZE.height as f32];
            let next = kind.source_uv([uv[0] + step[0], uv[1] + step[1]], aspect);
            if (next[0] - su).abs() > 3.0 * step[0] || (next[1] - sv).abs() > 3.0 * step[1] {
                continue;
            }
            checked += 1;
            let error = (r - su).abs().max((g - sv).abs());
            worst = worst.max(error);
        }
        assert!(checked > 30, "{kind:?}: too few points checked ({checked})");
        assert!(
            worst < 0.02,
            "{kind:?}: the shader read {worst} away from the reference"
        );
    }
}

/// Each reflection really changes the picture — a pass that silently did
/// nothing would match the reference only for `None`.
#[test]
fn a_reflection_changes_the_picture_and_none_does_not() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor =
        Compositor::new(device, queue, RenderConfig::export_to_texture(SIZE)).unwrap();
    let picture = position_picture();
    // Right of centre and near the bottom, steeply below the middle: outside
    // the kaleidoscope's first wedge (which it reads as it is) and on the
    // mirrored side of every fold.
    let point = [(150, 105)];
    let plain = render(&mut compositor, &picture, Reflection::None, &point)[0];
    assert!(plain[0] > 0.55 && plain[1] > 0.9, "{plain:?}");
    for kind in [
        Reflection::LeftRight,
        Reflection::TopBottom,
        Reflection::FourWay,
        Reflection::Kaleidoscope,
    ] {
        let drawn = render(&mut compositor, &picture, kind, &point)[0];
        assert!(
            (drawn[0] - plain[0]).abs() > 0.1 || (drawn[1] - plain[1]).abs() > 0.1,
            "{kind:?} left the picture as it was: {drawn:?}"
        );
    }
}
