//! Grades part-way up the stack: adjustment layers, as the renderer draws them.
//!
//! The claim that makes an adjustment layer what it is: it grades the picture
//! **beneath** it and leaves everything drawn above alone. A grade that reached
//! the titles on top would be a master grade with a time range, and a grade
//! that reached nothing would be a control that does nothing — both look like
//! "something happened" on a casual glance, so each is measured.
//!
//! Skips itself when no adapter can be found, for the §52.1 reasons given in
//! `blur_gpu.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_renderer::wgpu;
use bettercut_renderer::{Compositor, Grade, Layer, RenderConfig};
use bettercut_timeline::AdjustmentLook;
use bettercut_timeline::{ClipLook, ColorAdjust, Crop, MasterLook, Resolution, Transform, Vec2};

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
                label: Some("grade test"),
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
    height: 32,
};

fn frame(width: u32, height: u32, paint: impl Fn(u32, u32) -> [u8; 4]) -> VideoFrame {
    let mut data = vec![0_u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let at = ((y * width + x) * 4) as usize;
            data[at..at + 4].copy_from_slice(&paint(x, y));
        }
    }
    VideoFrame {
        timestamp: bettercut_foundation::MediaTime::ZERO,
        width,
        height,
        color: ColorMetadata::srgb(),
        storage: FrameStorage::System {
            data,
            stride: width * 4,
        },
    }
}

fn look(transform: Transform, blur: f32) -> ClipLook {
    ClipLook {
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
        vignette: 0.0,
        reflection: bettercut_timeline::Reflection::None,
        crop: Crop::NONE,
        transform,
        opacity: 1.0,
        color: ColorAdjust::default(),
        blur,
        chroma_key: None,
        mask: None,
        blend: bettercut_timeline::BlendMode::Normal,
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

/// A grade that darkens hard, so "was it graded" is a large difference.
fn darken(beneath: usize) -> Grade {
    Grade {
        beneath,
        look: AdjustmentLook {
            color: ColorAdjust {
                brightness: 0.2,
                ..ColorAdjust::default()
            },
            blur: 0.0,
            strength: 1.0,
            vignette: 0.0,
            grain: 0.0,
        },
    }
}

/// `darken` at a different strength.
fn darken_at(beneath: usize, strength: f32) -> Grade {
    let grade = darken(beneath);
    Grade {
        look: AdjustmentLook {
            strength,
            ..grade.look
        },
        ..grade
    }
}

/// Mid-grey across the whole frame: the footage.
fn footage() -> VideoFrame {
    frame(OUTPUT.width, OUTPUT.height, |_, _| [180, 180, 180, 255])
}

/// A small white patch: the title on top. Placed in the frame's left quarter.
fn title() -> VideoFrame {
    frame(OUTPUT.width, OUTPUT.height, |_, _| [255, 255, 255, 255])
}

fn title_transform() -> Transform {
    Transform {
        scale: Vec2::new(0.25, 0.25),
        position: Vec2::new(-0.3, 0.0),
        ..Transform::default()
    }
}

/// Where the title sits, and a point of footage well clear of it.
fn title_pixel() -> (u32, u32) {
    // position -0.3 of the frame width left of centre.
    ((OUTPUT.width as f32 * 0.2) as u32, OUTPUT.height / 2)
}
fn footage_pixel() -> (u32, u32) {
    (OUTPUT.width - 6, OUTPUT.height / 2)
}

/// **The defining claim.** Footage beneath the grade is darkened; the title
/// drawn above it is not.
#[test]
fn a_grade_darkens_what_is_beneath_it_and_not_what_is_above() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let footage = footage();
    let title = title();

    let layers = [
        Layer {
            frame: &footage,
            look: look(Transform::default(), 0.0),
        },
        Layer {
            frame: &title,
            look: look(title_transform(), 0.0),
        },
    ];

    compositor
        .composite_graded(&layers, &[darken(1)], MasterLook::default())
        .expect("composite");

    let (fx, fy) = footage_pixel();
    let footage_after = compositor.read_pixel(fx, fy).expect("in frame");
    let (tx, ty) = title_pixel();
    let title_after = compositor.read_pixel(tx, ty).expect("in frame");

    assert!(
        footage_after[0] < 0.4,
        "the footage beneath the grade was not darkened: {footage_after:?}"
    );
    assert!(
        title_after[0] > 0.95,
        "the title above the grade was darkened too: {title_after:?}"
    );
}

/// And a grade above everything grades everything, titles included — which is
/// what makes the position the thing that decides, rather than a special case
/// for titles.
#[test]
fn a_grade_at_the_top_grades_the_whole_stack() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let footage = footage();
    let title = title();

    let layers = [
        Layer {
            frame: &footage,
            look: look(Transform::default(), 0.0),
        },
        Layer {
            frame: &title,
            look: look(title_transform(), 0.0),
        },
    ];
    compositor
        .composite_graded(&layers, &[darken(2)], MasterLook::default())
        .expect("composite");

    let (tx, ty) = title_pixel();
    let title_after = compositor.read_pixel(tx, ty).expect("in frame");
    assert!(
        title_after[0] < 0.5,
        "a grade over the whole stack left the title untouched: {title_after:?}"
    );
}

