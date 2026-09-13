//! §22's crop, which runs *before* the transform.
//!
//! That ordering is the whole of what these check. The difference between a
//! crop and a rectangular mask is not visible in "is the edge gone" — both
//! remove it. It is visible in what happens to what is left: a mask leaves the
//! shot the size it was with a hole in it, while a crop makes the remainder a
//! new picture, of a new shape, and fits *that* to the frame.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ClipLook, ColorAdjust, Crop, MasterLook, Resolution, Transform};

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
                label: Some("crop test"),
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

/// Twice as wide as it is tall, and the source is the same shape — so an
/// uncropped clip fills the frame exactly and any letterboxing seen later is
/// the crop's doing.
const OUTPUT: Resolution = Resolution {
    width: 64,
    height: 32,
};

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

fn compositor(device: &wgpu::Device, queue: &wgpu::Queue) -> Compositor {
    Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor")
}

/// Four quarters, so which part of the source survived is readable from one
/// pixel. Four rather than two: with a left/right split alone, an edge that
/// cropped from the wrong side would pass by symmetry.
fn quarters() -> VideoFrame {
    frame(|x, y| match (x < OUTPUT.width / 2, y < OUTPUT.height / 2) {
        (true, true) => [255, 0, 0, 255],     // top left: red
        (false, true) => [0, 255, 0, 255],    // top right: green
        (true, false) => [0, 0, 255, 255],    // bottom left: blue
        (false, false) => [255, 255, 0, 255], // bottom right: yellow
    })
}

fn draw(compositor: &mut Compositor, shot: &VideoFrame, crop: Crop) {
    compositor
        .composite(
            &[Layer {
                frame: shot,
                look: ClipLook {
                    sharpen: 0.0,
                    lut: None,
                    rgb_split: 0.0,
                    glitch: 0.0,
                    reflection: bettercut_timeline::Reflection::None,
                    crop,
                    transform: Transform::default(),
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    chroma_key: None,
                    mask: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                },
            }],
            MasterLook::default(),
        )
        .expect("composite");
}

fn which(pixel: [f32; 3]) -> &'static str {
    let bright = |v: f32| v > 0.5;
    match (bright(pixel[0]), bright(pixel[1]), bright(pixel[2])) {
        (true, false, false) => "red",
        (false, true, false) => "green",
        (false, false, true) => "blue",
        (true, true, false) => "yellow",
        _ => "something else",
    }
}

/// Cropping away the left half leaves the right half, re-centred — and the
/// left half is gone from the frame entirely.
///
/// Both halves of that matter. A mask would leave green and yellow where they
/// already were, in the right of the frame; a crop moves them to the middle.
/// And a transform that merely slid the shot across would put red and blue
/// off-screen without removing them, which the sweep below rules out.
///
/// It does not *fill* the frame, and should not: half of a 2:1 source is a
/// square, and a square fitted to a 2:1 frame letterboxes. That is the crop
/// working — `the_remainder_is_fitted_on_its_own_shape` is where it is stated.
#[test]
fn a_crop_leaves_only_what_it_kept() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let shot = quarters();

    draw(
        &mut compositor,
        &shot,
        Crop {
            left: 0.5,
            ..Crop::NONE
        },
    );

    let top = compositor
        .read_pixel(OUTPUT.width / 2, 2)
        .expect("in frame");
    assert_eq!(
        which(top),
        "green",
        "the kept half is not on screen: {top:?}"
    );
    let bottom = compositor
        .read_pixel(OUTPUT.width / 2, OUTPUT.height - 3)
        .expect("in frame");
    assert_eq!(which(bottom), "yellow");

    // Nothing from the cropped-away half survives anywhere in the frame.
    for x in 0..OUTPUT.width {
        for y in [1, OUTPUT.height / 2, OUTPUT.height - 2] {
            let pixel = compositor.read_pixel(x, y).expect("in frame");
            let colour = which(pixel);
            assert!(
                colour != "red" && colour != "blue",
                "the cropped-away half is still on screen at ({x}, {y}): {colour}"
            );
        }
    }
}

/// Each edge takes from its own side.
#[test]
fn each_edge_takes_from_its_own_side() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let shot = quarters();

    let middle = |compositor: &mut Compositor| {
        which(
            compositor
                .read_pixel(OUTPUT.width / 2, OUTPUT.height / 2)
                .expect("in frame"),
        )
    };

    // Everything but the top-left quarter taken away.
    draw(
        &mut compositor,
        &shot,
        Crop {
            right: 0.5,
            bottom: 0.5,
            ..Crop::NONE
        },
    );
    assert_eq!(
        middle(&mut compositor),
        "red",
        "cropping right and bottom did not leave the top-left quarter"
    );

    // And the opposite pair leaves the opposite quarter.
    draw(
        &mut compositor,
        &shot,
        Crop {
            left: 0.5,
            top: 0.5,
            ..Crop::NONE
        },
    );
    assert_eq!(
        middle(&mut compositor),
        "yellow",
        "cropping left and top did not leave the bottom-right quarter"
    );
}

