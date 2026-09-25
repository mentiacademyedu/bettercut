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
use bettercut_renderer::{Compositor, Grade, Layer, RenderConfig};
use bettercut_timeline::{
    BlendMode, ChromaKey, ClipLook, ColorAdjust, Mask, MaskShape, MasterLook, Resolution,
    Transform, Vec2,
};

const SIZE: u32 = 256;
/// Cells a side in a stored signature.
const CELLS: u32 = 4;
/// How far a signature may drift before it is a regression rather than a
/// driver. 8-bit rounding is about 0.004 in sRGB; this is several times that.
const SIGNATURE_TOLERANCE: f32 = 0.02;

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
                label: Some("golden frame test"),
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

/// The white a §25 flash lays over the cut.
///
/// The frame the engine actually generates, not a copy of it — a copy would go
/// on passing after the real one changed. `'static` because a case's layers
/// borrow from the fixture frame's lifetime and this one outlives every case.
fn flash_white() -> &'static VideoFrame {
    static WHITE: std::sync::OnceLock<VideoFrame> = std::sync::OnceLock::new();
    WHITE.get_or_init(|| bettercut_playback::solid_frame([255, 255, 255]))
}

fn plain(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
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
            smooth_skin: 0.0,
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
    }]
}

fn transformed(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        look: ClipLook {
            corner_pin: Default::default(),
            transform: Transform {
                position: Vec2::new(0.15, -0.1),
                scale: Vec2::new(0.7, 0.7),
                rotation_degrees: 22.0,
                anchor: Vec2::new(0.5, 0.5),
                flip_h: false,
                flip_v: false,
            },
            ..plain(frame).remove(0).look
        },
        ..plain(frame).remove(0)
    }]
}

/// Mirrored on both axes, off centre and turned, so the case exercises the
/// mirror's interaction with the anchor and the rotation rather than the easy
/// symmetric one — where a flip is invisible.
fn mirrored(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        look: ClipLook {
            corner_pin: Default::default(),
            transform: Transform {
                position: Vec2::new(-0.12, 0.08),
                scale: Vec2::new(0.65, 0.65),
                rotation_degrees: 18.0,
                anchor: Vec2::new(0.25, 0.75),
                flip_h: true,
                flip_v: true,
            },
            ..plain(frame).remove(0).look
        },
        ..plain(frame).remove(0)
    }]
}

/// §22's crop, uneven on all four edges so no symmetry can hide an axis being
/// swapped — and combined with a transform, because the crop runs first and the
/// two together are where an ordering mistake shows.
fn cropped(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        look: ClipLook {
            corner_pin: Default::default(),
            crop: bettercut_timeline::Crop {
                left: 0.12,
                top: 0.3,
                right: 0.25,
                bottom: 0.05,
            },
            transform: Transform {
                position: Vec2::new(0.08, -0.05),
                scale: Vec2::new(0.8, 0.8),
                rotation_degrees: -10.0,
                anchor: Vec2::new(0.5, 0.5),
                flip_h: false,
                flip_v: false,
            },
            ..plain(frame).remove(0).look
        },
        ..plain(frame).remove(0)
    }]
}

fn colour_graded(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        look: ClipLook {
            corner_pin: Default::default(),
            color: ColorAdjust {
                brightness: 1.3,
                contrast: 1.4,
                saturation: 0.6,
                temperature: 0.5,
                tint: -0.25,
                vibrance: 0.0,
                wheels: bettercut_timeline::ColorWheels::IDENTITY,
                secondary: bettercut_timeline::HslSecondary::IDENTITY,
            },
            ..plain(frame).remove(0).look
        },
        ..plain(frame).remove(0)
    }]
}

fn blurred(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        look: ClipLook {
            corner_pin: Default::default(),
            blur: 45.0,
            ..plain(frame).remove(0).look
        },
        ..plain(frame).remove(0)
    }]
}

/// A kaleidoscope: the one reflection that maps through angles, and so the one
/// where preview and export could most plausibly round differently.
fn kaleidoscoped(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        look: ClipLook {
            corner_pin: Default::default(),
            reflection: bettercut_timeline::Reflection::Kaleidoscope,
            ..plain(frame).remove(0).look
        },
        ..plain(frame).remove(0)
    }]
}