/// Strength is a blend: half strength lands between the ungraded picture and
/// the fully graded one, not at either end.
#[test]
fn half_strength_is_half_the_grade() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let footage = footage();
    let layers = [Layer {
        frame: &footage,
        look: look(Transform::default(), 0.0),
    }];
    let (x, y) = footage_pixel();

    let mut read = |grades: &[Grade]| {
        compositor
            .composite_graded(&layers, grades, MasterLook::default())
            .expect("composite");
        compositor.read_pixel(x, y).expect("in frame")[0]
    };

    let untouched = read(&[]);
    let full = read(&[darken(1)]);
    let half = read(&[darken_at(1, 0.5)]);

    assert!(
        half < untouched - 0.05 && half > full + 0.05,
        "half strength did not land between the two: ungraded {untouched}, half {half}, full {full}"
    );
}

/// A grade that changes nothing draws exactly what no grade draws — so an
/// adjustment clip left at its defaults costs the picture nothing.
#[test]
fn a_grade_that_changes_nothing_changes_nothing() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let footage = footage();
    let layers = [Layer {
        frame: &footage,
        look: look(Transform::default(), 0.0),
    }];
    let (x, y) = footage_pixel();

    compositor
        .composite_graded(&layers, &[], MasterLook::default())
        .expect("composite");
    let without = compositor.read_pixel(x, y).expect("in frame");

    for idle in [
        darken_at(1, 0.0),
        Grade {
            beneath: 1,
            look: AdjustmentLook::default(),
        },
    ] {
        compositor
            .composite_graded(&layers, &[idle], MasterLook::default())
            .expect("composite");
        let with = compositor.read_pixel(x, y).expect("in frame");
        assert_eq!(with, without, "an idle grade {idle:?} changed the picture");
    }
}

/// Two grades stack, in order: each works on what the one before left.
#[test]
fn two_grades_stack() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let footage = footage();
    let layers = [Layer {
        frame: &footage,
        look: look(Transform::default(), 0.0),
    }];
    let (x, y) = footage_pixel();

    let mut read = |grades: &[Grade]| {
        compositor
            .composite_graded(&layers, grades, MasterLook::default())
            .expect("composite");
        compositor.read_pixel(x, y).expect("in frame")[0]
    };
    let one = read(&[darken(1)]);
    let two = read(&[darken(1), darken(1)]);
    assert!(
        two < one - 0.01,
        "a second grade did nothing on top of the first: one {one}, two {two}"
    );
}

