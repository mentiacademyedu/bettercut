//! §45's corner pin, as the GPU draws it: the picture's corners moved one at a
//! time, so it can be set into a screen filmed at an angle.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{
    ClipLook, ColorAdjust, CornerPin, Crop, MasterLook, Resolution, Transform,
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
                label: Some("corner pin test"),
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

fn white() -> VideoFrame {
    let (width, height) = (16, 16);
    VideoFrame {
        timestamp: bettercut_foundation::MediaTime::ZERO,
        width,
        height,
        color: ColorMetadata::srgb(),
        storage: FrameStorage::System {
            data: vec![255_u8; (width * height * 4) as usize],
            stride: width * 4,
        },
    }
}

fn look(corner_pin: CornerPin) -> ClipLook {
    ClipLook {
        corner_pin,
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
        crop: Crop::NONE,
        transform: Transform {
            scale: bettercut_timeline::Vec2::new(0.5, 0.5),
            ..Transform::default()
        },
        opacity: 1.0,
        color: ColorAdjust::default(),
        blur: 0.0,
        chroma_key: None,
        luma_key: None,
        mask: None,
        blend: bettercut_timeline::BlendMode::Normal,
    }
}

fn compositor(device: &wgpu::Device, queue: &wgpu::Queue) -> Compositor {
    Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor")
}

/// The defining claim: a corner dragged outwards covers frame the unpinned
/// picture did not, and the picture is no longer a rectangle.
#[test]
fn a_dragged_corner_reaches_where_the_rectangle_did_not() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let picture = white();

    // Half-size and centred: it covers the middle quarter, so the corners of
    // the frame are background.
    compositor
        .composite(
            &[Layer {
                frame: &picture,
                look: look(CornerPin::NONE),
            }],
            MasterLook::default(),
        )
        .expect("composite");
    let corner = compositor.read_pixel(6, 6).expect("in frame");
    assert!(
        corner[0] < 0.5,
        "the plain picture already covers the corner"
    );

    // The top-left corner dragged out to the frame's own corner.
    let pinned = CornerPin::NONE.with_corner(0, [-0.25, -0.25]);
    compositor
        .composite(
            &[Layer {
                frame: &picture,
                look: look(pinned),
            }],
            MasterLook::default(),
        )
        .expect("composite");
    let now = compositor.read_pixel(6, 6).expect("in frame");
    assert!(
        now[0] > 0.5,
        "the dragged corner did not reach the frame's corner: {now:?}"
    );
    // And the opposite corner is where it always was.
    let far = compositor.read_pixel(58, 58).expect("in frame");
    assert!(far[0] < 0.5, "the far corner moved too: {far:?}");
}

/// A pin of nothing is the picture exactly as it was — the path every ordinary
/// layer takes.
#[test]
fn a_pin_of_nothing_changes_nothing() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let picture = white();

    compositor
        .composite(
            &[Layer {
                frame: &picture,
                look: look(CornerPin::NONE),
            }],
            MasterLook::default(),
        )
        .expect("composite");
    let plain: Vec<[f32; 3]> = (0..8)
        .map(|n| compositor.read_pixel(n * 8, 32).expect("in frame"))
        .collect();

    compositor
        .composite(
            &[Layer {
                frame: &picture,
                look: look(CornerPin::default()),
            }],
            MasterLook::default(),
        )
        .expect("composite");
    for (n, before) in plain.iter().enumerate() {
        let after = compositor.read_pixel(n as u32 * 8, 32).expect("in frame");
        assert!(
            (after[0] - before[0]).abs() < 0.02,
            "column {n} changed: {before:?} then {after:?}"
        );
    }
}
