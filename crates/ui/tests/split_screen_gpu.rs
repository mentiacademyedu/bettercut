//! A split screen, drawn: each shot fills its half, and nothing of the
//! background shows between them.
//!
//! The layout arithmetic is checked on its own in `editor-core`; this is the
//! same numbers through the real compositor, so a disagreement between the
//! layout's idea of "position" and the renderer's would show here as a black
//! seam or a picture in the wrong half.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::SplitLayout;
use bettercut_editor_core::split_screen::frame_in_cell;
use bettercut_editor_core::timeline::{
    BlendMode, ClipLook, ColorAdjust, MasterLook, Resolution, Transform, Vec2,
};
use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};

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
                label: Some("split screen test"),
                ..Default::default()
            }))
            .ok()
        })
        .clone()
}

const OUT: Resolution = Resolution {
    width: 192,
    height: 108,
};

/// A landscape frame of one flat colour.
fn flat(rgb: [u8; 3]) -> VideoFrame {
    let (w, h) = (64_u32, 36_u32);
    let data: Vec<u8> = (0..w * h)
        .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
        .collect();
    VideoFrame {
        timestamp: bettercut_editor_core::foundation::MediaTime::ZERO,
        width: w,
        height: h,
        color: ColorMetadata::srgb(),
        storage: FrameStorage::System {
            data,
            stride: w * 4,
        },
    }
}

#[test]
fn two_shots_side_by_side_fill_their_halves() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUT),
    )
    .unwrap();
    let (red, blue) = (flat([220, 30, 30]), flat([30, 30, 220]));
    let output_aspect = OUT.width as f32 / OUT.height as f32;

    let layers: Vec<Layer<'_>> = [&red, &blue]
        .into_iter()
        .zip(SplitLayout::SideBySide.cells())
        .map(|(frame, cell)| {
            let (crop, scale, [x, y]) = frame_in_cell(16.0 / 9.0, output_aspect, *cell);
            Layer {
                frame,
                look: ClipLook {
                    corner_pin: Default::default(),
                    old_film: 0.0,
                    glow: 0.0,
                    shadow: Default::default(),
                    border: Default::default(),
                    crop,
                    transform: Transform {
                        position: Vec2::new(x, y),
                        scale: Vec2::new(scale, scale),
                        ..Transform::default()
                    },
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
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
                    reflection: bettercut_editor_core::timeline::Reflection::None,
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: BlendMode::Normal,
                },
            }
        })
        .collect();
    compositor
        .composite(&layers, MasterLook::default())
        .unwrap();

    let row = OUT.height / 2;
    for x in 0..OUT.width {
        let [r, _, b] = compositor.read_pixel(x, row).unwrap();
        // One pixel either side of the seam may be a blend of the two.
        if x.abs_diff(OUT.width / 2) <= 1 {
            continue;
        }
        if x < OUT.width / 2 {
            assert!(
                r > 0.6 && b < 0.3,
                "pixel {x} in the left half is not the first shot: r {r} b {b}"
            );
        } else {
            assert!(
                b > 0.6 && r < 0.3,
                "pixel {x} in the right half is not the second shot: r {r} b {b}"
            );
        }
    }
    for y in [0, OUT.height - 1] {
        let [r, _, b] = compositor.read_pixel(OUT.width / 4, y).unwrap();
        assert!(
            r > 0.6 || b > 0.6,
            "the top or bottom edge shows background at row {y}"
        );
    }
}
