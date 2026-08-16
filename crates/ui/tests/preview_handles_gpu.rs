//! The preview's drag handles must sit on the picture (§41, §54).
//!
//! `preview_overlay::layer_box` works out where a clip lands in the frame by
//! inverting the matrix `layer_uniform` builds. That derivation is exactly the
//! kind that is plausible and wrong — a sign flip or a factor of two puts the
//! handles somewhere the picture is not, and nothing else in the program would
//! notice.
//!
//! So this composites a real frame through the real renderer and reads back
//! where the picture actually is, then compares. The unit tests beside
//! `layer_box` check its arithmetic; this checks the arithmetic is the *right*
//! arithmetic.
//!
//! Skips itself when no adapter is available, for the §52.1 reasons given in
//! `renderer/tests/blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::timeline::{ColorAdjust, MasterLook, Resolution, Transform, Vec2};
use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_ui::preview_overlay::layer_box;

/// Output size. Large enough that a one-texel disagreement is well under the
/// tolerance, small enough to composite in milliseconds.
const OUT_W: u32 = 320;
const OUT_H: u32 = 320;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("preview handle test"),
        ..Default::default()
    }))
    .ok()
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

/// A solid white frame of the given size, opaque.
///
/// Solid on purpose: the test looks for where white appears against the
/// compositor's black canvas, so the edges of the white area *are* the edges of
/// the picture.
fn white(width: u32, height: u32) -> VideoFrame {
    let stride = width * 4;
    VideoFrame {
        timestamp: bettercut_editor_core::foundation::MediaTime::ZERO,
        width,
        height,
        color: ColorMetadata::bt709_limited_8bit(),
        storage: FrameStorage::System {
            data: vec![255; (stride * height) as usize],
            stride,
        },
    }
}

/// Composite one layer and report where the white lands, in 0..1 frame units.
fn rendered_box(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: (u32, u32),
    transform: Transform,
) -> egui::Rect {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(Resolution::new(OUT_W, OUT_H)),
    )
    .expect("compositor");

    let frame = white(source.0, source.1);
    compositor
        .composite(
            &[Layer {
                frame: &frame,
                transform,
                opacity: 1.0,
                color: ColorAdjust::default(),
                blur: 0.0,
            }],
            MasterLook::default(),
        )
        .expect("composite");

    let pixels = read_back(device, queue, compositor.target());
    bounds_of_white(&pixels)
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let padded = (OUT_W * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(OUT_H),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("readback encoder"),
    });
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
                rows_per_image: Some(OUT_H),
            },
        },
        wgpu::Extent3d {
            width: OUT_W,
            height: OUT_H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");

    let mapped = slice.get_mapped_range().expect("mapped");
    let mut out = Vec::with_capacity((OUT_W * OUT_H) as usize);
    for y in 0..OUT_H {
        let row = (y * padded) as usize;
        for x in 0..OUT_W {
            out.push(mapped[row + (x * 4) as usize]);
        }
    }
    drop(mapped);
    buffer.unmap();
    out
}