/// A blurred grade blurs, even with a blurred clip beneath it and a blurred
/// master above — the three chains do not interfere.
///
/// Not the slot-exhaustion case, though it was first written as one: three
/// chains fit comfortably in the blur node's smallest buffer whatever count it
/// is given, and undercounting passed here. The test below is sized on the
/// boundary where a miscount actually drops a blur.
#[test]
fn a_blurred_grade_still_blurs_among_other_blurs() {
    let (device, queue) = gpu_or_skip!();

    // Large enough for a blur to spread across real texels. At the 64×32 the
    // other tests use, a few percent of the frame is under one texel and every
    // reading here came back exactly zero — which proved nothing either way.
    const SIZE: u32 = 256;
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(Resolution::new(SIZE, SIZE)),
    )
    .expect("compositor");

    // A hard vertical edge: black on the left, white on the right.
    let edge = frame(SIZE, SIZE, |x, _| {
        if x < SIZE / 2 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    // Three texels left of the edge: black while sharp, lifted once blur
    // spreads the white that far. Blur 40 reaches about six texels at this
    // size, so twelve — where this first probed — is beyond every blur here.
    let probe = (SIZE / 2 - 3, SIZE / 2);

    let mut read = |layer_blur: f32, grade_blur: f32, master_blur: f32| {
        let layers = [Layer {
            frame: &edge,
            look: look(Transform::default(), layer_blur),
        }];
        let grade = Grade {
            beneath: 1,
            look: AdjustmentLook {
                blur: grade_blur,
                ..AdjustmentLook::default()
            },
        };
        let master = MasterLook {
            blur: master_blur,
            ..MasterLook::default()
        };
        compositor
            .composite_graded(&layers, &[grade], master)
            .expect("composite");
        compositor.read_pixel(probe.0, probe.1).expect("in frame")[0]
    };

    // First, that the probe can see a blur at all — a grade's blur alone must
    // reach it. Without this, the comparison below could pass or fail on a
    // probe that nothing ever reaches.
    let sharp = read(0.0, 0.0, 0.0);
    let grade_only = read(0.0, 40.0, 0.0);
    assert!(
        grade_only > sharp + 0.02,
        "a grade's blur on its own did not reach the probe: sharp {sharp}, blurred {grade_only}"
    );

    // Then the case that runs out of slots: a blurred clip, a blurred grade and
    // a blurred master in the same frame. The clip and master blurs are large
    // enough to run chains of their own — a blur too small to matter is skipped
    // before it takes a slot, and would leave nothing to run out of.
    let without = read(20.0, 0.0, 20.0);
    let with = read(20.0, 40.0, 20.0);
    assert!(
        with > without + 0.02,
        "the grade's blur did nothing among the other blurs: without {without}, with {with}"
    );
}

/// Every blur in a frame runs, when grades take slots of their own.
///
/// The blur node sizes its uniform buffer once per frame from the number of
/// chains it is told to expect, rounded up to a power of two from 16 passes —
/// eight chains. Tell it about the clips and the master but not the grades, and
/// a frame with seven blurred clips, a blurred grade and a blurred master asks
/// for a ninth: the last chain to run, the master's, is skipped with only a
/// warning in a log.
///
/// Sized exactly on that boundary. Below it every chain fits whatever the count
/// says, which is why the smaller test above could not tell.
#[test]
fn every_blur_in_the_frame_runs_when_grades_take_slots() {
    let (device, queue) = gpu_or_skip!();
    const SIZE: u32 = 256;
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(Resolution::new(SIZE, SIZE)),
    )
    .expect("compositor");

    let edge = frame(SIZE, SIZE, |x, _| {
        if x < SIZE / 2 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    // Seven blurred clips: each runs a chain of its own.
    let layers: Vec<Layer<'_>> = (0..7)
        .map(|_| Layer {
            frame: &edge,
            look: look(Transform::default(), 20.0),
        })
        .collect();
    let grade = Grade {
        beneath: layers.len(),
        look: AdjustmentLook {
            blur: 20.0,
            ..AdjustmentLook::default()
        },
    };
    let probe = (SIZE / 2 - 4, SIZE / 2);

    let mut read = |master_blur: f32| {
        let master = MasterLook {
            blur: master_blur,
            ..MasterLook::default()
        };
        compositor
            .composite_graded(&layers, &[grade], master)
            .expect("composite");
        compositor.read_pixel(probe.0, probe.1).expect("in frame")[0]
    };

    let without = read(0.0);
    let with = read(60.0);
    assert!(
        with > without + 0.02,
        "the master's blur was skipped once a grade took a slot: without {without}, with {with}"
    );
}

// ---- vignette ---------------------------------------------------------------

/// A vignette darkens the corners and leaves the middle alone — through the
/// master grade and through an adjustment, the two things that can carry one.
///
/// Both halves are asserted, because each alone passes on a wrong shader: one
/// that dimmed the whole frame darkens the corners too, and one that did
/// nothing leaves the middle alone too.
#[test]
fn a_vignette_darkens_the_corners_and_not_the_middle() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let footage = footage();
    let layers = [Layer {
        frame: &footage,
        look: look(Transform::default(), 0.0),
    }];
    let corner = (1, 1);
    let middle = (OUTPUT.width / 2, OUTPUT.height / 2);

    compositor
        .composite_graded(&layers, &[], MasterLook::default())
        .expect("composite");
    let plain_corner = compositor.read_pixel(corner.0, corner.1).expect("in frame")[0];
    let plain_middle = compositor.read_pixel(middle.0, middle.1).expect("in frame")[0];

    let through_master = MasterLook {
        vignette: 1.0,
        grain: 0.0,
        ..MasterLook::default()
    };
    let through_adjustment = [Grade {
        beneath: 1,
        look: AdjustmentLook {
            vignette: 1.0,
            grain: 0.0,
            ..AdjustmentLook::default()
        },
    }];

    for (route, grades, master) in [
        ("master", &[][..], through_master),
        ("adjustment", &through_adjustment[..], MasterLook::default()),
    ] {
        compositor
            .composite_graded(&layers, grades, master)
            .expect("composite");
        let dark_corner = compositor.read_pixel(corner.0, corner.1).expect("in frame")[0];
        let kept_middle = compositor.read_pixel(middle.0, middle.1).expect("in frame")[0];

        assert!(
            dark_corner < plain_corner - 0.2,
            "{route}: the corner was not darkened: {plain_corner} -> {dark_corner}"
        );
        assert!(
            (kept_middle - plain_middle).abs() < 0.02,
            "{route}: the middle was darkened too: {plain_middle} -> {kept_middle}"
        );
    }
}

/// No vignette is no change, anywhere in the frame — so a project that never
/// asked for one draws exactly as it did before vignettes existed.
#[test]
fn no_vignette_changes_nothing() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let footage = footage();
    let layers = [Layer {
        frame: &footage,
        look: look(Transform::default(), 0.0),
    }];

    let mut read_all = |master: MasterLook| {
        compositor
            .composite_graded(&layers, &[], master)
            .expect("composite");
        [(1, 1), (OUTPUT.width - 2, OUTPUT.height - 2), (32, 16)]
            .map(|(x, y)| compositor.read_pixel(x, y).expect("in frame"))
    };
    // A master that forces the full-frame pass, so the vignette term in that
    // draw is really exercised at zero rather than skipped with the pass.
    let with_pass = MasterLook {
        color: ColorAdjust {
            brightness: 1.0001,
            ..ColorAdjust::default()
        },
        ..MasterLook::default()
    };
    let without = read_all(MasterLook::default());
    let with_zero = read_all(with_pass);
    for (a, b) in without.iter().zip(&with_zero) {
        assert!(
            (a[0] - b[0]).abs() < 0.01,
            "a zero vignette changed the picture: {a:?} vs {b:?}"
        );
    }
}