/// A crop changes the picture's **shape**, so what is left is fitted to the
/// frame on its own terms.
///
/// The source and the frame are both 2:1, so uncropped the picture fills
/// exactly. Crop it to a centre column and the remainder is tall and narrow: it
/// must letterbox at the sides. Fitting the *source's* aspect instead — which
/// is what a rectangular mask leaves — keeps it full-width, and this fails.
#[test]
fn the_remainder_is_fitted_on_its_own_shape() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let shot = frame(|_, _| [255, 255, 255, 255]);

    draw(&mut compositor, &shot, Crop::NONE);
    let edge_before = compositor
        .read_pixel(1, OUTPUT.height / 2)
        .expect("in frame");
    assert!(
        edge_before[0] > 0.9,
        "the fixture does not fill the frame uncropped, so the test below \
         proves nothing: {edge_before:?}"
    );

    draw(
        &mut compositor,
        &shot,
        Crop {
            left: 0.4,
            right: 0.4,
            ..Crop::NONE
        },
    );

    let middle = compositor
        .read_pixel(OUTPUT.width / 2, OUTPUT.height / 2)
        .expect("in frame");
    assert!(middle[0] > 0.9, "the picture left the middle of the frame");

    let edge = compositor
        .read_pixel(1, OUTPUT.height / 2)
        .expect("in frame");
    assert!(
        edge[0] < 0.1,
        "a crop to a narrow column still filled the width, so the shape of \
         what was left was ignored: {edge:?}"
    );
}

/// The mask stays over the part of the shot it was drawn on.
///
/// The mask is stated in the clip's own frame, and after a crop that frame is
/// what the crop left — not the original source. Sharing one set of coordinates
/// between the sampling window and the mask would shrink every existing mask
/// into a corner the moment a clip was cropped.
#[test]
fn a_mask_is_measured_against_what_the_crop_left() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let shot = frame(|_, _| [255, 255, 255, 255]);

    // A *small* ellipse over the middle of the visible picture. Small on
    // purpose: after cropping half the width away, the middle of what is left
    // sits at 0.75 in the source. A generous mask would still contain that
    // point, and the test would pass whichever set of coordinates the mask was
    // measured in — proving nothing.
    let mask = bettercut_timeline::Mask {
        shape: bettercut_timeline::MaskShape::Ellipse,
        center: [0.5, 0.5],
        size: [0.12, 0.12],
        ..bettercut_timeline::Mask::default()
    };

    for crop in [
        Crop::NONE,
        Crop {
            left: 0.5,
            ..Crop::NONE
        },
    ] {
        compositor
            .composite(
                &[Layer {
                    frame: &shot,
                    look: ClipLook {
                        sharpen: 0.0,
                        lut: None,
                        rgb_split: 0.0,
                        glitch: 0.0,
                        reflection: bettercut_timeline::Reflection::None,
                        crop,
                        transform: Transform::default(),
                        opacity: 1.0,
                        color: ColorAdjust::default(),
                        blur: 0.0,
                        chroma_key: None,
                        mask: Some(mask),
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                }],
                MasterLook::default(),
            )
            .expect("composite");

        let centre = compositor
            .read_pixel(OUTPUT.width / 2, OUTPUT.height / 2)
            .expect("in frame");
        assert!(
            centre[0] > 0.9,
            "the mask left the middle of the picture with crop {crop:?}: {centre:?}"
        );
    }
}

/// A crop that asks for everything is refused before it reaches the GPU.
///
/// A zero-sized quad is not a picture and cannot be dragged back out of — the
/// same trap the lower bound on scale exists for.
#[test]
fn a_crop_cannot_take_the_whole_picture() {
    let (across, down) = Crop {
        left: 0.8,
        right: 0.8,
        top: 0.9,
        bottom: 0.9,
    }
    .remaining();

    assert!(
        across >= bettercut_timeline::MIN_CROP_REMAINING - 1e-6,
        "nothing left across: {across}"
    );
    assert!(
        down >= bettercut_timeline::MIN_CROP_REMAINING - 1e-6,
        "nothing left down: {down}"
    );
}

/// The excess comes off both edges evenly, so an over-wide crop read from a
/// file leaves the surviving picture centred rather than sliding it to
/// whichever edge happened to be clamped second.
#[test]
fn clamping_an_impossible_crop_does_not_slide_the_picture() {
    let crop = Crop {
        left: 0.8,
        right: 0.8,
        ..Crop::NONE
    }
    .clamped();

    assert!(
        (crop.left - crop.right).abs() < 1e-6,
        "the clamp slid the picture sideways: {crop:?}"
    );
}

/// §50: a project file can carry anything, including values that are not
/// numbers. A NaN edge must not become a NaN sampling window.
#[test]
fn a_crop_that_is_not_a_number_is_no_crop() {
    let crop = Crop {
        left: f32::NAN,
        top: f32::INFINITY,
        right: -3.0,
        bottom: f32::NEG_INFINITY,
    }
    .clamped();

    assert!(
        [crop.left, crop.top, crop.right, crop.bottom]
            .iter()
            .all(|edge| edge.is_finite() && (0.0..=1.0).contains(edge)),
        "a nonsense crop survived into the sampling window: {crop:?}"
    );
}
