//! Reading one rendered pixel back (The eyedropper), and §22's background.
//!
//! The background is here because the readback is what makes it measurable:
//! "the colour in the letterbox" is a claim about one pixel in a corner.
//!
//! Keying starts by naming the screen's colour, and a colour wheel is the wrong
//! instrument for that — the answer is already on screen. This is how it gets
//! off the screen and into the key.
//!
//! Two things have to hold, and only one of them is obvious. The colour must be
//! the one that is actually there; and the *coordinates* must mean what the
//! caller thinks, because an x/y that quietly transposes would pick a plausible
//! wrong colour off a real shot and look like the key simply being bad.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Layer, RenderConfig};
use bettercut_timeline::{ClipLook, ColorAdjust, MasterLook, Resolution, Transform};

/// One GPU device for the whole binary, made the first time a test asks.
///
/// **Not an optimisation.** Each test used to make its own, and libtest runs
/// them on parallel threads, so thirteen devices were being created at once.
/// On this machine's driver that deadlocks: when the run hung, eleven of the
/// thirteen tests were stuck together — every thread waiting on one lock, not
/// one test being slow — and the two that finished were the ones that got a
/// device first. It hung the entire workspace run with no output, which is the
/// worst way a test can fail.
///
/// The editor itself has exactly one device, so a shared one is also the more
/// honest test: thirteen at once is a situation the product never reaches.
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
                label: Some("pixel read test"),
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

/// Deliberately not square, so a transposed read cannot pass by accident.
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

fn composite(compositor: &mut Compositor, shot: &VideoFrame) {
    compositor
        .composite(
            &[Layer {
                frame: shot,

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
            }],
            MasterLook::default(),
        )
        .expect("composite");
}

fn compositor(device: &wgpu::Device, queue: &wgpu::Queue) -> Compositor {
    Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(OUTPUT),
    )
    .expect("compositor")
}

/// Draw one shot with a given transform, over a blue frame so that "covered by
/// the clip" is a question the readback can answer.
fn composite_transform(compositor: &mut Compositor, shot: &VideoFrame, transform: Transform) {
    compositor
        .composite(
            &[Layer {
                frame: shot,
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
                    transform,
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                },
            }],
            MasterLook {
                background: [0.0, 0.0, 1.0],
                ..MasterLook::default()
            },
        )
        .expect("composite");
}

/// Red on the left half, green on the right. Which end is which is the whole
/// question a mirror asks.
fn two_halves() -> VideoFrame {
    frame(|x, _| {
        if x < OUTPUT.width / 2 {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        }
    })
}

/// What the clip covers along one row, as the first and last x that is not the
/// blue background, plus the colour at each end.
fn span(compositor: &mut Compositor, y: u32) -> (u32, u32, char, char) {
    let mut covered = Vec::new();
    for x in 0..OUTPUT.width {
        let pixel = compositor.read_pixel(x, y).expect("in frame");
        // Blue background, versus a red or green that has no blue in it.
        if pixel[2] < 0.5 {
            let which = if pixel[0] > pixel[1] { 'r' } else { 'g' };
            covered.push((x, which));
        }
    }
    assert!(!covered.is_empty(), "the clip covered nothing at all");
    let (first, left) = covered[0];
    let (last, right) = covered[covered.len() - 1];
    (first, last, left, right)
}

/// The mirror reverses the picture: what was on the left is on the right.
#[test]
fn a_mirrored_shot_shows_its_other_end_first() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let shot = two_halves();

    composite_transform(&mut compositor, &shot, Transform::default());
    let (_, _, left, right) = span(&mut compositor, OUTPUT.height / 2);
    assert_eq!(
        (left, right),
        ('r', 'g'),
        "the shot is not as it was painted"
    );

    composite_transform(
        &mut compositor,
        &shot,
        Transform {
            flip_h: true,
            ..Transform::default()
        },
    );
    let (_, _, left, right) = span(&mut compositor, OUTPUT.height / 2);
    assert_eq!(
        (left, right),
        ('g', 'r'),
        "mirroring left the picture the way round it was"
    );
}