/// A clip's own vignette darkens the corners of its picture and leaves the
/// middle alone; without one, corner and middle match.
#[test]
fn a_clip_vignette_darkens_its_own_corners() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);
    let grey = frame(OUTPUT.width, OUTPUT.height, |_, _| [180, 180, 180, 255]);

    let mut render = |vignette: f32| {
        let look = ClipLook {
            corner_pin: Default::default(),
            vignette,
            ..look(Transform::default(), 0.0)
        };
        compositor
            .composite(&[Layer { frame: &grey, look }], MasterLook::default())
            .expect("composite");
        (
            compositor.read_pixel(1, 1).unwrap()[0],
            compositor
                .read_pixel(OUTPUT.width / 2, OUTPUT.height / 2)
                .unwrap()[0],
        )
    };
    let (plain_corner, plain_middle) = render(0.0);
    assert!(
        (plain_corner - plain_middle).abs() < 0.01,
        "setup: an even frame is not even"
    );
    let (corner, middle) = render(1.0);
    assert!(
        corner < plain_corner * 0.6,
        "the corner was not darkened: {corner} of {plain_corner}"
    );
    assert!(
        (middle - plain_middle).abs() < 0.02,
        "the middle changed: {middle} of {plain_middle}"
    );
}

/// **Vibrance's defining claim.** It pushes the colours that have least of it
/// hardest: a nearly grey patch gains far more saturation than an already
/// vivid one, which is what keeps skin from going orange while a flat sky
/// comes back.
#[test]
fn vibrance_lifts_a_dull_colour_further_than_a_vivid_one() {
    let (device, queue) = gpu_or_skip!();
    let mut compositor = compositor(&device, &queue);

    // Left half nearly grey with a red lean, right half strongly red.
    let picture = frame(OUTPUT.width, OUTPUT.height, |x, _| {
        if x < OUTPUT.width / 2 {
            [150, 130, 130, 255]
        } else {
            [230, 40, 40, 255]
        }
    });
    let dull = (OUTPUT.width / 4, OUTPUT.height / 2);
    let vivid = (OUTPUT.width * 3 / 4, OUTPUT.height / 2);

    // How far each half's colour spreads, with and without vibrance.
    let spread = |compositor: &Compositor, at: (u32, u32)| {
        let pixel = compositor.read_pixel(at.0, at.1).expect("in frame");
        let high = pixel[0].max(pixel[1]).max(pixel[2]);
        let low = pixel[0].min(pixel[1]).min(pixel[2]);
        high - low
    };

    let plain = look(Transform::default(), 0.0);
    compositor
        .composite(
            &[Layer {
                frame: &picture,
                look: plain,
            }],
            MasterLook::default(),
        )
        .expect("composite");
    let (dull_before, vivid_before) = (spread(&compositor, dull), spread(&compositor, vivid));

    let mut vibrant = plain;
    vibrant.color.vibrance = 1.0;
    compositor
        .composite(
            &[Layer {
                frame: &picture,
                look: vibrant,
            }],
            MasterLook::default(),
        )
        .expect("composite");
    let (dull_after, vivid_after) = (spread(&compositor, dull), spread(&compositor, vivid));

    assert!(
        dull_after > dull_before * 1.2,
        "the dull half gained almost nothing: {dull_before} → {dull_after}"
    );
    // In proportion to what each already had: a vivid half has so much colour
    // that even a small push is a large number, and it is the proportion that
    // says which one vibrance favoured.
    assert!(
        dull_after / dull_before > vivid_after / vivid_before,
        "the vivid half gained proportionally as much as the dull one: \
         dull {dull_before} → {dull_after}, vivid {vivid_before} → {vivid_after}"
    );
}
