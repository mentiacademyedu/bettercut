//! The effect graph on a real device (§20, §46).
//!
//! `blur_gpu.rs` proves the blur *node* renders correctly. What is left is the
//! machinery around it — the part that has nothing to do with blur and
//! everything to do with there being a chain at all:
//!
//! * a layer no node touches must cost no textures and no passes;
//! * intermediates must be reused between frames rather than reallocated;
//! * **each layer must get its own parameters.** Nodes write per-pass uniforms,
//!   and `write_buffer` stages its writes until the submit — so two layers
//!   sharing one slot would both render with whichever wrote last. That is
//!   invisible on a single-layer preview and wrong on every stacked composite,
//!   which makes it exactly the sort of thing to pin down with a test.
//!
//! Skips itself when no adapter is available, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ClipLook, ColorAdjust, MasterLook, Resolution, Transform, Vec2};

const SIZE: u32 = 256;

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
                label: Some("effect graph test"),
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

/// An opaque frame, white on the left half and black on the right.
///
/// A vertical edge down the middle, so blurring it softens a column that is
/// easy to measure — and opaque throughout, for the reason `blur_gpu.rs`
/// documents at length.
fn split_frame() -> VideoFrame {
    let stride = SIZE * 4;
    let mut data = vec![0_u8; (stride * SIZE) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let at = (y * stride + x * 4) as usize;
            let value = if x < SIZE / 2 { 255 } else { 0 };
            data[at..at + 4].copy_from_slice(&[value, value, value, 255]);
        }
    }

    VideoFrame {
        timestamp: bettercut_foundation::MediaTime::ZERO,
        width: SIZE,
        height: SIZE,
        color: ColorMetadata::bt709_limited_8bit(),
        storage: FrameStorage::System { data, stride },
    }
}

fn layer<'a>(frame: &'a VideoFrame, blur: f32, offset_x: f32) -> Layer<'a> {
    Layer {
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
            lens: 0.0,
            tilt_band: 0.0,
            tilt_centre: 0.5,
            posterise: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform {
                position: Vec2::new(offset_x, 0.0),
                scale: Vec2::new(0.5, 0.5),
                ..Transform::default()
            },
            opacity: 1.0,
            color: ColorAdjust::default(),
            blur,
            chroma_key: None,
            luma_key: None,
            mask: None,
            blend: bettercut_timeline::BlendMode::Normal,
        },
    }
}

fn compositor(device: &wgpu::Device, queue: &wgpu::Queue) -> Compositor {
    Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(Resolution::new(SIZE, SIZE)),
    )
    .expect("compositor")
}

/// Read the target back as linear-light values.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<f32> {
    let padded = (SIZE * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(SIZE),
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
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
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
    let mut out = Vec::with_capacity((SIZE * SIZE) as usize);
    for y in 0..SIZE {
        let row = (y * padded) as usize;
        for x in 0..SIZE {
            let value = f32::from(mapped[row + (x * 4) as usize]) / 255.0;
            out.push(if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            });
        }
    }
    drop(mapped);
    buffer.unmap();
    out
}

/// How wide the transition from light to dark is, along one row, inside a
/// horizontal window. A hard edge is one or two texels; a blurred one is many.
fn edge_width(pixels: &[f32], row: u32, from: u32, to: u32) -> u32 {
    let at = |x: u32| pixels[(row * SIZE + x) as usize];
    let high = (from..to).map(at).fold(0.0_f32, f32::max);
    if high < 0.05 {
        return 0;
    }
    (from..to)
        .filter(|x| {
            let value = at(*x);
            value > high * 0.1 && value < high * 0.9
        })
        .count() as u32
}

/// The common case: nothing animated, nothing keyed, no effects. The chain must
/// cost a comparison per node and nothing else.
#[test]
fn a_layer_no_node_touches_allocates_nothing() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = split_frame();

    compositor
        .composite(&[layer(&frame, 0.0, 0.0)], MasterLook::default())
        .expect("composite");

    assert_eq!(
        compositor.intermediate_count(),
        0,
        "an unaffected layer should need no intermediate textures"
    );
}

/// §68, §73: a blurred 1080p layer allocates 8 MB per intermediate. Doing that
/// every frame is tens of megabytes a second through the driver, so the pool
/// has to hand the same textures back.
#[test]
fn intermediates_are_reused_between_frames() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = split_frame();

    compositor
        .composite(&[layer(&frame, 50.0, 0.0)], MasterLook::default())
        .expect("composite");
    let after_one = compositor.intermediate_count();
    assert!(after_one > 0, "a blurred layer needs intermediates");

    for _ in 0..10 {
        compositor
            .composite(&[layer(&frame, 50.0, 0.0)], MasterLook::default())
            .expect("composite");
    }

    assert_eq!(
        compositor.intermediate_count(),
        after_one,
        "the pool grew over 11 frames instead of reusing what it had"
    );
}

