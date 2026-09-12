//! A title, composited on a real device (§26.1).
//!
//! Everything else about text is checked without a GPU: the shaping, the
//! decorations, which layers a moment resolves to. None of that would notice
//! the one mistake that makes titles unusable — every layer is *fitted* to the
//! canvas, so a bitmap that happens to be 400 pixels wide gets blown up to fill
//! the frame, and the same words in a longer sentence come out smaller.
//!
//! So this composites a real rasterized title over a real background and reads
//! the pixels back: how much of the frame it covers, that it did not replace
//! the picture behind it, and that its own scale still works.
//!
//! Skips itself when no adapter is available, for the §52.1 reasons given in
//! `renderer/tests/blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::text::{Rgba, TextStyle};
use bettercut_editor_core::timeline::{
    ColorAdjust, MasterLook, Resolution, TextClip, Transform, Vec2, natural_size_transform,
};
use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_playback::TextFrames;
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};

const OUT_W: u32 = 640;
const OUT_H: u32 = 360;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("text composite test"),
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

/// A solid mid-blue background, opaque — so anything that is not blue in the
/// result came from the title.
fn background() -> VideoFrame {
    let stride = OUT_W * 4;
    let mut data = vec![0_u8; (stride * OUT_H) as usize];
    for pixel in data.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[0, 0, 200, 255]);
    }
    VideoFrame {
        timestamp: bettercut_editor_core::foundation::MediaTime::ZERO,
        width: OUT_W,
        height: OUT_H,
        color: ColorMetadata::srgb(),
        storage: FrameStorage::System { data, stride },
    }
}

/// A title, styled so its ink is unmistakable: pure red, no outline, no shadow.
fn title(text: &str, size: f32) -> TextClip {
    TextClip {
        style: TextStyle {
            size,
            color: Rgba::opaque(255, 0, 0),
            stroke: None,
            shadow: None,
            background: None,
            ..TextStyle::default()
        },
        ..TextClip::new(text, TimelineTime::ZERO).expect("valid")
    }
}

/// Composite the background with the title over it, and return the pixels.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    clip: &TextClip,
    transform: Transform,
) -> Vec<u8> {
    let mut titles = TextFrames::new();
    let text_frame = titles
        .frame_for(clip, None)
        .expect("the title rasterized to nothing");
    let back = background();

    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(Resolution::new(OUT_W, OUT_H)),
    )
    .expect("compositor");

    compositor
        .composite(
            &[
                Layer {
                    frame: &back,
                    transform: Transform::default(),
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                },
                Layer {
                    frame: &text_frame,
                    // The correction the preview and the export both apply.
                    transform: natural_size_transform(
                        transform,
                        text_frame.width,
                        text_frame.height,
                        OUT_W,
                        OUT_H,
                    ),
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                },
            ],
            MasterLook::default(),
        )
        .expect("composite");

    read_back(device, queue, compositor.target())
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
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");

    let mapped = slice.get_mapped_range().expect("mapped");
    let mut tight = Vec::with_capacity((OUT_W * OUT_H * 4) as usize);
    for row in 0..OUT_H {
        let start = (row * padded) as usize;
        tight.extend_from_slice(&mapped[start..start + (OUT_W * 4) as usize]);
    }
    drop(mapped);
    buffer.unmap();
    tight
}

/// Pixels where red clearly dominates — the title's ink.
fn ink_pixels(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 120 && p[2] < 120)
        .count()
}

#[test]
fn a_title_draws_over_the_picture_without_covering_it() {
    let (device, queue) = gpu_or_skip!();
    let pixels = render(&device, &queue, &title("Hello", 48.0), Transform::default());

    let ink = ink_pixels(&pixels);
    let total = (OUT_W * OUT_H) as usize;

    assert!(ink > 0, "the title never reached the frame");
    assert!(
        ink < total / 3,
        "the title covered {ink} of {total} pixels — it is being fitted to the \
         canvas rather than drawn at its own size"
    );
}

/// The size control has to mean something on screen. Fitted rather than drawn
/// naturally, both of these would cover the same area — the bitmap would be
/// stretched to the frame either way — which is exactly the bug this catches.
#[test]
fn a_larger_size_covers_more_of_the_frame() {
    let (device, queue) = gpu_or_skip!();

    let small = ink_pixels(&render(
        &device,
        &queue,
        &title("Hello", 24.0),
        Transform::default(),
    ));
    let large = ink_pixels(&render(
        &device,
        &queue,
        &title("Hello", 72.0),
        Transform::default(),
    ));

    assert!(
        large > small * 2,
        "tripling the size took the coverage from {small} to {large}"
    );
}