/// The bounding box of everything bright, in 0..1 frame units.
///
/// Half-intensity is the threshold: the edge texel of a scaled picture is
/// blended with the black behind it, so anything brighter than mid-grey is
/// inside the picture and anything darker is outside.
fn bounds_of_white(pixels: &[u8]) -> egui::Rect {
    let (mut min_x, mut min_y) = (OUT_W, OUT_H);
    let (mut max_x, mut max_y) = (0_u32, 0_u32);

    for y in 0..OUT_H {
        for x in 0..OUT_W {
            if pixels[(y * OUT_W + x) as usize] > 128 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    assert!(min_x <= max_x, "nothing was drawn");
    egui::Rect::from_min_max(
        egui::pos2(min_x as f32 / OUT_W as f32, min_y as f32 / OUT_H as f32),
        // +1 because the last lit texel's far edge is one texel further on.
        egui::pos2(
            (max_x + 1) as f32 / OUT_W as f32,
            (max_y + 1) as f32 / OUT_H as f32,
        ),
    )
}

fn transform(px: f32, py: f32, sx: f32, sy: f32) -> Transform {
    Transform {
        position: Vec2::new(px, py),
        scale: Vec2::new(sx, sy),
        ..Transform::default()
    }
}

/// Every case together: the handles must land on the picture for square,
/// letterboxed and pillarboxed sources, moved and scaled.
#[test]
fn the_handle_box_matches_where_the_picture_is_drawn() {
    let (device, queue) = gpu_or_skip!();

    // Two texels of tolerance. The renderer resolves edges to a texel, and the
    // readback rounds to one; anything larger than this is a real disagreement.
    let tolerance = 2.0 / OUT_W as f32;

    let cases: &[(&str, (u32, u32), Transform)] = &[
        (
            "square, untouched",
            (256, 256),
            transform(0.0, 0.0, 1.0, 1.0),
        ),
        (
            "square, half size",
            (256, 256),
            transform(0.0, 0.0, 0.5, 0.5),
        ),
        (
            "square, moved right and up",
            (256, 256),
            transform(0.2, -0.15, 0.5, 0.5),
        ),
        // Wide source in a square frame: letterboxed, so the box is short.
        (
            "16:9 letterboxed",
            (320, 180),
            transform(0.0, 0.0, 1.0, 1.0),
        ),
        (
            "16:9 letterboxed, moved down",
            (320, 180),
            transform(0.0, 0.25, 1.0, 1.0),
        ),
        // Tall source in a square frame: pillarboxed, so the box is narrow.
        (
            "9:16 pillarboxed",
            (180, 320),
            transform(0.0, 0.0, 1.0, 1.0),
        ),
        (
            "9:16 pillarboxed, scaled down and moved",
            (180, 320),
            transform(-0.2, 0.1, 0.6, 0.6),
        ),
    ];

    for (name, source, transform) in cases {
        let source_aspect = source.0 as f32 / source.1 as f32;
        let output_aspect = OUT_W as f32 / OUT_H as f32;

        let predicted = layer_box(source_aspect, output_aspect, *transform);
        let actual = rendered_box(&device, &queue, *source, *transform);

        for (edge, a, b) in [
            ("left", predicted.left(), actual.left()),
            ("right", predicted.right(), actual.right()),
            ("top", predicted.top(), actual.top()),
            ("bottom", predicted.bottom(), actual.bottom()),
        ] {
            // An edge outside the frame cannot be measured — the renderer has
            // nowhere to draw it and the readback stops at the frame boundary.
            // The prediction is still right, and being right is what puts the
            // handle correctly off-screen; what is checked there is that the
            // picture really does run all the way to the edge it was clipped at.
            if a < 0.0 {
                assert!(
                    b <= tolerance,
                    "{name}: the {edge} edge is predicted off-frame at {a:.4}, so \
                     the picture should reach the frame edge, but it starts at {b:.4}"
                );
                continue;
            }
            if a > 1.0 {
                assert!(
                    b >= 1.0 - tolerance,
                    "{name}: the {edge} edge is predicted off-frame at {a:.4}, so \
                     the picture should reach the frame edge, but it stops at {b:.4}"
                );
                continue;
            }

            assert!(
                (a - b).abs() <= tolerance,
                "{name}: the handles put the {edge} edge at {a:.4} and the \
                 renderer drew it at {b:.4}"
            );
        }
    }
}

/// Proof the comparison can fail: a box computed for the wrong source shape
/// must not match. Without this, a `layer_box` that ignored its arguments and
/// returned the whole frame would pass the test above on the square cases.
#[test]
fn the_check_notices_a_wrong_box() {
    let (device, queue) = gpu_or_skip!();

    // Render a wide source, but predict as if it were square.
    let actual = rendered_box(&device, &queue, (320, 180), transform(0.0, 0.0, 1.0, 1.0));
    let wrong = layer_box(1.0, 1.0, transform(0.0, 0.0, 1.0, 1.0));

    assert!(
        (wrong.height() - actual.height()).abs() > 0.1,
        "a letterboxed picture and a full-frame one measured the same, so the \
         comparison proves nothing"
    );
}
