//! Golden-frame tests (§51.1).
//!
//! > §46 is enforced by test, not by discipline.
//!
//! §46 is the guide's most-repeated architectural rule — one render graph, two
//! configurations — and the failure it guards against is the one a user finds
//! last: an export that does not match the preview they approved. These tests
//! are the mechanism. **A golden-frame failure is a release blocker.**
//!
//! ## Two different questions, two different checks
//!
//! **Does export match preview?** Render the same layers under both configs at
//! the same resolution and compare per pixel. This is §51.1's stated procedure
//! and it is driver-independent: both sides run on whatever GPU is present, so
//! the comparison holds anywhere.
//!
//! **Did the picture change at all?** The check above cannot answer that — if
//! both configs regress together it stays green. So each case also has a small
//! stored signature under `tests/golden/`, compared with a tolerance.
//!
//! A signature is a 4×4 grid holding, per cell, the mean linear-light RGB and
//! the sharpest step between neighbouring texels — where the light is, and how
//! crisp it is. Not a stored PNG: a full reference image would have to be
//! regenerated per GPU, because an NVIDIA
//! card and an Intel iGPU disagree in the last bit of 8-bit rounding, so a
//! byte-exact reference fails on the second machine for reasons that are not
//! regressions — and a reference nobody trusts gets deleted. A coarse signature
//! survives that and still catches what actually goes wrong: a clip rendering
//! black, a transform flipping, colour inverting, an effect silently bypassed.
//!
//! Regenerate deliberately, after looking at why one moved:
//!
//! ```text
//! BETTERCUT_UPDATE_GOLDEN=1 cargo test -p bettercut-renderer --test golden_frames
//! ```
//!
//! ## What §51.1 lists that is not here
//!
//! * **Text overlay** — Phase 2 (§61), so there is nothing to render.
//! * **Limited-range and BT.601 sources** — these are resolved at the *decode*
//!   boundary (§21a.1: "colour resolved once at the upload boundary"), so by
//!   the time a frame reaches the compositor it is already sRGB RGBA and its
//!   `ColorMetadata` is a record of where it came from. Feeding the compositor
//!   two frames tagged differently would produce identical output and prove
//!   nothing; that conversion is the decoder's test to own.
//! * **NTSC frame-rate source** — a timing property (§9), not a pixel one. It
//!   is covered where the ticks are, in the timeline and playback crates.
//!
//! Skips itself when no adapter is available, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ColorAdjust, MasterLook, Resolution, Transform, Vec2};

const SIZE: u32 = 256;
/// Cells a side in a stored signature.
const CELLS: u32 = 4;
/// How far a signature may drift before it is a regression rather than a
/// driver. 8-bit rounding is about 0.004 in sRGB; this is several times that.
const SIGNATURE_TOLERANCE: f32 = 0.02;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("golden frame test"),
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

/// A frame with structure in every direction: a red-green gradient, a blue
/// wedge down the diagonal, and a white block off-centre.
///
/// Deliberately asymmetric. A symmetric fixture cannot tell a correct transform
/// from one that flipped or transposed the picture, which is the class of bug a
/// golden frame is meant to catch.
fn fixture_frame() -> VideoFrame {
    let stride = SIZE * 4;
    let mut data = vec![0_u8; (stride * SIZE) as usize];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let at = (y * stride + x * 4) as usize;
            let red = (x * 255 / (SIZE - 1)) as u8;
            let green = (y * 255 / (SIZE - 1)) as u8;
            let blue = if x + y < SIZE { 200 } else { 40 };
            data[at..at + 4].copy_from_slice(&[red, green, blue, 255]);
        }
    }

    // An off-centre white block: bright, and in a place no symmetry maps onto
    // itself.
    for y in (SIZE / 8)..(SIZE / 8 + SIZE / 6) {
        for x in (SIZE / 6)..(SIZE / 6 + SIZE / 4) {
            let at = (y * stride + x * 4) as usize;
            data[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
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

/// One case's layers, built fresh per render because a `Layer` borrows a frame.
type Build = fn(&VideoFrame) -> Vec<Layer<'_>>;

fn plain(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        frame,
        transform: Transform::default(),
        opacity: 1.0,
        color: ColorAdjust::default(),
        blur: 0.0,
    }]
}

fn transformed(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        transform: Transform {
            position: Vec2::new(0.15, -0.1),
            scale: Vec2::new(0.7, 0.7),
            rotation_degrees: 22.0,
            anchor: Vec2::new(0.5, 0.5),
        },
        ..plain(frame).remove(0)
    }]
}

fn colour_graded(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        color: ColorAdjust {
            brightness: 1.3,
            contrast: 1.4,
            saturation: 0.6,
        },
        ..plain(frame).remove(0)
    }]
}

fn blurred(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        blur: 45.0,
        ..plain(frame).remove(0)
    }]
}