/// More words at the same size means more ink, not smaller letters. Under
/// fitting, a longer sentence would be shrunk to fit the same rectangle.
#[test]
fn a_longer_line_does_not_shrink_the_letters() {
    let (device, queue) = gpu_or_skip!();

    let short = ink_pixels(&render(
        &device,
        &queue,
        &title("Hi", 40.0),
        Transform::default(),
    ));
    let long = ink_pixels(&render(
        &device,
        &queue,
        &title("Hi there again", 40.0),
        Transform::default(),
    ));

    assert!(
        long > short * 2,
        "seven times the characters gave {long} ink against {short}"
    );
}

/// The clip's own scale still applies on top of the natural size.
#[test]
fn the_clips_scale_still_works() {
    let (device, queue) = gpu_or_skip!();

    let plain = ink_pixels(&render(
        &device,
        &queue,
        &title("Hello", 40.0),
        Transform::default(),
    ));
    let doubled = ink_pixels(&render(
        &device,
        &queue,
        &title("Hello", 40.0),
        Transform {
            scale: Vec2::new(2.0, 2.0),
            ..Transform::default()
        },
    ));

    assert!(
        doubled > plain * 2,
        "doubling the scale took the coverage from {plain} to {doubled}"
    );
}

/// The transparent parts of the bitmap must let the picture through. A title
/// composited as if it were opaque would paint a black rectangle over the shot.
#[test]
fn the_background_shows_through_around_the_letters() {
    let (device, queue) = gpu_or_skip!();
    let pixels = render(&device, &queue, &title("Hello", 48.0), Transform::default());

    let blue = pixels
        .chunks_exact(4)
        .filter(|p| p[2] > 120 && p[0] < 120)
        .count();
    assert!(
        blue > (OUT_W * OUT_H) as usize / 2,
        "only {blue} pixels of the picture survived; the title is painting its \
         own transparent background over the shot"
    );

    // And specifically at the very corner, which no reasonable title reaches.
    let corner = &pixels[0..4];
    assert!(
        corner[2] > 120,
        "the frame's corner was painted over: {corner:?}"
    );
}

/// The drag handles have to sit on the picture (§41, §54).
///
/// `layer_box` inverts the matrix the shader builds, and for a title that
/// matrix has the natural-size correction folded into it. A sign flip or a
/// forgotten correction puts the handles somewhere the title is not, and
/// nothing else in the program would notice — so this composites a real title
/// and compares where it landed with where the overlay says it is.
#[test]
fn the_drag_handles_sit_on_the_title() {
    use bettercut_editor_core::text::Background;
    use bettercut_ui::preview_overlay::generated_layer_box;

    let (device, queue) = gpu_or_skip!();

    // An opaque panel behind the words, so the edges of the *bitmap* are
    // visible in the result. Glyphs alone leave transparent margins, and their
    // ink bounds are not the layer's bounds.
    let clip = TextClip {
        style: TextStyle {
            size: 48.0,
            color: Rgba::opaque(255, 0, 0),
            stroke: None,
            shadow: None,
            background: Some(Background {
                color: Rgba::opaque(255, 0, 0),
                padding: 10.0,
                corner_radius: 0.0,
            }),
            ..TextStyle::default()
        },
        transform: Transform {
            position: Vec2::new(-0.15, 0.2),
            scale: Vec2::new(1.4, 1.4),
            ..Transform::default()
        },
        ..TextClip::new("Handles", TimelineTime::ZERO).expect("valid")
    };

    let mut titles = TextFrames::new();
    let frame = titles
        .frame_for(&clip, None)
        .expect("rasterized to nothing");
    let pixels = render(&device, &queue, &clip, clip.transform);

    // Where the red panel actually is, in 0..1 frame units.
    let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for y in 0..OUT_H {
        for x in 0..OUT_W {
            let i = ((y * OUT_W + x) * 4) as usize;
            if pixels[i] > 120 && pixels[i + 2] < 120 {
                left = left.min(x as f32);
                top = top.min(y as f32);
                right = right.max(x as f32 + 1.0);
                bottom = bottom.max(y as f32 + 1.0);
            }
        }
    }
    assert!(left <= right, "the title never reached the frame");

    // The exact function the overlay calls, so breaking it here breaks the
    // handles there.
    let expected = generated_layer_box(clip.transform, frame.width, frame.height, OUT_W, OUT_H);

    // Three pixels: the bitmap carries a two-pixel margin outside its
    // background panel, and the composite resamples.
    let tolerance = 3.0;
    for (name, drawn, predicted) in [
        ("left", left / OUT_W as f32, expected.min.x),
        ("top", top / OUT_H as f32, expected.min.y),
        ("right", right / OUT_W as f32, expected.max.x),
        ("bottom", bottom / OUT_H as f32, expected.max.y),
    ] {
        let drawn_px = drawn * OUT_W as f32;
        let predicted_px = predicted * OUT_W as f32;
        assert!(
            (drawn_px - predicted_px).abs() <= tolerance + 4.0,
            "{name}: the handles say {predicted_px:.1} px, the picture is at {drawn_px:.1} px"
        );
    }
}