/// Sharpened, and on its own: the fixture's gradient and its hard white block
/// give the mask both a smooth area to leave alone and edges to bring out.
fn sharpened(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![Layer {
        look: ClipLook {
            corner_pin: Default::default(),
            sharpen: 70.0,
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
            ..plain(frame).remove(0).look
        },
        ..plain(frame).remove(0)
    }]
}

/// §22's compositing: a half-opaque layer over an opaque one, blended in linear
/// light. The case most likely to expose a colour-space mistake, because the
/// wrong space is still plausible-looking.
fn two_tracks(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![
        Layer {
            look: ClipLook {
                corner_pin: Default::default(),
                color: ColorAdjust {
                    brightness: 0.4,
                    ..ColorAdjust::default()
                },
                ..plain(frame).remove(0).look
            },
            ..plain(frame).remove(0)
        },
        Layer {
            look: ClipLook {
                corner_pin: Default::default(),
                transform: Transform {
                    position: Vec2::new(-0.2, 0.15),
                    scale: Vec2::new(0.55, 0.55),
                    ..Transform::default()
                },
                opacity: 0.5,
                ..plain(frame).remove(0).look
            },
            ..plain(frame).remove(0)
        },
    ]
}

/// The mask: half the picture kept behind a soft edge.
///
/// Here rather than only in `mask_gpu` because that test asks whether the shape
/// is right, and this one asks whether the *export* draws the same shape as the
/// preview — which is a different question, and the one §46 is about.
fn masked(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![
        Layer {
            look: ClipLook {
                corner_pin: Default::default(),
                color: ColorAdjust {
                    brightness: 0.35,
                    ..ColorAdjust::default()
                },
                ..plain(frame).remove(0).look
            },
            ..plain(frame).remove(0)
        },
        Layer {
            look: ClipLook {
                corner_pin: Default::default(),
                mask: Some(Mask {
                    shape: MaskShape::Ellipse,
                    center: [0.45, 0.55],
                    size: [0.3, 0.22],
                    feather: 0.12,
                    rotation_degrees: 20.0,
                    invert: false,
                }),
                ..plain(frame).remove(0).look
            },
            ..plain(frame).remove(0)
        },
    ]
}

/// The chroma key over a graded ground: the key's edge is where a
/// colour-space mistake between the two configurations would show first.
fn keyed(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![
        Layer {
            look: ClipLook {
                corner_pin: Default::default(),
                color: ColorAdjust {
                    brightness: 0.5,
                    saturation: 1.4,
                    ..ColorAdjust::default()
                },
                ..plain(frame).remove(0).look
            },
            ..plain(frame).remove(0)
        },
        Layer {
            look: ClipLook {
                corner_pin: Default::default(),
                luma_key: None,
                chroma_key: Some(ChromaKey {
                    // Keyed off a colour the fixture actually contains, so the
                    // case has both kept and removed pixels in it.
                    color: [0.2, 0.5, 0.3],
                    tolerance: 0.18,
                    softness: 0.1,
                    spill: 0.5,
                }),
                ..plain(frame).remove(0).look
            },
            ..plain(frame).remove(0)
        },
    ]
}

/// §22's blend modes, two of them over one ground.
/// §25's flash, part-way through: a one-pixel white layer stretched over the
/// whole picture.
///
/// A generated layer is a shape nothing else in here has — every other case
/// composites a 256×256 decoded frame, and this one asks the sampler to spread
/// a single texel across the target. Edge sampling on a 1×1 texture is exactly
/// the sort of thing that can differ between two pipeline configurations, which
/// is what §51.1 is for.
///
/// Part-way rather than fully white: at full solidity the target is a flat
/// field, and a signature of flat white would be the same whatever went wrong
/// underneath it.
///
/// The target here is square, so covering it needs no correction — the case
/// that would catch a botched cover transform is in the playback crate, where
/// the frame can be any shape.
fn flashed(frame: &VideoFrame) -> Vec<Layer<'_>> {
    let white = flash_white();
    vec![
        plain(frame).remove(0),
        Layer {
            frame: white,
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
                smooth_skin: 0.0,
                vignette: 0.0,
                reflection: bettercut_timeline::Reflection::None,
                crop: bettercut_timeline::Crop::NONE,
                transform: flash_transform(white),
                opacity: 0.6,
                color: ColorAdjust::default(),
                blur: 0.0,
                chroma_key: None,
                luma_key: None,
                mask: None,
                blend: BlendMode::Normal,
            },
        },
    ]
}