/// **A mirror does not move the clip.** It changes which part of the source
/// lands where and nothing else, so the clip covers the identical region of the
/// frame before and after.
///
/// With the anchor off centre — a clip scaled about its left edge, which is
/// what dragging a corner handle gives — a mirror written as a negative scale
/// alone turns the quad about that anchor and throws the picture across the
/// frame. It still looks mirrored, so a test that only checked which end was
/// which would pass while the shot jumped sideways the moment the button was
/// pressed.
#[test]
fn a_mirror_leaves_the_clip_exactly_where_it_was() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let shot = two_halves();

    let placed = Transform {
        scale: bettercut_timeline::Vec2::new(0.4, 0.4),
        anchor: bettercut_timeline::Vec2::new(0.0, 0.0),
        position: bettercut_timeline::Vec2::new(-0.2, 0.0),
        ..Transform::default()
    };

    composite_transform(&mut compositor, &shot, placed);
    let (first, last, ..) = span(&mut compositor, OUTPUT.height / 2);

    composite_transform(
        &mut compositor,
        &shot,
        Transform {
            flip_h: true,
            ..placed
        },
    );
    let (mirrored_first, mirrored_last, left, right) = span(&mut compositor, OUTPUT.height / 2);

    assert_eq!(
        (first, last),
        (mirrored_first, mirrored_last),
        "the mirror moved the clip: it covered {first}..={last} and now covers \
         {mirrored_first}..={mirrored_last}"
    );
    assert_eq!(
        (left, right),
        ('g', 'r'),
        "the clip stayed put but was not mirrored"
    );
}

/// The vertical mirror is the same thing on the other axis, and worth its own
/// test because the two are separate flags written into separate terms — a
/// horizontal one that also flipped vertically would pass every test above.
#[test]
fn the_vertical_mirror_turns_the_picture_over_and_not_sideways() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    // Red on top, green below: a horizontal mirror cannot change this, and a
    // vertical one must.
    let shot = frame(|_, y| {
        if y < OUTPUT.height / 2 {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        }
    });

    let top_is = |compositor: &mut Compositor| {
        let pixel = compositor
            .read_pixel(OUTPUT.width / 2, 2)
            .expect("in frame");
        if pixel[0] > pixel[1] { 'r' } else { 'g' }
    };

    composite_transform(&mut compositor, &shot, Transform::default());
    assert_eq!(top_is(&mut compositor), 'r');

    composite_transform(
        &mut compositor,
        &shot,
        Transform {
            flip_h: true,
            ..Transform::default()
        },
    );
    assert_eq!(
        top_is(&mut compositor),
        'r',
        "a left-to-right mirror turned the picture over"
    );

    composite_transform(
        &mut compositor,
        &shot,
        Transform {
            flip_v: true,
            ..Transform::default()
        },
    );
    assert_eq!(
        top_is(&mut compositor),
        'g',
        "a top-to-bottom mirror left the picture the way up it was"
    );
}

/// Four quarters, four colours. Reading each one back must give that colour and
/// not its neighbour.
#[test]
fn each_quarter_reads_back_its_own_colour() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    const TOP_LEFT: [u8; 4] = [200, 30, 30, 255];
    const TOP_RIGHT: [u8; 4] = [30, 200, 30, 255];
    const BOTTOM_LEFT: [u8; 4] = [30, 30, 200, 255];
    const BOTTOM_RIGHT: [u8; 4] = [200, 200, 30, 255];

    composite(
        &mut compositor,
        &frame(|x, y| match (x < OUTPUT.width / 2, y < OUTPUT.height / 2) {
            (true, true) => TOP_LEFT,
            (false, true) => TOP_RIGHT,
            (true, false) => BOTTOM_LEFT,
            (false, false) => BOTTOM_RIGHT,
        }),
    );

    let close = |got: [f32; 3], want: [u8; 4], where_: &str| {
        for channel in 0..3 {
            let expected = f32::from(want[channel]) / 255.0;
            assert!(
                (got[channel] - expected).abs() < 0.02,
                "{where_}: read {got:?}, expected {want:?}"
            );
        }
    };

    let (w, h) = (OUTPUT.width, OUTPUT.height);
    close(
        compositor.read_pixel(4, 4).expect("in frame"),
        TOP_LEFT,
        "top left",
    );
    close(
        compositor.read_pixel(w - 5, 4).expect("in frame"),
        TOP_RIGHT,
        "top right",
    );
    close(
        compositor.read_pixel(4, h - 5).expect("in frame"),
        BOTTOM_LEFT,
        "bottom left",
    );
    close(
        compositor.read_pixel(w - 5, h - 5).expect("in frame"),
        BOTTOM_RIGHT,
        "bottom right",
    );
}

