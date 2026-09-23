//! Colour match: a clip's grade chosen so it looks like a reference frame.
//!
//! # What is matched
//!
//! Two shots of the same scene from different cameras rarely agree: one is
//! darker, flatter, warmer. Matching looks at the few things a viewer notices
//! first — how bright the picture is, how far its tones spread, its colour
//! cast, and how colourful it is — and picks the [`ColorAdjust`] that brings
//! the clip's ungraded frame to the reference's figures.
//!
//! # How
//!
//! Every figure is measured in linear light, where the compositor grades, and
//! the grade is found by applying a copy of the compositor's own colour maths
//! ([`graded`]) to a sample of the clip's pixels and nudging each control
//! towards the reference until the figures agree. Solving against the real
//! formula rather than an idealised one means the clamp at black and the
//! saturation mix are accounted for, not hoped away. Each control stays
//! within the range its Inspector slider offers, so a match never produces a
//! grade the user cannot see or undo by hand.
//!
//! Only statistics are compared, not pixels, so the reference can be any
//! frame: another angle, another shot, a still.

use crate::clip::ColorAdjust;

/// The most pixels a sample keeps. Enough that the statistics are stable, few
/// enough that solving is instant.
const MAX_SAMPLES: usize = 96 * 54;

/// Linear mid-grey, the pivot the compositor's contrast expands around.
const MID_GREY: f32 = 0.18;

/// Rec.709 luma weights for linear RGB.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The ranges the Inspector's colour sliders offer.
const BRIGHTNESS: (f32, f32) = (0.0, 2.0);
const CONTRAST: (f32, f32) = (0.0, 2.0);
const SATURATION: (f32, f32) = (0.0, 2.0);
const BALANCE: (f32, f32) = (-1.0, 1.0);

/// A frame's pixels in linear light, thinned to at most [`MAX_SAMPLES`].
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSample {
    pixels: Vec<[f32; 3]>,
}

/// The figures a match compares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    /// Average luma.
    pub luma: f32,
    /// How far luma spreads around its average: the standard deviation.
    pub spread: f32,
    /// Warm against cool: the log ratio of average red to average blue.
    pub warmth: f32,
    /// Green against magenta: the log ratio of average green to the average
    /// of red and blue.
    pub greenness: f32,
    /// How colourful: the average distance of a pixel from its own grey.
    pub colourfulness: f32,
}