/// Where the engine puts a generated layer, asked of the engine.
fn flash_transform(white: &VideoFrame) -> Transform {
    use bettercut_playback::{LayerRequest, LayerSource};

    let request = LayerRequest {
        angle: None,
        clip: bettercut_foundation::ClipId::new(),
        track: bettercut_foundation::TrackId::new(),
        source: LayerSource::Solid {
            rgb: [255, 255, 255],
        },
        source_time: bettercut_foundation::MediaTime::ZERO,
        look: bettercut_timeline::ClipLook {
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
            smooth_skin: 0.0,
            vignette: 0.0,
            reflection: bettercut_timeline::Reflection::None,
            crop: bettercut_timeline::Crop::NONE,
            transform: Transform::default(),
            opacity: 1.0,
            color: ColorAdjust::IDENTITY,
            blur: 0.0,
            chroma_key: None,
            luma_key: None,
            mask: None,
            blend: BlendMode::Normal,
        },
        reveal: None,
    };
    bettercut_playback::layer_transform(&request, white, Resolution::new(SIZE, SIZE))
}

/// Motion blur: the same picture at the places it passed through.
///
/// Built the way the engine builds it — three copies along a path, each at a
/// third of the opacity — so the case pins what a smear *looks* like, which the
/// engine's own tests cannot: they count layers and check the shares add up,
/// and a trail drawn in front of the picture instead of behind it would satisfy
/// both.
fn smeared(frame: &VideoFrame) -> Vec<Layer<'_>> {
    let plain = plain(frame).remove(0);
    (0..3)
        .map(|index| {
            let t = index as f32 / 2.0;
            Layer {
                look: ClipLook {
                    corner_pin: Default::default(),
                    transform: Transform {
                        position: Vec2::new(-0.18 + 0.18 * t, 0.0),
                        scale: Vec2::new(0.6, 0.6),
                        ..Transform::default()
                    },
                    opacity: 1.0 / 3.0,
                    ..plain.look
                },
                ..plain
            }
        })
        .collect()
}

fn blended(frame: &VideoFrame) -> Vec<Layer<'_>> {
    vec![
        Layer {
            look: ClipLook {
                corner_pin: Default::default(),
                color: ColorAdjust {
                    brightness: 0.45,
                    ..ColorAdjust::default()
                },
                ..plain(frame).remove(0).look
            },
            ..plain(frame).remove(0)
        },
        Layer {
            look: ClipLook {
                corner_pin: Default::default(),
                transform: Transform {
                    position: Vec2::new(-0.15, 0.0),
                    scale: Vec2::new(0.6, 0.6),
                    ..Transform::default()
                },
                opacity: 0.8,
                blend: BlendMode::Screen,
                ..plain(frame).remove(0).look
            },
            ..plain(frame).remove(0)
        },
        Layer {
            look: ClipLook {
                corner_pin: Default::default(),
                transform: Transform {
                    position: Vec2::new(0.15, 0.0),
                    scale: Vec2::new(0.6, 0.6),
                    ..Transform::default()
                },
                opacity: 0.7,
                blend: BlendMode::Multiply,
                ..plain(frame).remove(0).look
            },
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
            look: ClipLook {
                corner_pin: Default::default(),
                transform: Transform {
                    position: Vec2::new(-0.2, 0.15),
                    scale: Vec2::new(0.55, 0.55),
                    rotation_degrees: -15.0,
                    anchor: Vec2::new(0.5, 0.5),
                    flip_h: false,
                    flip_v: false,
                },
                opacity: 0.65,
                color: ColorAdjust {
                    brightness: 1.2,
                    contrast: 0.8,
                    saturation: 1.5,
                    temperature: -0.35,
                    tint: 0.2,
                    vibrance: 0.0,
                    wheels: bettercut_timeline::ColorWheels::IDENTITY,
                    secondary: bettercut_timeline::HslSecondary::IDENTITY,
                },
                blur: 25.0,
                ..plain(frame).remove(0).look
            },
            ..plain(frame).remove(0)
        },
    ]
}