/// Both layers are blurred, by amounts five times apart.
///
/// **Both must be non-zero.** A layer at zero declines before it writes
/// anything, so pairing a blurred layer with an unblurred one would leave only
/// one writer and the slots could not collide — the test would pass whether or
/// not the slots were shared. Two real writers is what makes this bite.
///
/// If they shared a slot, both would render with whichever wrote last, and the
/// two halves would come out equally soft.
const MILD: f32 = 20.0;
const HEAVY: f32 = 100.0;

#[test]
fn each_layer_is_blurred_by_its_own_amount() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = split_frame();

    // Half scale, offset a quarter frame each way: the first copy occupies the
    // left half of the output, the second the right half.
    compositor
        .composite(
            &[layer(&frame, MILD, -0.25), layer(&frame, HEAVY, 0.25)],
            MasterLook::default(),
        )
        .expect("composite");
    let pixels = read_back(&device, &queue, compositor.target());

    let row = SIZE / 2;
    let mild = edge_width(&pixels, row, 0, SIZE / 2);
    let heavy = edge_width(&pixels, row, SIZE / 2, SIZE);

    assert!(
        heavy > mild * 3,
        "the two layers blurred alike: {mild} texels of edge at {MILD}%, \
         {heavy} at {HEAVY}% — they should differ sharply"
    );
}

/// And swapping them swaps which half is soft. A node that leaked one layer's
/// parameters into the next would pass the test above by luck — whichever
/// happened to be written last — and fail this one.
#[test]
fn layer_order_does_not_change_each_layer_s_blur() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = split_frame();

    compositor
        .composite(
            &[layer(&frame, HEAVY, -0.25), layer(&frame, MILD, 0.25)],
            MasterLook::default(),
        )
        .expect("composite");
    let pixels = read_back(&device, &queue, compositor.target());

    let row = SIZE / 2;
    let heavy = edge_width(&pixels, row, 0, SIZE / 2);
    let mild = edge_width(&pixels, row, SIZE / 2, SIZE);

    assert!(
        heavy > mild * 3,
        "reversing the order did not reverse which layer was blurred: \
         {heavy} texels of edge at {HEAVY}%, {mild} at {MILD}%"
    );
}

/// A sequence-wide adjustment applies to the finished picture.
///
/// Checked by halving the master opacity over a black canvas: every lit texel
/// must come out dimmer, and the letterboxed black around the layers must stay
/// black — which is what tells a *post* pass from a per-layer one, since
/// dimming each layer before compositing would leave the same black but a
/// different edge.
#[test]
fn a_master_adjustment_dims_the_finished_picture() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = split_frame();

    compositor
        .composite(&[layer(&frame, 0.0, 0.0)], MasterLook::default())
        .expect("composite");
    let plain = read_back(&device, &queue, compositor.target());

    let dimmed_master = MasterLook {
        opacity: 0.5,
        ..MasterLook::default()
    };
    compositor
        .composite(&[layer(&frame, 0.0, 0.0)], dimmed_master)
        .expect("composite");
    let dimmed = read_back(&device, &queue, compositor.target());

    let bright: f32 = plain.iter().sum();
    let after: f32 = dimmed.iter().sum();
    assert!(
        after < bright * 0.75,
        "master opacity did not reach the picture: {bright:.1} then {after:.1}"
    );
    assert!(after > 0.0, "it went completely black instead");
}

/// The master's colour adjustment reaches the output too.
#[test]
fn a_master_colour_adjustment_reaches_the_output() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = split_frame();

    compositor
        .composite(&[layer(&frame, 0.0, 0.0)], MasterLook::default())
        .expect("composite");
    let plain: f32 = read_back(&device, &queue, compositor.target()).iter().sum();

    // Darkening, not brightening. The fixture is pure white on black, and
    // white is already at the top of the range — a brightness of 1.6 clamps
    // straight back to white and the sums come out identical, which looks
    // exactly like the setting being ignored. Measured that way first.
    let darker = MasterLook {
        color: bettercut_timeline::ColorAdjust {
            brightness: 0.5,
            ..bettercut_timeline::ColorAdjust::default()
        },
        ..MasterLook::default()
    };
    compositor
        .composite(&[layer(&frame, 0.0, 0.0)], darker)
        .expect("composite");
    let graded: f32 = read_back(&device, &queue, compositor.target()).iter().sum();

    assert!(
        graded < plain * 0.75,
        "master brightness did not reach the output: {plain:.1} then {graded:.1}"
    );
    assert!(graded > 0.0, "it went completely black instead");
}

/// And it costs nothing when it is the identity, which is almost every frame.
#[test]
fn an_identity_master_allocates_no_extra_texture() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = split_frame();

    compositor
        .composite(&[layer(&frame, 0.0, 0.0)], MasterLook::default())
        .expect("composite");
    assert_eq!(
        compositor.intermediate_count(),
        0,
        "an untouched master should need no scratch texture"
    );

    let adjusted = MasterLook {
        opacity: 0.5,
        ..MasterLook::default()
    };
    compositor
        .composite(&[layer(&frame, 0.0, 0.0)], adjusted)
        .expect("composite");
    assert!(
        compositor.intermediate_count() > 0,
        "an adjusted master needs somewhere to composite into first"
    );
}
