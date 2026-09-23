//! The chroma key, on a real device.
//!
//! A key is a claim about pixels, so it is measured in pixels: render a frame
//! that is half green screen and half subject, and count what survived.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ChromaKey, ClipLook, ColorAdjust, MasterLook, Resolution, Transform};

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
                label: Some("chroma key test"),
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

/// A frame whose pixels come from `paint`, given the column and row.
fn frame(paint: impl Fn(u32, u32) -> [u8; 4]) -> VideoFrame {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let mut data = vec![0_u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let at = ((y * w + x) * 4) as usize;
            data[at..at + 4].copy_from_slice(&paint(x, y));
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

/// Chroma-key green on the left half, a subject on the right.
fn screen_and_subject(subject: [u8; 4]) -> VideoFrame {
    frame(move |x, _| {
        if x < OUTPUT.width / 2 {
            [0, 177, 64, 255]
        } else {
            subject
        }
    })
}

/// The everyday case: a subject nothing like the screen.
const ORANGE: [u8; 4] = [230, 120, 40, 255];

/// The hard case: a subject the same family of colour as the screen — an olive
/// jacket in front of a green screen. This is what tolerance is *for*.
const OLIVE: [u8; 4] = [120, 150, 60, 255];

/// Composite one layer over a red ground and hand back the pixels.
///
/// Red, because it is nothing like either the screen or the subject: wherever
/// the ground shows through, the key removed something.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    key: Option<ChromaKey>,
    subject: [u8; 4],
) -> Vec<u8> {
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");

    let ground = frame(|_, _| [255, 0, 0, 255]);
    let shot = screen_and_subject(subject);
    compositor
        .composite(
            &[
                Layer {
                    frame: &ground,

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
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: None,
                        luma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
                Layer {
                    frame: &shot,

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
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: key,
                        luma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
            ],
            MasterLook::default(),
        )
        .expect("composite");

    read_back(device, queue, compositor.target())
}

/// The pixel at the middle of the left (screen) half and of the right
/// (subject) half.
fn halves(pixels: &[u8]) -> ([u8; 3], [u8; 3]) {
    let row = OUTPUT.height / 2;
    let at = |x: u32| {
        let i = ((row * OUTPUT.width + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    (at(OUTPUT.width / 4), at(OUTPUT.width * 3 / 4))
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (w, h) = (OUTPUT.width, OUTPUT.height);
    let padded = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("read back"),
        size: u64::from(padded * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
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
    let mut tight = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h {
        let start = (row * padded) as usize;
        tight.extend_from_slice(&mapped[start..start + (w * 4) as usize]);
    }
    drop(mapped);
    buffer.unmap();
    tight
}

/// The screen goes, the subject stays.
#[test]
fn the_screen_is_removed_and_the_subject_is_not() {
    let (device, queue) = gpu_or_skip!();
    let key = ChromaKey {
        color: [0.0, 0.69, 0.25], // the screen's own colour
        ..ChromaKey::default()
    };

    let (screen, subject) = halves(&render(&device, &queue, Some(key), ORANGE));

    assert!(
        screen[0] > 200 && screen[1] < 60,
        "the ground does not show through the screen: {screen:?}"
    );
    assert!(
        subject[0] > 150 && subject[1] > 60,
        "the subject was removed too: {subject:?}"
    );
}

/// Without a key, nothing is removed — the same frame covers the ground.
#[test]
fn no_key_removes_nothing() {
    let (device, queue) = gpu_or_skip!();
    let (screen, subject) = halves(&render(&device, &queue, None, ORANGE));

    assert!(
        screen[1] > screen[0],
        "the screen half is not green: {screen:?}"
    );
    assert!(
        subject[0] > subject[1],
        "the subject half is not orange: {subject:?}"
    );
}

/// A key of some *other* colour leaves the screen alone: it removes what it was
/// pointed at, not whatever happens to be most saturated.
#[test]
fn a_key_of_another_colour_leaves_the_screen() {
    let (device, queue) = gpu_or_skip!();
    let key = ChromaKey {
        color: [0.0, 0.2, 1.0], // blue
        ..ChromaKey::default()
    };

    let (screen, _) = halves(&render(&device, &queue, Some(key), ORANGE));
    assert!(
        screen[1] > screen[0],
        "a blue key removed a green screen: {screen:?}"
    );
}

/// Tolerance is the dial that matters, and what it decides is the near cases:
/// an olive jacket in front of a green screen is kept by a tight key and eaten
/// by a loose one.
#[test]
fn tolerance_decides_the_near_cases() {
    let (device, queue) = gpu_or_skip!();
    let narrow = ChromaKey {
        color: [0.0, 0.69, 0.25],
        tolerance: 0.02,
        softness: 0.01,
        spill: 0.0,
    };
    // The olive sits about 0.48 from the screen on the chromaticity plane, so
    // this is a key wide enough to reach it and the tight one above is not.
    let wide = ChromaKey {
        tolerance: 0.5,
        ..narrow
    };

    let (_, kept) = halves(&render(&device, &queue, Some(narrow), OLIVE));
    let (_, eaten) = halves(&render(&device, &queue, Some(wide), OLIVE));

    assert!(
        kept[1] > kept[0],
        "a tight key should leave an olive jacket alone: {kept:?}"
    );
    assert!(
        eaten[0] > 150 && eaten[1] < 90,
        "a loose key should take it: {eaten:?}"
    );
}

/// The dial cannot be turned up until the picture disappears: even the widest
/// key keeps a subject that is nothing like the screen. Chromaticity distance
/// runs past 1.0 and `MAX_SPREAD` is deliberately short of that — a key that
/// can remove everything is not a key.
#[test]
fn even_the_widest_key_keeps_an_unrelated_colour() {
    let (device, queue) = gpu_or_skip!();
    let key = ChromaKey {
        color: [0.0, 0.69, 0.25],
        tolerance: ChromaKey::MAX_SPREAD,
        softness: ChromaKey::MAX_SPREAD,
        spill: 0.0,
    };

    let (_, subject) = halves(&render(&device, &queue, Some(key), ORANGE));
    assert!(
        subject[0] > 150,
        "the widest key removed an orange subject: {subject:?}"
    );
}

/// Black is not a colour with proportions, and a key must not eat it: shadows
/// and dark hair are the first thing a bad key destroys.
#[test]
fn black_survives_a_key() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");

    let ground = frame(|_, _| [255, 0, 0, 255]);
    let dark = frame(|_, _| [0, 0, 0, 255]);
    compositor
        .composite(
            &[
                Layer {
                    frame: &ground,

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
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: None,
                        luma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
                Layer {
                    frame: &dark,

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
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: Some(ChromaKey::default()),
                        luma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
            ],
            MasterLook::default(),
        )
        .expect("composite");

    let pixels = read_back(&device, &queue, compositor.target());
    let middle = ((OUTPUT.height / 2 * OUTPUT.width + OUTPUT.width / 2) * 4) as usize;
    assert!(
        pixels[middle] < 40,
        "the key ate black, so every shadow would vanish: {:?}",
        &pixels[middle..middle + 3]
    );
}

/// The reason the key works on proportions rather than on colour distance: a
/// screen is never evenly lit, and the shadowed folds of it are a long way from
/// the lit parts in plain RGB. Keyed on proportions they are the same colour,
/// so both go; keyed on distance the shadows survive as dark green fringes.
#[test]
fn a_shadowed_screen_keys_as_readily_as_a_lit_one() {
    let (device, queue) = gpu_or_skip!();

    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor");

    // The same green at full light on the left and deep shadow on the right.
    let ground = frame(|_, _| [255, 0, 0, 255]);
    let lit_and_shadowed = frame(|x, _| {
        if x < OUTPUT.width / 2 {
            [0, 177, 64, 255]
        } else {
            [0, 53, 19, 255]
        }
    });
    compositor
        .composite(
            &[
                Layer {
                    frame: &ground,

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
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: None,
                        luma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
                Layer {
                    frame: &lit_and_shadowed,

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
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        // Picked off the lit part, as a user would.
                        luma_key: None,
                        chroma_key: Some(ChromaKey {
                            color: [0.0, 0.69, 0.25],
                            ..ChromaKey::default()
                        }),
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                },
            ],
            MasterLook::default(),
        )
        .expect("composite");

    let (lit, shadowed) = halves(&read_back(&device, &queue, compositor.target()));
    assert!(
        lit[0] > 200 && lit[1] < 60,
        "the lit half of the screen survived: {lit:?}"
    );
    assert!(
        shadowed[0] > 200 && shadowed[1] < 60,
        "the shadowed half of the screen survived, so every fold of a real \
         screen would leave a dark green fringe: {shadowed:?}"
    );
}