/// ## One deliberate change to these signatures
///
/// `everything` moved when §22's blend modes landed. The composite shader now
/// emits **premultiplied** colour — it must, because screen, multiply and add
/// are blend states that each need the source already weighted by its alpha —
/// and that changes where the clamp falls.
///
/// Hardware clamps a fragment to the target's range *before* blending. The old
/// path clamped the graded colour to 1.0 and then scaled it by the layer's
/// opacity; the new one scales first and clamps after. They agree everywhere
/// except where a grade pushes a channel past 1.0 *and* the layer is partly
/// transparent — which is exactly this case, brightness 1.2 and saturation 1.5
/// at 65% opacity, where a red detail went from 0.61 to 0.83.
///
/// The new number is the better one: the contribution 1.4 × 0.65 = 0.91 is
/// representable, and the old path threw that range away.
///
/// `two_tracks` moved as well, by far less — up to about two 8-bit steps on a
/// layer at 50% opacity. That is the other side of premultiplying on an 8-bit
/// target: the shader's `colour × alpha` is quantised on write and *then*
/// blended, where straight alpha quantised the colour and did the multiply at
/// blend precision. It is a real loss of a fraction of a step, accepted
/// knowingly — the alternative is a float target for every composite, which
/// costs far more than it buys on §52.1's hardware.
///
/// Both signatures were regenerated once, on purpose, with that understood.
/// An adjustment layer over the lower of two tracks (§22): graded, softened
/// and at less than full strength, so the grade's colour, its blur and its
/// strength are all in the picture being compared. The upper track is drawn
/// above it and must come through ungraded.
const ADJUSTMENT: &[Grade] = &[Grade {
    beneath: 1,
    look: bettercut_timeline::AdjustmentLook {
        color: ColorAdjust {
            brightness: 0.8,
            contrast: 1.3,
            saturation: 0.4,
            temperature: -0.3,
            tint: 0.0,
            vibrance: 0.0,
            wheels: bettercut_timeline::ColorWheels::IDENTITY,
            secondary: bettercut_timeline::HslSecondary::IDENTITY,
        },
        blur: 12.0,
        strength: 0.85,
        // Left out here, so this case keeps the signature it was stored with;
        // the vignette has a case of its own below.
        vignette: 0.0,
        grain: 0.0,
    },
}];

/// A vignette on its own, strong enough that the corners the signature grid
/// samples are clearly darker than the middle.
const VIGNETTE: &[Grade] = &[Grade {
    beneath: 1,
    look: bettercut_timeline::AdjustmentLook {
        color: ColorAdjust {
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
            vibrance: 0.0,
            wheels: bettercut_timeline::ColorWheels::IDENTITY,
            secondary: bettercut_timeline::HslSecondary::IDENTITY,
        },
        blur: 0.0,
        strength: 1.0,
        vignette: 0.9,
        grain: 0.0,
    },
}];

/// Heavy grain on its own. The signature grid averages a whole cell, so the
/// noise mostly cancels in the stored numbers — what this pins is that preview
/// and export draw the same grain for the same frame (§46), pixel for pixel.
const GRAIN: &[Grade] = &[Grade {
    beneath: 1,
    look: bettercut_timeline::AdjustmentLook {
        color: ColorAdjust {
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
            vibrance: 0.0,
            wheels: bettercut_timeline::ColorWheels::IDENTITY,
            secondary: bettercut_timeline::HslSecondary::IDENTITY,
        },
        blur: 0.0,
        strength: 1.0,
        vignette: 0.0,
        grain: 1.0,
    },
}];

