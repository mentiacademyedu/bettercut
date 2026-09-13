//! RGB split and glitch, drawn (`crate::glitch`).
//!
//! RGB split must move red and blue in opposite directions and leave green
//! where it was — or it is a blur, or a shift of the whole picture. Glitch must
//! throw whole bands, differently each frame and identically on the same
//! frame. And neither, set to nothing, may change a pixel or cost a pass.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ClipLook, ColorAdjust, Crop, MasterLook, Resolution, Transform};

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
                label: Some("glitch test"),
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

/// Composite, then read back only `pixels` — reading every pixel one at a
/// time is a readback per pixel, and turned a second-long test into twelve.
fn render(
    compositor: &mut Compositor,
    frame: &VideoFrame,
    rgb_split: f32,
    glitch: f32,
    seed: u32,
    pixels: &[(u32, u32)],
) -> Vec<[f32; 3]> {
    compositor.set_grain_seed(seed);
    compositor
        .composite(
            &[Layer {
                frame,
                look: ClipLook {
                    crop: Crop::NONE,
                    transform: Transform::default(),
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    sharpen: 0.0,
                    lut: None,
                    rgb_split,
                    glitch,
                    reflection: bettercut_timeline::Reflection::None,
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
        .map(|(x, y)| compositor.read_pixel(*x, *y).unwrap())
        .collect()
}

fn compositor(device: &wgpu::Device, queue: &wgpu::Queue) -> Compositor {
    Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(SIZE),
    )
    .unwrap()
}

/// A white bar on black: with a split, red spills out on one side of it and
/// blue on the other, while green stays exactly on the bar.
#[test]
fn rgb_split_pulls_red_and_blue_apart() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let (left, right) = (120, 136);
    let bar = frame(|x, _| {
        if (left..right).contains(&x) {
            [255, 255, 255]
        } else {
            [0, 0, 0]
        }
    });

    let row = SIZE.height / 2;
    // At full, the split is 2% of 256 wide: about five pixels.
    let split = render(
        &mut compositor,
        &bar,
        100.0,
        0.0,
        0,
        &[(left - 3, row), (right + 2, row), (left + 1, row)],
    );

    let [r, g, b] = split[0];
    assert!(
        r > 0.5 && b < 0.1 && g < 0.1,
        "left of the bar should be red only: {r} {g} {b}"
    );
    let [r, g, b] = split[1];
    assert!(
        b > 0.5 && r < 0.1 && g < 0.1,
        "right of the bar should be blue only: {r} {g} {b}"
    );
    let [_, g, _] = split[2];
    assert!(g > 0.9, "green moved off the bar: {g}");
}

/// Glitch throws whole bands: some rows change, each changed band moves as
/// one, the same frame breaks the same way, and the next frame differently.
#[test]
fn glitch_throws_bands_that_change_each_frame() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let ramp = frame(|x, _| [(x * 255 / (SIZE.width - 1)) as u8, 0, 0]);

    // Down the middle column: a thrown band shows as a different red there.
    let column: Vec<(u32, u32)> = (0..SIZE.height).map(|y| (SIZE.width / 2, y)).collect();
    let plain = render(&mut compositor, &ramp, 0.0, 0.0, 3, &column);
    let first = render(&mut compositor, &ramp, 0.0, 100.0, 3, &column);
    let again = render(&mut compositor, &ramp, 0.0, 100.0, 3, &column);
    let next = render(&mut compositor, &ramp, 0.0, 100.0, 4, &column);

    assert_eq!(first, again, "the same frame broke up differently");

    let moved_rows = |pixels: &[[f32; 3]]| -> Vec<u32> {
        (0..SIZE.height)
            .filter(|y| (pixels[*y as usize][0] - plain[*y as usize][0]).abs() > 0.02)
            .collect()
    };
    let (moved_first, moved_next) = (moved_rows(&first), moved_rows(&next));
    assert!(!moved_first.is_empty(), "nothing was thrown at full glitch");
    assert!(
        moved_first.len() < SIZE.height as usize,
        "every row was thrown; that is a shift, not a glitch"
    );
    assert_ne!(
        moved_first, moved_next,
        "the next frame broke up the same way"
    );

    // And *which* bands break changes from frame to frame, not just how far
    // the same bands are thrown: across several frames far more of the
    // picture takes a turn than any one frame breaks.
    let mut union: Vec<u32> = Vec::new();
    let mut largest = 0;
    for seed in 3..11 {
        let frame = render(&mut compositor, &ramp, 0.0, 100.0, seed, &column);
        let rows = moved_rows(&frame);
        largest = largest.max(rows.len());
        for row in rows {
            if !union.contains(&row) {
                union.push(row);
            }
        }
    }
    assert!(
        union.len() * 2 >= largest * 3,
        "the same bands break every frame: {} rows over eight frames, {largest} in one",
        union.len()
    );

    // Rows in one band (28 per frame height: four rows here) move together.
    let band = SIZE.height / 28;
    for y in &moved_first {
        let start = y - y % band;
        for row in start..start + band {
            assert!(
                moved_first.contains(&row),
                "row {row} stayed while row {y} of its band moved"
            );
        }
    }
}

/// Nothing set is nothing drawn, and no pass taken.
#[test]
fn none_changes_nothing() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let ramp = frame(|x, y| [(x % 256) as u8, (y * 2 % 256) as u8, 90]);
    let grid: Vec<(u32, u32)> = (0..SIZE.height)
        .step_by(16)
        .flat_map(|y| (0..SIZE.width).step_by(16).map(move |x| (x, y)))
        .collect();
    let a = render(&mut compositor, &ramp, 0.0, 0.0, 1, &grid);
    assert_eq!(compositor.intermediate_count(), 0);
    let b = render(&mut compositor, &ramp, 0.0, 0.0, 2, &grid);
    assert_eq!(a, b);
}
