//! Colour lookup tables, drawn (`crate::lut`).
//!
//! The claims: a table that changes nothing changes nothing — which is what
//! catches every off-by-half-a-texel mistake in how the table is sampled; a
//! table that swaps channels swaps them; the GPU's lookup agrees with the CPU
//! reference in `bettercut_timeline::lut`; strength mixes; and a clip naming a
//! table that was never loaded is drawn as it is, costing nothing.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::LutId;
use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{
    ClipLook, ClipLut, ColorAdjust, Crop, CubeLut, MasterLook, Resolution, Transform,
};

/// One GPU device for the whole binary; `pixel_read.rs` has the account of why.
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
                label: Some("lut test"),
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
    width: 64,
    height: 8,
};

/// A ramp of colours across the frame, so a lookup is checked at many inputs:
/// red rising left to right, green falling, blue a steady middle value.
fn ramp() -> VideoFrame {
    let (w, h) = (SIZE.width, SIZE.height);
    let mut data = vec![0_u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / (w - 1)) as u8;
            let at = ((y * w + x) * 4) as usize;
            data[at..at + 4].copy_from_slice(&[r, 255 - r, 90, 255]);
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

fn render(compositor: &mut Compositor, frame: &VideoFrame, lut: Option<ClipLut>) -> Vec<[f32; 3]> {
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
                    lut,
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
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                },
            }],
            MasterLook::default(),
        )
        .expect("composite");
    (0..SIZE.width)
        .map(|x| compositor.read_pixel(x, SIZE.height / 2).expect("in frame"))
        .collect()
}

fn compositor(device: &wgpu::Device, queue: &wgpu::Queue) -> Compositor {
    Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(SIZE),
    )
    .expect("compositor")
}

/// A table built from a function of the encoded colour.
fn table(size: u32, f: impl Fn([f32; 3]) -> [f32; 3]) -> CubeLut {
    let identity = CubeLut::identity(size);
    CubeLut {
        table: identity.table.iter().map(|rgb| f(*rgb)).collect(),
        ..identity
    }
}

fn worst(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
    a.iter()
        .zip(b)
        .flat_map(|(p, q)| (0..3).map(move |c| (p[c] - q[c]).abs()))
        .fold(0.0, f32::max)
}

/// **The table that changes nothing changes nothing.** Every way of sampling a
/// 3D table slightly wrong — the edges of the texture instead of the centres of
/// its texels, the axes in the wrong order — shows up here as a tint or a
/// compression of the range.
#[test]
fn an_identity_table_leaves_the_picture_alone() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = ramp();
    let id = LutId::new();
    compositor.load_lut(id, &CubeLut::identity(17));

    let plain = render(&mut compositor, &frame, None);
    let graded = render(&mut compositor, &frame, Some(ClipLut::new(id)));

    let error = worst(&plain, &graded);
    assert!(error < 0.01, "an identity table moved a channel by {error}");
}

/// Swapping red and blue in the table swaps them in the picture: the axes are
/// the right way round.
#[test]
fn a_channel_swap_table_swaps_the_channels() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = ramp();
    let id = LutId::new();
    compositor.load_lut(id, &table(9, |[r, g, b]| [b, g, r]));

    let plain = render(&mut compositor, &frame, None);
    let swapped = render(&mut compositor, &frame, Some(ClipLut::new(id)));

    let expected: Vec<[f32; 3]> = plain.iter().map(|[r, g, b]| [*b, *g, *r]).collect();
    let error = worst(&expected, &swapped);
    assert!(error < 0.02, "the swap is off by {error}");
}

/// The GPU agrees with the CPU reference on a table that is not linear — a
/// strong curve, so trilinear interpolation between coarse samples is
/// genuinely exercised — up to the 8-bit storage of the table.
#[test]
fn the_gpu_agrees_with_the_reference_lookup() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = ramp();
    let lut = table(9, |[r, g, b]| [r * r, g.sqrt(), 1.0 - b]);
    let id = LutId::new();
    compositor.load_lut(id, &lut);

    let plain = render(&mut compositor, &frame, None);
    let graded = render(&mut compositor, &frame, Some(ClipLut::new(id)));

    // `read_pixel` reads display-encoded values, which is what a creative
    // table is built for, so the reference applies the table to them directly.
    let reference: Vec<[f32; 3]> = plain.iter().map(|encoded| lut.apply(*encoded)).collect();
    let error = worst(&reference, &graded);
    assert!(
        error < 0.02,
        "the GPU lookup differs from the reference by {error}"
    );
}

/// Half strength lands between the picture and the full grade.
#[test]
fn strength_mixes_the_grade_in() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = ramp();
    let id = LutId::new();
    compositor.load_lut(id, &table(5, |[r, g, b]| [1.0 - r, 1.0 - g, 1.0 - b]));

    let plain = render(&mut compositor, &frame, None);
    let full = render(&mut compositor, &frame, Some(ClipLut::new(id)));
    let half = render(
        &mut compositor,
        &frame,
        Some(ClipLut {
            lut: id,
            strength: 0.5,
        }),
    );

    // The mix is made on display-encoded values, which `read_pixel` reads, so
    // halfway there is halfway here.
    for ((p, f), h) in plain.iter().zip(&full).zip(&half) {
        let (p, f, h) = (p[0], f[0], h[0]);
        assert!(
            (h - (p + f) / 2.0).abs() < 0.02,
            "half strength gave {h}, between {p} and {f}"
        );
    }
}

/// A clip naming a table that was never loaded — its file is missing — is
/// drawn as it is, and takes no intermediate texture to do it.
#[test]
fn an_unloaded_table_draws_the_picture_as_it_is() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let frame = ramp();

    let plain = render(&mut compositor, &frame, None);
    let missing = render(&mut compositor, &frame, Some(ClipLut::new(LutId::new())));

    assert_eq!(plain, missing);
    assert_eq!(compositor.intermediate_count(), 0);
    assert!(!compositor.has_lut(LutId::new()));
}