const CASES: &[(&str, Build, &[Grade])] = &[
    ("plain", plain, &[]),
    ("transform", transformed, &[]),
    ("mirrored", mirrored, &[]),
    ("cropped", cropped, &[]),
    ("colour", colour_graded, &[]),
    ("blur", blurred, &[]),
    ("sharpened", sharpened, &[]),
    ("two_tracks", two_tracks, &[]),
    ("everything", everything, &[]),
    ("masked", masked, &[]),
    ("keyed", keyed, &[]),
    ("blended", blended, &[]),
    ("flashed", flashed, &[]),
    ("smeared", smeared, &[]),
    ("adjusted", two_tracks, ADJUSTMENT),
    ("vignetted", plain, VIGNETTE),
    ("grained", plain, GRAIN),
    ("kaleidoscope", kaleidoscoped, &[]),
];

/// Render one case under one configuration and read it back as linear RGB.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    build: Build,
    grades: &[Grade],
    config: RenderConfig,
) -> Vec<[f32; 3]> {
    let mut compositor =
        Compositor::new(device.clone(), queue.clone(), config).expect("compositor");
    let frame = fixture_frame();
    compositor
        .composite_graded(&build(&frame), grades, MasterLook::default())
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
/// Each effect case has to actually *use* its effect.
///
/// A golden case that renders the same with the feature switched off passes
/// forever and proves nothing — it would sit in the suite looking like cover
/// for masks or keying while guarding neither. So each one is rendered again
/// with only that feature removed, and the two have to differ.
#[test]
fn each_effect_case_would_notice_losing_its_effect() {
    let (device, queue) = gpu_or_skip!();
    let resolution = Resolution::new(SIZE, SIZE);
    let config = RenderConfig::export_to_texture(resolution);

    let without_mask: Build = |frame| {
        let mut layers = masked(frame);
        layers[1].look.mask = None;
        layers
    };
    let without_key: Build = |frame| {
        let mut layers = keyed(frame);
        layers[1].look.chroma_key = None;
        layers
    };
    let without_blend: Build = |frame| {
        let mut layers = blended(frame);
        layers[1].look.blend = BlendMode::Normal;
        layers[2].look.blend = BlendMode::Normal;
        layers
    };

    for (name, with, without) in [
        ("masked", masked as Build, without_mask),
        ("keyed", keyed as Build, without_key),
        ("blended", blended as Build, without_blend),
        // Without its reflection, a kaleidoscope is the shot as it was.
        ("kaleidoscope", kaleidoscoped as Build, plain),
        // Without its white, a flash is just the shot.
        ("flashed", flashed as Build, plain),
        // And without its trail, a smear is one copy at the end of the path.
        ("smeared", smeared as Build, |frame| {
            vec![Layer {
                look: ClipLook {
                    corner_pin: Default::default(),
                    transform: Transform {
                        scale: Vec2::new(0.6, 0.6),
                        ..Transform::default()
                    },
                    ..plain(frame).remove(0).look
                },
                ..plain(frame).remove(0)
            }]
        }),
    ] {
        let on = render(&device, &queue, with, &[], config);
        let off = render(&device, &queue, without, &[], config);

        let moved = on
            .iter()
            .zip(&off)
            .map(|(a, b)| {
                (0..3)
                    .map(|channel| (a[channel] - b[channel]).abs())
                    .fold(0.0_f32, f32::max)
            })
            .fold(0.0_f32, f32::max);

        assert!(
            moved > 0.05,
            "the {name} case renders almost the same without its effect \
             (worst channel moved {moved:.4}), so it guards nothing"
        );
    }
}

#[test]
fn preview_and_export_render_the_same_picture() {
    let (device, queue) = gpu_or_skip!();
    let resolution = Resolution::new(SIZE, SIZE);

    for (name, build, grades) in CASES {
        let preview = render(
            &device,
            &queue,
            *build,
            grades,
            RenderConfig::preview(resolution),
        );
        let export = render(
            &device,
            &queue,
            *build,
            grades,
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

    for (name, build, grades) in CASES {
        let rendered = signature(&render(
            &device,
            &queue,
            *build,
            grades,
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
        .map(|(name, build, grades)| {
            (
                *name,
                signature(&render(
                    &device,
                    &queue,
                    *build,
                    grades,
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