/// sRGB-encoded 8-bit to linear light.
fn linear(byte: u8) -> f32 {
    let v = f32::from(byte) / 255.0;
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn luma(rgb: [f32; 3]) -> f32 {
    rgb[0] * LUMA[0] + rgb[1] * LUMA[1] + rgb[2] * LUMA[2]
}

impl FrameSample {
    /// Sample a frame of sRGB-encoded RGBA rows, `width` by `height`, on an
    /// even grid. Transparent pixels are left out: they are not part of the
    /// picture. `None` when the rows do not match the size or nothing opaque
    /// is left.
    pub fn from_rgba(rgba: &[u8], width: u32, height: u32) -> Option<Self> {
        let (width, height) = (width as usize, height as usize);
        if width == 0 || height == 0 || rgba.len() < width * height * 4 {
            return None;
        }
        // The same step across and down, keeping the frame's shape.
        let step = ((width * height) as f64 / MAX_SAMPLES as f64)
            .sqrt()
            .ceil()
            .max(1.0) as usize;
        let mut pixels = Vec::with_capacity(MAX_SAMPLES);
        for y in (step / 2..height).step_by(step) {
            for x in (step / 2..width).step_by(step) {
                let i = (y * width + x) * 4;
                if rgba[i + 3] < 128 {
                    continue;
                }
                pixels.push([linear(rgba[i]), linear(rgba[i + 1]), linear(rgba[i + 2])]);
            }
        }
        Self::from_linear(pixels)
    }

    /// A sample of pixels already in linear light. `None` when empty.
    pub fn from_linear(pixels: Vec<[f32; 3]>) -> Option<Self> {
        (!pixels.is_empty()).then_some(Self { pixels })
    }

    pub fn pixels(&self) -> &[[f32; 3]] {
        &self.pixels
    }

    /// The figures for this sample as it is.
    pub fn stats(&self) -> FrameStats {
        stats_of(self.pixels.iter().copied())
    }

    /// The figures for this sample once `grade` is applied.
    pub fn stats_graded(&self, grade: &ColorAdjust) -> FrameStats {
        stats_of(self.pixels.iter().map(|p| graded(*p, grade)))
    }
}

fn stats_of(pixels: impl Iterator<Item = [f32; 3]>) -> FrameStats {
    // Summed in f64: thousands of small values lose precision in f32.
    let (mut n, mut sum, mut sum_sq, mut colour) = (0.0_f64, [0.0_f64; 3], 0.0_f64, 0.0_f64);
    let mut lumas = 0.0_f64;
    for p in pixels {
        let l = f64::from(luma(p));
        n += 1.0;
        for k in 0..3 {
            sum[k] += f64::from(p[k]);
        }
        lumas += l;
        sum_sq += l * l;
        colour += p
            .iter()
            .map(|c| (f64::from(*c) - l).powi(2))
            .sum::<f64>()
            .sqrt();
    }
    if n == 0.0 {
        return FrameStats {
            luma: 0.0,
            spread: 0.0,
            warmth: 0.0,
            greenness: 0.0,
            colourfulness: 0.0,
        };
    }
    // A floor under the averages, so a black or single-channel frame gives a
    // finite ratio rather than a division by zero.
    const FLOOR: f64 = 1e-4;
    let mean = sum.map(|s| (s / n).max(FLOOR));
    let luma = lumas / n;
    FrameStats {
        luma: luma as f32,
        spread: (sum_sq / n - luma * luma).max(0.0).sqrt() as f32,
        warmth: (mean[0] / mean[2]).ln() as f32,
        greenness: (mean[1] / ((mean[0] + mean[2]) / 2.0)).ln() as f32,
        colourfulness: (colour / n) as f32,
    }
}

/// One linear pixel through a grade — the compositor's `adjust_colour`,
/// step for step: exposure, white balance, contrast about mid-grey,
/// saturation, then the clamp at black.
pub fn graded(rgb: [f32; 3], grade: &ColorAdjust) -> [f32; 3] {
    let (warm, magenta) = (
        grade.temperature.clamp(-1.0, 1.0),
        grade.tint.clamp(-1.0, 1.0),
    );
    let balance = [
        1.0 + 0.30 * warm + 0.15 * magenta,
        1.0 - 0.30 * magenta,
        1.0 - 0.30 * warm + 0.15 * magenta,
    ];
    let mut out = [0.0_f32; 3];
    for k in 0..3 {
        out[k] = ((rgb[k] * grade.brightness * balance[k]) - MID_GREY) * grade.contrast + MID_GREY;
    }
    let l = luma(out);
    out.map(|c| (l + (c - l) * grade.saturation).max(0.0))
}

/// The grade that brings `clip` — an ungraded frame of the clip — closest to
/// the figures of `reference`.
///
/// Solved by repeated small corrections against [`graded`], each control held
/// to its slider's range. A reference with no colour to speak of (a black or
/// grey frame) leaves the clip's colourfulness alone rather than draining it.
/// What a well-exposed, neutrally balanced frame measures: the mid-grey
/// convention (18% reflectance) for the average, a spread that fills the
/// range without crushing either end, and no cast either way. Colourfulness
/// is not here — it is the shot's own, and "auto" must not drain a sunset.
pub const NEUTRAL_LUMA: f32 = 0.18;
pub const NEUTRAL_SPREAD: f32 = 0.16;

/// The grade that brings `clip` to a neutral exposure and balance
/// ([`NEUTRAL_LUMA`], [`NEUTRAL_SPREAD`], no warmth, no green), keeping its
/// own colourfulness: what an "auto" button does. The same solver as a
/// match, aimed at a frame nobody shot.
pub fn auto_grade(clip: &FrameSample) -> ColorAdjust {
    let own = clip.stats();
    let neutral = FrameStats {
        luma: NEUTRAL_LUMA,
        spread: NEUTRAL_SPREAD,
        warmth: 0.0,
        greenness: 0.0,
        colourfulness: own.colourfulness,
    };
    // Held back from the extremes: an "auto" that slams a control to its
    // end has not judged the shot, it has given up on it. Halfway is as far
    // as it goes; the rest is a person's call.
    let mut grade = match_grade(clip, &neutral);
    grade.brightness = grade.brightness.clamp(0.5, 2.0);
    grade.contrast = grade.contrast.clamp(0.6, 1.6);
    grade.temperature = grade.temperature.clamp(-0.5, 0.5);
    grade.tint = grade.tint.clamp(-0.5, 0.5);
    grade
}

pub fn match_grade(clip: &FrameSample, reference: &FrameStats) -> ColorAdjust {
    let mut grade = ColorAdjust::default();
    let clip_stats = clip.stats();
    let clamp = |v: f32, (lo, hi): (f32, f32)| {
        if v.is_finite() {
            v.clamp(lo, hi)
        } else {
            lo.max(0.0).min(hi)
        }
    };

    for _ in 0..40 {
        // Spread first: contrast moves brightness too, so brightness follows.
        let now = clip.stats_graded(&grade);
        if now.spread > 1e-4 && reference.spread > 1e-4 {
            grade.contrast = clamp(
                grade.contrast * (reference.spread / now.spread).powf(0.7),
                CONTRAST,
            );
        }

        let now = clip.stats_graded(&grade);
        if now.luma > 1e-4 && reference.luma > 1e-4 {
            grade.brightness = clamp(
                grade.brightness * (reference.luma / now.luma).powf(0.7),
                BRIGHTNESS,
            );
        }

        // The white balance: each log ratio moves about this much per unit of
        // its control at the neutral point (0.6 and 0.45 from the gains above).
        let now = clip.stats_graded(&grade);
        grade.temperature = clamp(
            grade.temperature + 0.7 * (reference.warmth - now.warmth) / 0.6,
            BALANCE,
        );
        grade.tint = clamp(
            grade.tint - 0.7 * (reference.greenness - now.greenness) / 0.45,
            BALANCE,
        );

        let now = clip.stats_graded(&grade);
        if clip_stats.colourfulness > 1e-3
            && reference.colourfulness > 1e-3
            && now.colourfulness > 1e-4
        {
            grade.saturation = clamp(
                grade.saturation * (reference.colourfulness / now.colourfulness).powf(0.7),
                SATURATION,
            );
        }
    }
    grade
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spread of colours and tones, as a real frame has.
    fn scene() -> FrameSample {
        let mut pixels = Vec::new();
        for i in 0..40 {
            for j in 0..30 {
                let t = i as f32 / 39.0;
                let u = j as f32 / 29.0;
                pixels.push([
                    0.02 + 0.6 * t * (0.5 + 0.5 * u),
                    0.03 + 0.5 * t * (1.0 - 0.4 * u),
                    0.02 + 0.45 * (1.0 - t) * u + 0.1 * t,
                ]);
            }
        }
        FrameSample::from_linear(pixels).unwrap()
    }

    fn close(a: FrameStats, b: FrameStats, tolerance: f32) -> bool {
        (a.luma - b.luma).abs() <= tolerance * b.luma.max(0.01)
            && (a.spread - b.spread).abs() <= tolerance * b.spread.max(0.01)
            && (a.warmth - b.warmth).abs() <= tolerance
            && (a.greenness - b.greenness).abs() <= tolerance
            && (a.colourfulness - b.colourfulness).abs() <= tolerance * b.colourfulness.max(0.01)
    }

    /// A dark, warm frame comes up and cools; a frame already neutral is
    /// left nearly alone; either keeps its own colourfulness.
    #[test]
    fn auto_grade_brings_a_frame_to_neutral_without_draining_it() {
        let scene = scene();
        let dim_warm = FrameSample::from_linear(
            scene
                .pixels()
                .iter()
                .map(|p| [p[0] * 0.5, p[1] * 0.45, p[2] * 0.36])
                .collect(),
        )
        .unwrap();
        let grade = auto_grade(&dim_warm);
        assert!(grade.brightness > 1.0, "not brightened: {grade:?}");
        assert!(grade.temperature < 0.0, "not cooled: {grade:?}");
        let before = dim_warm.stats();
        let after = dim_warm.stats_graded(&grade);
        assert!((after.luma - NEUTRAL_LUMA).abs() < 0.05, "{after:?}");
        assert!(
            after.warmth.abs() < before.warmth.abs(),
            "the cast was not reduced: before {before:?}, after {after:?}"
        );
        assert!(
            (after.colourfulness - before.colourfulness).abs()
                < 0.3 * before.colourfulness.max(0.01),
            "colourfulness changed: before {before:?}, after {after:?}"
        );

        // A neutral frame is already there: the solver barely moves it.
        let neutral = FrameSample::from_linear(
            scene
                .pixels()
                .iter()
                .map(|p| {
                    let g = luma(*p);
                    [g, g, g]
                })
                .collect(),
        )
        .unwrap();
        let stats = neutral.stats();
        let grade = auto_grade(&neutral);
        assert!(
            grade.temperature.abs() < 0.05 && grade.tint.abs() < 0.05,
            "a grey frame was given a cast: {grade:?} from {stats:?}"
        );
    }

    #[test]
    fn a_frame_matched_to_itself_needs_no_grade() {
        let clip = scene();
        let grade = match_grade(&clip, &clip.stats());
        let neutral = ColorAdjust::default();
        assert!(
            (grade.brightness - neutral.brightness).abs() < 0.02,
            "{grade:?}"
        );
        assert!(
            (grade.contrast - neutral.contrast).abs() < 0.02,
            "{grade:?}"
        );
        assert!(
            (grade.saturation - neutral.saturation).abs() < 0.02,
            "{grade:?}"
        );
        assert!(
            grade.temperature.abs() < 0.02 && grade.tint.abs() < 0.02,
            "{grade:?}"
        );
    }

    /// The same scene graded warmer, darker, punchier and less colourful:
    /// matching the ungraded scene to it lands on its figures, and on a grade
    /// that leans the same way on every control.
    #[test]
    fn a_known_grade_is_found_again() {
        let clip = scene();
        let known = ColorAdjust {
            brightness: 0.8,
            contrast: 1.3,
            saturation: 0.7,
            temperature: 0.4,
            tint: -0.2,
            vibrance: 0.0,
            wheels: Default::default(),
            secondary: crate::clip::HslSecondary::IDENTITY,
        };
        let reference = clip.stats_graded(&known);
        let grade = match_grade(&clip, &reference);
        assert!(
            close(clip.stats_graded(&grade), reference, 0.03),
            "{grade:?}"
        );
        assert!(
            (grade.brightness - known.brightness).abs() < 0.1,
            "{grade:?}"
        );
        assert!((grade.contrast - known.contrast).abs() < 0.1, "{grade:?}");
        assert!(
            (grade.saturation - known.saturation).abs() < 0.1,
            "{grade:?}"
        );
        assert!(
            (grade.temperature - known.temperature).abs() < 0.1,
            "{grade:?}"
        );
        assert!((grade.tint - known.tint).abs() < 0.1, "{grade:?}");
    }

    /// A reference beyond what the sliders reach gets the slider's end, never
    /// a value past it.
    #[test]
    fn a_match_stays_within_the_sliders() {
        let clip = scene();
        let blazing = FrameStats {
            luma: 50.0,
            spread: 40.0,
            warmth: 9.0,
            greenness: -9.0,
            colourfulness: 30.0,
        };
        let grade = match_grade(&clip, &blazing);
        assert!(
            grade.brightness <= 2.0 && grade.contrast <= 2.0 && grade.saturation <= 2.0,
            "{grade:?}"
        );
        assert!(
            grade.temperature <= 1.0 && grade.tint >= -1.0 && grade.tint <= 1.0,
            "{grade:?}"
        );
        assert_eq!(grade.temperature, 1.0);
    }

    /// A grey reference has no colour to copy; the clip keeps its own.
    #[test]
    fn a_grey_reference_does_not_drain_the_colour() {
        let clip = scene();
        let grey = FrameSample::from_linear(vec![[0.2, 0.2, 0.2]; 10]).unwrap();
        let grade = match_grade(&clip, &grey.stats());
        assert!((grade.saturation - 1.0).abs() < 1e-6, "{grade:?}");
    }

    /// sRGB rows decode to linear light, skipping transparent pixels, and a
    /// big frame is thinned to the sample limit.
    #[test]
    fn rows_are_sampled_in_linear_light() {
        let mut rgba = vec![0_u8; 4 * 4 * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[188, 188, 188, if i == 0 { 0 } else { 255 }]);
        }
        let sample = FrameSample::from_rgba(&rgba, 4, 4).unwrap();
        assert_eq!(sample.pixels().len(), 15, "the transparent pixel was kept");
        // sRGB 188 is about linear 0.5.
        assert!(
            (sample.pixels()[0][0] - 0.5).abs() < 0.01,
            "{:?}",
            sample.pixels()[0]
        );

        let big = vec![255_u8; 1920 * 1080 * 4];
        let sample = FrameSample::from_rgba(&big, 1920, 1080).unwrap();
        assert!(sample.pixels().len() <= MAX_SAMPLES && sample.pixels().len() > MAX_SAMPLES / 2);
        assert!(FrameSample::from_rgba(&big[..100], 1920, 1080).is_none());
    }

    #[test]
    fn graded_matches_the_neutral_grade() {
        let p = [0.3, 0.1, 0.7];
        let out = graded(p, &ColorAdjust::default());
        for k in 0..3 {
            assert!((out[k] - p[k]).abs() < 1e-6);
        }
    }
}