/// §22's compositing: a half-opaque layer over an opaque one, blended in linear
/// light. The case most likely to expose a colour-space mistake, because the
/// wrong space is still plausible-looking.
fn two_tracks(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![
        Layer {
            color: ColorAdjust {
                brightness: 0.4,
                ..ColorAdjust::default()
            },
            ..plain(frame).remove(0)
        },
        Layer {
            transform: Transform {
                position: Vec2::new(-0.2, 0.15),
                scale: Vec2::new(0.55, 0.55),
                ..Transform::default()
            },
            opacity: 0.5,
            ..plain(frame).remove(0)
        },
    ]
}

/// Everything at once, which is what a real clip looks like and the only case
/// that would catch two features interacting.
fn everything(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![
        two_tracks(frame).remove(0),
        Layer {
            transform: Transform {
                position: Vec2::new(-0.2, 0.15),
                scale: Vec2::new(0.55, 0.55),
                rotation_degrees: -15.0,
                anchor: Vec2::new(0.5, 0.5),
            },
            opacity: 0.65,
            color: ColorAdjust {
                brightness: 1.2,
                contrast: 0.8,
                saturation: 1.5,
            },
            blur: 25.0,
            ..plain(frame).remove(0)
        },
    ]
}

const CASES: &[(&str, Build)] = &[
    ("plain", plain),
    ("transform", transformed),
    ("colour", colour_graded),
    ("blur", blurred),
    ("two_tracks", two_tracks),
    ("everything", everything),
];

/// Render one case under one configuration and read it back as linear RGB.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    build: Build,
    config: RenderConfig,
) -> Vec<[f32; 3]> {
    let mut compositor =
        Compositor::new(device.clone(), queue.clone(), config).expect("compositor");
    let frame = fixture_frame();
    compositor
        .composite(&build(&frame), MasterLook::default())
        .expect("composite");
    read_back(device, queue, compositor.target())
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<[f32; 3]> {
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
            let at = row + (x * 4) as usize;
            out.push([
                srgb_to_linear(mapped[at]),
                srgb_to_linear(mapped[at + 1]),
                srgb_to_linear(mapped[at + 2]),
            ]);
        }
    }
    drop(mapped);
    buffer.unmap();
    out
}