/// x is across and y is down, and the frame is twice as wide as it is tall —
/// so a read that swapped them would be off the frame in one direction and
/// wrong in the other.
#[test]
fn the_coordinates_are_x_across_and_y_down() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    // A single bright column near the right edge, nothing else.
    let column = OUTPUT.width - 3;
    composite(
        &mut compositor,
        &frame(move |x, _| {
            if x == column {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        }),
    );

    let on = compositor.read_pixel(column, 1).expect("in frame");
    assert!(on[0] > 0.9, "did not find the column at x={column}: {on:?}");

    let off = compositor.read_pixel(1, 1).expect("in frame");
    assert!(
        off[0] < 0.1,
        "found light where the frame is black: {off:?}"
    );
}

/// Off the frame is `None`, not a clamped edge pixel: a click outside the
/// picture has no colour to report, and reporting the nearest one would put a
/// key colour into the project that the user never pointed at.
#[test]
fn a_point_outside_the_frame_reads_nothing() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    composite(&mut compositor, &frame(|_, _| [10, 20, 30, 255]));

    assert!(compositor.read_pixel(OUTPUT.width, 0).is_none());
    assert!(compositor.read_pixel(0, OUTPUT.height).is_none());
    assert!(compositor.read_pixel(OUTPUT.width, OUTPUT.height).is_none());
    assert!(
        compositor
            .read_pixel(OUTPUT.width - 1, OUTPUT.height - 1)
            .is_some(),
        "the last pixel in the frame is inside it"
    );
}

/// The value the eyedropper hands to a key must be the value the key would
/// compare against — sRGB in 0..1, as `ChromaKey::color` holds. A green screen
/// read back and put straight into a key should match itself.
#[test]
fn a_screen_colour_reads_back_as_the_key_would_hold_it() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    const SCREEN: [u8; 4] = [0, 177, 64, 255];
    composite(&mut compositor, &frame(|_, _| SCREEN));

    let picked = compositor.read_pixel(10, 10).expect("in frame");
    let key = bettercut_timeline::ChromaKey {
        color: picked,
        ..bettercut_timeline::ChromaKey::default()
    }
    .clamped();

    assert_eq!(
        key.color, picked,
        "a colour picked off the frame was not one a key may hold"
    );
    for channel in 0..3 {
        let expected = f32::from(SCREEN[channel]) / 255.0;
        assert!(
            (picked[channel] - expected).abs() < 0.02,
            "picked {picked:?} for a screen of {SCREEN:?}"
        );
    }
}

/// §22: what shows where no picture does. Black for every project that has
/// never asked for anything else, and the reason vertical footage in a
/// landscape frame need not be surrounded by black bars.
#[test]
fn the_background_shows_where_no_picture_does() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    // A shot covering the middle third, so there is frame left over on either
    // side for the background to show in.
    let shot = frame(|_, _| [20, 20, 20, 255]);
    let master = MasterLook {
        background: [1.0, 0.0, 0.0],
        ..MasterLook::default()
    };
    compositor
        .composite(
            &[Layer {
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
                    transform: Transform {
                        scale: bettercut_timeline::Vec2::new(0.3, 0.3),
                        ..Transform::default()
                    },
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                },
            }],
            master,
        )
        .expect("composite");

    let corner = compositor.read_pixel(1, 1).expect("in frame");
    assert!(
        corner[0] > 0.9 && corner[1] < 0.1 && corner[2] < 0.1,
        "the letterbox was not the background colour: {corner:?}"
    );

    let middle = compositor
        .read_pixel(OUTPUT.width / 2, OUTPUT.height / 2)
        .expect("in frame");
    assert!(
        middle[0] < 0.3,
        "the background covered the shot as well: {middle:?}"
    );
}

