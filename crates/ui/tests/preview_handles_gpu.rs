//! The preview's drag handles must sit on the picture (§54).
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

use bettercut_editor_core::timeline::{
    ClipLook, ColorAdjust, MasterLook, Resolution, Transform, Vec2,
};
use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_ui::preview_overlay::{clip_box, layer_box};

/// Output size. Large enough that a one-texel disagreement is well under the
/// tolerance, small enough to composite in milliseconds.
const OUT_W: u32 = 320;
const OUT_H: u32 = 320;

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
                label: Some("preview handle test"),
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
    rendered_cropped_box(
        device,
        queue,
        source,
        bettercut_editor_core::timeline::Crop::NONE,
        transform,
    )
}

/// Where the renderer actually draws a *cropped* clip.
fn rendered_cropped_box(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: (u32, u32),
    crop: bettercut_editor_core::timeline::Crop,
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
                    vignette: 0.0,
                    reflection: bettercut_editor_core::timeline::Reflection::None,
                    crop,
                    transform,
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: bettercut_editor_core::timeline::BlendMode::Normal,
                },
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

/// §22's crop reshapes the picture, so the handles have to reshape with it.
///
/// The case the test above never had. The preview computed its box from the
/// *source's* shape while the renderer fitted the *cropped* one, so every
/// cropped clip had its move, scale and rotate handles drawn around a shape the
/// picture no longer was — and nothing noticed, because no case here cropped.
#[test]
fn the_handle_box_follows_a_crop() {
    use bettercut_editor_core::timeline::Crop;

    let (device, queue) = gpu_or_skip!();
    let tolerance = 2.0 / OUT_W as f32;
    let output_aspect = OUT_W as f32 / OUT_H as f32;

    let cases: &[(&str, (u32, u32), Crop, Transform)] = &[
        // Square source cropped to a wide band: it should come out letterboxed.
        (
            "square cropped top and bottom",
            (256, 256),
            Crop {
                top: 0.3,
                bottom: 0.3,
                ..Crop::NONE
            },
            transform(0.0, 0.0, 1.0, 1.0),
        ),
        // Square cropped unevenly to a narrow column: pillarboxed.
        (
            "square cropped at the sides",
            (256, 256),
            Crop {
                left: 0.35,
                right: 0.1,
                ..Crop::NONE
            },
            transform(0.0, 0.0, 1.0, 1.0),
        ),
        // A wide source cropped back to square fills the square frame, where
        // uncropped it would letterbox — the case the old box got most wrong.
        (
            "16:9 cropped to square",
            (320, 180),
            Crop {
                left: 0.21875,
                right: 0.21875,
                ..Crop::NONE
            },
            transform(0.0, 0.0, 1.0, 1.0),
        ),
        (
            "cropped, then moved and scaled",
            (256, 256),
            Crop {
                top: 0.25,
                bottom: 0.25,
                ..Crop::NONE
            },
            transform(0.15, -0.1, 0.6, 0.6),
        ),
    ];

    for (name, source, crop, transform) in cases {
        let source_aspect = source.0 as f32 / source.1 as f32;
        let predicted = clip_box(source_aspect, output_aspect, *crop, *transform);
        let actual = rendered_cropped_box(&device, &queue, *source, *crop, *transform);

        for (edge, a, b) in [
            ("left", predicted.left(), actual.left()),
            ("right", predicted.right(), actual.right()),
            ("top", predicted.top(), actual.top()),
            ("bottom", predicted.bottom(), actual.bottom()),
        ] {
            assert!(
                (a - b).abs() <= tolerance,
                "{name}: the handles put the {edge} edge at {a:.4} and the \
                 renderer drew the cropped picture's edge at {b:.4}"
            );
        }
    }
}

/// §54's rotate handle: the turned box has to sit on the turned picture.
///
/// On a 16:9 frame, because that is where rotation used to go wrong — the
/// renderer rotated in clip space, which is not square, and sheared the picture
/// (it vanished entirely at 45°). The overlay rotates in screen pixels; this
/// checks the two now agree, corner by corner, against what the GPU drew.
#[test]
fn a_rotated_clips_corners_sit_on_its_picture() {
    use bettercut_ui::preview_overlay::{corners, to_canvas};

    let (device, queue) = gpu_or_skip!();
    let (width, height) = (480_u32, 270_u32);
    let output = Resolution::new(width, height);

    let transform = Transform {
        position: Vec2::new(0.08, -0.04),
        scale: Vec2::new(0.45, 0.45),
        rotation_degrees: 30.0,
        ..Transform::default()
    };
    // A 2:1 source, so a mix-up between its axes shows as well.
    let frame = white(200, 100);

    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(output),
    )
    .expect("compositor");
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
                    vignette: 0.0,
                    reflection: bettercut_editor_core::timeline::Reflection::None,
                    crop: bettercut_editor_core::timeline::Crop::NONE,
                    transform,
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: bettercut_editor_core::timeline::BlendMode::Normal,
                },
            }],
            MasterLook::default(),
        )
        .expect("composite");

    // Read back at this size rather than the file's shared constants.
    let texture = compositor.target();
    let padded = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rotated readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rotated readback"),
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
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
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

    // The four extremes of the white region — for a turned rectangle, those
    // are its four corners.
    let mut white_pixels = Vec::new();
    for y in 0..height {
        let row = (y * padded) as usize;
        for x in 0..width {
            if mapped[row + (x * 4) as usize] > 128 {
                white_pixels.push(egui::pos2(x as f32 + 0.5, y as f32 + 0.5));
            }
        }
    }
    drop(mapped);
    buffer.unmap();
    assert!(white_pixels.len() > 100, "the rotated clip was not drawn");

    let extreme = |key: fn(&egui::Pos2) -> f32, smallest: bool| {
        white_pixels
            .iter()
            .copied()
            .reduce(|a, b| {
                let pick_b = if smallest {
                    key(&b) < key(&a)
                } else {
                    key(&b) > key(&a)
                };
                if pick_b { b } else { a }
            })
            .expect("pixels")
    };
    let drawn = [
        extreme(|p| p.y, true),  // topmost
        extreme(|p| p.x, false), // rightmost
        extreme(|p| p.y, false), // bottommost
        extreme(|p| p.x, true),  // leftmost
    ];

    // Where the overlay puts the corners, on a canvas the size of the frame.
    let canvas =
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32));
    let box_on_canvas = to_canvas(
        layer_box(2.0, width as f32 / height as f32, transform),
        canvas,
    );
    let predicted = corners(box_on_canvas, transform.rotation_degrees);

    // Every drawn extreme has a predicted corner beside it.
    for point in drawn {
        let nearest = predicted
            .iter()
            .map(|corner| corner.distance(point))
            .fold(f32::MAX, f32::min);
        assert!(
            nearest <= 3.0,
            "the picture has a corner at {point:?} and the nearest handle is {nearest:.1} px \
             away; handles at {predicted:?}"
        );
    }
}