/// The inverse of what the sRGB-aware target applied on write (§21a.1).
///
/// Comparisons happen in linear light because that is the space the
/// compositing happened in; a difference measured on the stored bytes would be
/// weighted by the transfer curve rather than by how wrong it looks.
fn srgb_to_linear(byte: u8) -> f32 {
    let value = f32::from(byte) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// Per cell of a `CELLS`×`CELLS` grid: the mean linear RGB, and how much
/// **detail** the cell holds — the mean absolute difference between
/// horizontally adjacent texels.
///
/// The detail term is what makes the signature able to see an effect at all.
/// Blur moves light around without creating or destroying any — that is the
/// property `blur_gpu.rs` asserts — so a mean-only signature reads a blurred
/// frame and a sharp one as the same picture, and
/// `the_signatures_tell_the_cases_apart` failed until this was added.
///
/// Two earlier attempts at that term were not enough, and the reason is worth
/// recording:
///
/// * **Standard deviation** over a 64-texel cell is dominated by the fixture's
///   gradient sweeping across it, which a small blur barely touches.
/// * **Mean** neighbour difference washes the edges out — a 64-texel edge
///   averaged over 4096 texels is divided away by the smooth majority.
///
/// So it is the *strongest* edge in the cell. Sparse by nature and the thing
/// blur actually destroys: a hard white-to-black step is a neighbour difference
/// near 1.0 sharp and near 0.15 once blurred, which no amount of 8-bit rounding
/// confuses.
fn signature(pixels: &[[f32; 3]]) -> Vec<f32> {
    let cell = SIZE / CELLS;
    let mut out = Vec::with_capacity((CELLS * CELLS * 6) as usize);
    let at = |x: u32, y: u32| pixels[(y * SIZE + x) as usize];

    for row in 0..CELLS {
        for column in 0..CELLS {
            let mut total = [0.0_f64; 3];
            let mut sharpest = [0.0_f32; 3];
            for y in (row * cell)..((row + 1) * cell) {
                for x in (column * cell)..((column + 1) * cell) {
                    let pixel = at(x, y);
                    // The last column of the last cell has no right neighbour.
                    let next = at((x + 1).min(SIZE - 1), y);
                    for channel in 0..3 {
                        total[channel] += f64::from(pixel[channel]);
                        sharpest[channel] =
                            sharpest[channel].max((next[channel] - pixel[channel]).abs());
                    }
                }
            }

            let count = f64::from(cell * cell);
            out.extend(total.iter().map(|sum| (sum / count) as f32));
            out.extend_from_slice(&sharpest);
        }
    }
    out
}

fn golden_path(case: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{case}.txt"))
}

fn write_golden(case: &str, values: &[f32]) {
    let path = golden_path(case);
    std::fs::create_dir_all(path.parent().expect("has a parent")).expect("create tests/golden");
    let mut text = format!(
        "# {case}: one line per cell of a {CELLS}x{CELLS} grid, row-major.\n\
         # mean R G B, then the sharpest neighbour step R G B, in linear light.\n\
         # Regenerate with BETTERCUT_UPDATE_GOLDEN=1 (see the module docs first).\n"
    );
    for cell in values.chunks(6) {
        text.push_str(&format!(
            "{:.4} {:.4} {:.4}   {:.4} {:.4} {:.4}\n",
            cell[0], cell[1], cell[2], cell[3], cell[4], cell[5]
        ));
    }
    std::fs::write(&path, text).expect("write golden");
}

fn read_golden(case: &str) -> Option<Vec<f32>> {
    let text = std::fs::read_to_string(golden_path(case)).ok()?;
    Some(
        text.lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .flat_map(str::split_whitespace)
            .filter_map(|token| token.parse::<f32>().ok())
            .collect(),
    )
}

/// §51.1's stated procedure: the same frame through both configurations, per
/// pixel, within tolerance.
///
/// The tiers really do differ — blur spends 16 taps a side under preview and 40
/// under export — so this is not comparing a thing to itself. What it asserts
/// is that the difference is confined to how finely the work is sampled and
/// never reaches the picture.
#[test]
fn preview_and_export_render_the_same_picture() {
    let (device, queue) = gpu_or_skip!();
    let resolution = Resolution::new(SIZE, SIZE);

    for (name, build) in CASES {
        let preview = render(&device, &queue, *build, RenderConfig::preview(resolution));
        let export = render(
            &device,
            &queue,
            *build,
            RenderConfig::export_to_texture(resolution),
        );

        let mut worst = 0.0_f32;
        let mut total = 0.0_f64;
        for (a, b) in preview.iter().zip(&export) {
            for channel in 0..3 {
                let difference = (a[channel] - b[channel]).abs();
                worst = worst.max(difference);
                total += f64::from(difference);
            }
        }
        let mean = (total / (preview.len() * 3) as f64) as f32;

        assert!(
            mean < 0.002,
            "{name}: preview and export differ by {mean:.5} on average"
        );
        assert!(
            worst < 0.08,
            "{name}: preview and export differ by {worst:.5} at the worst texel"
        );
    }
}

/// The check the one above structurally cannot make: has the picture moved
/// since the reference was taken?
///
/// A regression that hits preview and export equally — a transform matrix
/// transposed, a colour pivot changed, an effect quietly skipped — keeps that
/// test green and fails this one.
#[test]
fn each_case_still_matches_its_stored_signature() {
    let (device, queue) = gpu_or_skip!();
    let update = std::env::var_os("BETTERCUT_UPDATE_GOLDEN").is_some();
    let resolution = Resolution::new(SIZE, SIZE);
    let mut missing = Vec::new();

    for (name, build) in CASES {
        let rendered = signature(&render(
            &device,
            &queue,
            *build,
            RenderConfig::export_to_texture(resolution),
        ));

        if update {
            write_golden(name, &rendered);
            continue;
        }

        let Some(stored) = read_golden(name) else {
            missing.push(*name);
            continue;
        };
        assert_eq!(
            stored.len(),
            rendered.len(),
            "{name}: stored signature has {} values, expected {}",
            stored.len(),
            rendered.len()
        );

        for (index, (expected, actual)) in stored.iter().zip(&rendered).enumerate() {
            let cell = index / 6;
            let within = index % 6;
            let measure = if within < 3 { "mean" } else { "detail" };
            let channel = ["red", "green", "blue"][within % 3];
            assert!(
                (expected - actual).abs() < SIGNATURE_TOLERANCE,
                "{name}: row {} column {}, {channel} {measure} moved \
                 {expected:.4} -> {actual:.4}\n\
                 If this change is intended, regenerate with BETTERCUT_UPDATE_GOLDEN=1.",
                cell / CELLS as usize,
                cell % CELLS as usize,
            );
        }
    }

    assert!(
        missing.is_empty() || update,
        "no stored signature for {missing:?}; generate with BETTERCUT_UPDATE_GOLDEN=1"
    );
}

/// The signatures only mean something if different pictures produce different
/// ones. A signature so coarse that every case looked alike would pass the test
/// above forever.
#[test]
fn the_signatures_tell_the_cases_apart() {
    let (device, queue) = gpu_or_skip!();
    let resolution = Resolution::new(SIZE, SIZE);

    let signatures: Vec<(&str, Vec<f32>)> = CASES
        .iter()
        .map(|(name, build)| {
            (
                *name,
                signature(&render(
                    &device,
                    &queue,
                    *build,
                    RenderConfig::export_to_texture(resolution),
                )),
            )
        })
        .collect();

    for (index, (name, one)) in signatures.iter().enumerate() {
        for (other_name, other) in &signatures[index + 1..] {
            let worst = one
                .iter()
                .zip(other)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            assert!(
                worst > SIGNATURE_TOLERANCE * 2.0,
                "{name} and {other_name} produce nearly identical signatures \
                 (worst difference {worst:.4}); the signature is too coarse to \
                 catch a regression between them"
            );
        }
    }
}