/// Black by default, which is what every project had before the colour
/// existed — and what `serde(default)` gives one written back then.
#[test]
fn the_background_is_black_unless_asked_otherwise() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    let shot = frame(|_, _| [200, 200, 200, 255]);
    compositor
        .composite(
            &[Layer {
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
                    transform: Transform {
                        scale: bettercut_timeline::Vec2::new(0.3, 0.3),
                        ..Transform::default()
                    },
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                },
            }],
            MasterLook::default(),
        )
        .expect("composite");

    let corner = compositor.read_pixel(1, 1).expect("in frame");
    assert!(
        corner.iter().all(|channel| *channel < 0.02),
        "the default background is not black: {corner:?}"
    );
}

/// The colour is stated in sRGB, as the rest of the model states colours, and
/// the target encodes on write — so a value handed to wgpu unconverted comes
/// out visibly pale. Mid-grey is where that shows worst.
#[test]
fn the_background_is_the_srgb_colour_it_was_given() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    let shot = frame(|_, _| [0, 0, 0, 255]);
    compositor
        .composite(
            &[Layer {
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
                    transform: Transform {
                        scale: bettercut_timeline::Vec2::new(0.1, 0.1),
                        ..Transform::default()
                    },
                    opacity: 1.0,
                    color: ColorAdjust::default(),
                    blur: 0.0,
                    chroma_key: None,
                    luma_key: None,
                    mask: None,
                    blend: bettercut_timeline::BlendMode::Normal,
                },
            }],
            MasterLook {
                background: [0.5, 0.5, 0.5],
                ..MasterLook::default()
            },
        )
        .expect("composite");

    let corner = compositor.read_pixel(1, 1).expect("in frame");
    for channel in corner {
        assert!(
            (channel - 0.5).abs() < 0.02,
            "asked for sRGB 0.5 and got {corner:?} — the conversion is wrong in one direction or missing"
        );
    }
}

/// §45's white balance, measured rather than eyeballed.
///
/// A neutral grey is the only honest test subject: it has no colour of its own,
/// so any difference between the channels afterwards is the control's doing and
/// nothing else. On a coloured patch a wrong coefficient hides inside the
/// colour that was already there.
mod white_balance {
    use super::*;

    fn graded(compositor: &mut Compositor, color: ColorAdjust) -> [f32; 3] {
        let grey = frame(|_, _| [128, 128, 128, 255]);
        compositor
            .composite(
                &[Layer {
                    frame: &grey,
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
                        color,
                        blur: 0.0,
                        chroma_key: None,
                        luma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                }],
                MasterLook::default(),
            )
            .expect("composite");
        compositor.read_pixel(8, 8).expect("in frame")
    }

    /// Warm is more red than blue, cool is the other way round, and neither
    /// touches a picture at zero.
    #[test]
    fn temperature_moves_red_against_blue() {
        let (device, queue) = gpu_or_skip!();
        let mut compositor = compositor(&device, &queue);

        let neutral = graded(&mut compositor, ColorAdjust::default());
        assert!(
            (neutral[0] - neutral[2]).abs() < 0.01,
            "a grey came out coloured with every control at its default: {neutral:?}"
        );

        let warm = graded(
            &mut compositor,
            ColorAdjust {
                temperature: 1.0,
                ..ColorAdjust::default()
            },
        );
        assert!(
            warm[0] > warm[2] + 0.1,
            "warm did not put more red in than blue: {warm:?}"
        );

        let cool = graded(
            &mut compositor,
            ColorAdjust {
                temperature: -1.0,
                ..ColorAdjust::default()
            },
        );
        assert!(
            cool[2] > cool[0] + 0.1,
            "cool did not put more blue in than red: {cool:?}"
        );
    }

