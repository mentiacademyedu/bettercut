//! Picture in picture, drawn: the inset lands in its corner, one margin in,
//! over the shot beneath it.
//!
//! The corner arithmetic is checked on its own in `editor-core`; this is the
//! same numbers through the real compositor, so a disagreement about which way
//! "down" is, or what a position is measured in, shows as an inset in the
//! wrong corner.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::pip::inset_frame;
use bettercut_editor_core::timeline::{
    BlendMode, ClipLook, ColorAdjust, Crop, MasterLook, Resolution, Transform, Vec2,
};
use bettercut_editor_core::{PipCorner, PipSize};
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
                label: Some("picture in picture test"),
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

fn look(scale: f32, position: [f32; 2]) -> ClipLook {
    ClipLook {
        crop: Crop::NONE,
        transform: Transform {
            position: Vec2::new(position[0], position[1]),
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
        reflection: bettercut_editor_core::timeline::Reflection::None,
        chroma_key: None,
        mask: None,
        blend: BlendMode::Normal,
    }
}

#[test]
fn a_small_inset_lands_in_the_top_right_corner() {
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
    let (scale, position) = inset_frame(
        16.0 / 9.0,
        output_aspect,
        PipCorner::TopRight,
        PipSize::Small,
    );

    let layers = [
        Layer {
            frame: &red,
            look: look(1.0, [0.0, 0.0]),
        },
        Layer {
            frame: &blue,
            look: look(scale, position),
        },
    ];
    compositor
        .composite(&layers, MasterLook::default())
        .unwrap();

    // 192 x 108: the inset is 48 x 27, about 4 px in from the top and right.
    let is_blue = |x, y| {
        let [r, _, b] = compositor.read_pixel(x, y).unwrap();
        b > 0.6 && r < 0.3
    };
    let is_red = |x, y| {
        let [r, _, b] = compositor.read_pixel(x, y).unwrap();
        r > 0.6 && b < 0.3
    };
    assert!(is_blue(164, 17), "the inset is not in the top right");
    assert!(
        is_blue(142, 6) && is_blue(186, 30),
        "the inset is smaller than it should be"
    );
    assert!(is_red(190, 17), "no margin on the right");
    assert!(is_red(164, 2), "no margin at the top");
    assert!(
        is_red(164, 34) && is_red(136, 17),
        "the inset is bigger than it should be"
    );
    assert!(
        is_red(96, 54) && is_red(20, 90),
        "the shot beneath is covered"
    );
}