    /// Tint is the other axis: green against the two that make magenta. It has
    /// to be genuinely perpendicular — a tint implemented as a second
    /// temperature would pass "it changed the picture" and be useless.
    #[test]
    fn tint_moves_green_against_red_and_blue() {
        let (device, queue) = gpu_or_skip!();
        let mut compositor = compositor(&device, &queue);

        let magenta = graded(
            &mut compositor,
            ColorAdjust {
                tint: 1.0,
                ..ColorAdjust::default()
            },
        );
        assert!(
            magenta[0] > magenta[1] + 0.1 && magenta[2] > magenta[1] + 0.1,
            "positive tint did not go towards magenta: {magenta:?}"
        );
        assert!(
            (magenta[0] - magenta[2]).abs() < 0.02,
            "tint leaned warm or cool instead of staying on its own axis: {magenta:?}"
        );

        let green = graded(
            &mut compositor,
            ColorAdjust {
                tint: -1.0,
                ..ColorAdjust::default()
            },
        );
        // A smaller margin than the magenta side above, and not arbitrarily:
        // this axis moves one channel against two, so going green lifts green
        // by 30% while going magenta lifts red *and* blue by 15% each and drops
        // green by 30%. The separation here is about 0.1 in sRGB terms — far
        // past noise, and far past what a swapped channel or a wrong sign would
        // leave, which is what this is actually asking about.
        assert!(
            green[1] > green[0] + 0.05 && green[1] > green[2] + 0.05,
            "negative tint did not go towards green: {green:?}"
        );
    }

    /// Black stays black. The white balance is a gain, not an offset, and the
    /// difference shows exactly here: an additive warm shift would lift every
    /// shadow in the shot to orange, which is the classic way a colour control
    /// ruins a picture while looking right on a mid-grey.
    #[test]
    fn the_white_balance_leaves_black_alone() {
        let (device, queue) = gpu_or_skip!();
        let mut compositor = compositor(&device, &queue);

        let black = frame(|_, _| [0, 0, 0, 255]);
        compositor
            .composite(
                &[Layer {
                    frame: &black,
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
                        color: ColorAdjust {
                            temperature: 1.0,
                            tint: 1.0,
                            ..ColorAdjust::default()
                        },
                        blur: 0.0,
                        chroma_key: None,
                        luma_key: None,
                        mask: None,
                        blend: bettercut_timeline::BlendMode::Normal,
                    },
                }],
                MasterLook::default(),
            )
            .expect("composite");

        let pixel = compositor.read_pixel(8, 8).expect("in frame");
        assert!(
            pixel.iter().all(|channel| *channel < 0.02),
            "the white balance lifted black off the floor: {pixel:?}"
        );
    }
}

/// The kept frame is the frame that was composited when it was kept — what a
/// side-by-side comparison draws on the other side of the divider.
#[test]
fn a_kept_frame_survives_the_next_composite() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    // A red frame, kept; then a green one composited over the top.
    let red = MasterLook {
        background: [1.0, 0.0, 0.0],
        ..MasterLook::default()
    };
    let green = MasterLook {
        background: [0.0, 1.0, 0.0],
        ..MasterLook::default()
    };
    compositor.composite(&[], red).expect("composite");
    compositor.keep_snapshot().expect("keep");
    compositor.composite(&[], green).expect("composite");

    // The live frame is the green one.
    let live = compositor.read_pixel(4, 4).expect("in frame");
    assert!(live[1] > 0.5 && live[0] < 0.5, "the live frame is {live:?}");
    // And the kept one is still there, its own size, ready to be drawn.
    let kept = compositor.snapshot_texture().expect("a kept frame");
    assert_eq!(kept.size(), compositor.target().size());
    assert!(compositor.snapshot_view().is_some());
}
