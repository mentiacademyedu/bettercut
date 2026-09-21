//! Colour curves: the grade a colourist draws.
//!
//! Each curve maps a brightness in to a brightness out, from the shadows at
//! the left to the highlights at the right. The master curve moves all three
//! channels; red, green and blue each move their own on top. Five handles a
//! curve, at fixed steps across the range, dragged up or down — enough for an
//! S-curve, a lifted black or a cooled highlight, and simple enough to use on a
//! trackpad.
//!
//! # Drawn through the LUT node
//!
//! A curve is a colour lookup, so it is turned into one: the clip's curves,
//! composed after any `.cube` file the clip already uses, are baked into a
//! generated table and run through the same effect-graph node a file LUT
//! does. Preview and export draw the same table (§46), and there is no second
//! colour path to keep in step with the first.
//!
//! A generated table's id comes from the curves themselves (and the file LUT
//! under them), so the same curves are the same table and a changed curve is a
//! new one. Those ids exist only while the program runs; the project saves the
//! handles, never a table.

use serde::{Deserialize, Serialize};

use bettercut_foundation::LutId;

use crate::lut::{ClipLut, CubeLut};

/// Handles on a curve.
pub const HANDLES: usize = 5;

/// Samples along each axis of a generated table: fine enough that a smooth
/// curve stays smooth through trilinear lookup.
pub const CURVE_LUT_SIZE: u32 = 17;

/// One curve: the output at each of [`HANDLES`] evenly spaced inputs, 0–1.
pub type Curve = [f32; HANDLES];

/// The straight line: every brightness out as it came in.
pub const STRAIGHT: Curve = [0.0, 0.25, 0.5, 0.75, 1.0];

/// A clip's four curves.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColourCurves {
    #[serde(default = "straight")]
    pub master: Curve,
    #[serde(default = "straight")]
    pub red: Curve,
    #[serde(default = "straight")]
    pub green: Curve,
    #[serde(default = "straight")]
    pub blue: Curve,
    /// A colour tone laid over the result — sepia, or a duotone — baked into
    /// the same table, so it costs nothing more to draw.
    #[serde(default)]
    pub tone: Tone,
    /// Colour wheels for the shadows, midtones and highlights, applied ahead
    /// of the curves.
    #[serde(default)]
    pub wheels: ColourWheels,
    /// One colour range at a time: the reds redder, the blues of a sky
    /// deeper, the greens moved towards yellow. After the wheels, before the
    /// curves.
    #[serde(default)]
    pub mixer: ColourMixer,
}

/// The colour ranges the mixer works on, round the colour wheel.
pub const MIXER_BANDS: [(&str, f32); 8] = [
    ("Red", 0.0),
    ("Orange", 30.0),
    ("Yellow", 60.0),
    ("Green", 120.0),
    ("Aqua", 180.0),
    ("Blue", 225.0),
    ("Purple", 270.0),
    ("Magenta", 315.0),
];

/// Hue, saturation and lightness for one colour range, each -1–1.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct MixerBand {
    /// Towards the next colour round the wheel (up to 30° either way).
    #[serde(default)]
    pub hue: f32,
    #[serde(default)]
    pub saturation: f32,
    #[serde(default)]
    pub lightness: f32,
}

/// Every range's adjustment, in [`MIXER_BANDS`] order.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ColourMixer {
    #[serde(default)]
    pub bands: [MixerBand; 8],
}

impl ColourMixer {
    pub fn is_neutral(&self) -> bool {
        self.bands
            .iter()
            .all(|b| b.hue.abs() < 1e-4 && b.saturation.abs() < 1e-4 && b.lightness.abs() < 1e-4)
    }

    pub fn clamped(self) -> Self {
        let hold = |v: f32| {
            if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        };
        Self {
            bands: self.bands.map(|b| MixerBand {
                hue: hold(b.hue),
                saturation: hold(b.saturation),
                lightness: hold(b.lightness),
            }),
        }
    }

    /// One display-encoded colour through the mixer. Each range reaches its
    /// neighbours with a soft falloff, and a colour counts for as much as it
    /// is saturated — a grey belongs to no range and is left alone.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        if self.is_neutral() {
            return rgb;
        }
        let (h, s, l) = to_hsl(rgb);
        if s < 1e-4 {
            return rgb;
        }
        let (mut dh, mut ds, mut dl, mut total) = (0.0, 0.0, 0.0, 0.0);
        for (band, (_, centre)) in self.bands.iter().zip(MIXER_BANDS) {
            let mut distance = (h - centre).abs() % 360.0;
            if distance > 180.0 {
                distance = 360.0 - distance;
            }
            // Full at the band's own hue, nothing 45° away.
            let weight = (1.0 - distance / 45.0).max(0.0);
            let weight = weight * weight * (3.0 - 2.0 * weight);
            dh += band.hue * weight;
            ds += band.saturation * weight;
            dl += band.lightness * weight;
            total += weight;
        }
        if total <= 0.0 {
            return rgb;
        }
        let presence = s.min(1.0);
        let hue = (h + dh * 30.0 * presence).rem_euclid(360.0);
        let saturation = (s * (1.0 + ds * presence)).clamp(0.0, 1.0);
        let lightness = (l + dl * 0.25 * presence).clamp(0.0, 1.0);
        from_hsl(hue, saturation, lightness)
    }
}

/// RGB (0–1) as hue in degrees, saturation and lightness (0–1).
fn to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let [r, g, b] = rgb.map(|v| v.clamp(0.0, 1.0));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6);
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s.min(1.0), l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m].map(|v| v.clamp(0.0, 1.0))
}

/// A colourist's three wheels: a colour pushed into the shadows, the
/// midtones and the highlights, each also brighter or darker.
///
/// Each is an offset per channel, -1–1, what a wheel's dot and its brightness
/// slider come to. Zero everywhere is no change.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ColourWheels {
    #[serde(default)]
    pub shadows: [f32; 3],
    #[serde(default)]
    pub midtones: [f32; 3],
    #[serde(default)]
    pub highlights: [f32; 3],
}

impl ColourWheels {
    /// Whether all three wheels are centred: nothing to draw.
    pub fn is_neutral(&self) -> bool {
        [self.shadows, self.midtones, self.highlights]
            .iter()
            .flatten()
            .all(|v| v.abs() < 1e-4)
    }

    /// Every offset held to -1–1; not-a-number is none.
    pub fn clamped(self) -> Self {
        let hold = |wheel: [f32; 3]| {
            wheel.map(|v| {
                if v.is_finite() {
                    v.clamp(-1.0, 1.0)
                } else {
                    0.0
                }
            })
        };
        Self {
            shadows: hold(self.shadows),
            midtones: hold(self.midtones),
            highlights: hold(self.highlights),
        }
    }

    /// One display-encoded colour through the wheels, as lift, gamma and gain:
    /// the shadows raise or lower the black end, the highlights scale the
    /// white end, and the midtones bend the middle.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut out = rgb;
        for channel in 0..3 {
            let x = rgb[channel].clamp(0.0, 1.0);
            let lift = self.shadows[channel] * 0.25;
            let gain = 1.0 + self.highlights[channel] * 0.5;
            let gamma = (-self.midtones[channel] * 0.8).exp();
            let lifted = (x * gain + lift * (1.0 - x)).clamp(0.0, 1.0);
            out[channel] = lifted.powf(gamma).clamp(0.0, 1.0);
        }
        out
    }

    /// The offsets a wheel's dot and brightness make: the dot at `angle`
    /// (radians, 0 is red) and `distance` (0–1) from the middle pushes towards
    /// that hue without changing brightness, and `brightness` (-1–1) moves all
    /// three channels together.
    pub fn offsets(angle: f32, distance: f32, brightness: f32) -> [f32; 3] {
        let hue = |shift: f32| (angle - shift).cos();
        let third = std::f32::consts::TAU / 3.0;
        let push = distance.clamp(0.0, 1.0);
        [
            hue(0.0) * push + brightness,
            hue(third) * push + brightness,
            hue(2.0 * third) * push + brightness,
        ]
        .map(|v| v.clamp(-1.0, 1.0))
    }

    /// The other way: a wheel's offsets as its dot's angle and distance, and
    /// its brightness.
    pub fn position(offsets: [f32; 3]) -> (f32, f32, f32) {
        let brightness = (offsets[0] + offsets[1] + offsets[2]) / 3.0;
        let [r, g, b] = offsets.map(|v| v - brightness);
        // The cosines of three hues a third apart, back to a vector.
        let x = r - 0.5 * (g + b);
        let y = (3.0_f32).sqrt() / 2.0 * (g - b);
        let angle = y.atan2(x);
        let distance = (x * x + y * y).sqrt() / 1.5;
        (angle, distance.min(1.0), brightness)
    }
}

/// A whole-picture colour tone: the picture's light and dark remapped to a
/// fixed colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Tone {
    #[default]
    None,
    /// The warm brown of an old photograph.
    Sepia,
    /// Two colours only: shadows in one, highlights in the other, the rest
    /// between — the poster look.
    Duotone { shadow: [u8; 3], highlight: [u8; 3] },
}

impl Tone {
    /// A deep blue into a hot pink: a duotone that reads as one at a glance.
    pub const DUOTONE: Self = Self::Duotone {
        shadow: [30, 20, 90],
        highlight: [255, 110, 150],
    };

    pub fn is_none(self) -> bool {
        self == Self::None
    }

    /// One display-encoded colour in this tone.
    pub fn apply(self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            Self::None => rgb,
            // The classic sepia mix: every channel from all three, warm.
            Self::Sepia => {
                let [r, g, b] = rgb;
                [
                    (0.393 * r + 0.769 * g + 0.189 * b).min(1.0),
                    (0.349 * r + 0.686 * g + 0.168 * b).min(1.0),
                    (0.272 * r + 0.534 * g + 0.131 * b).min(1.0),
                ]
            }
            Self::Duotone { shadow, highlight } => {
                let luma = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]).clamp(0.0, 1.0);
                let mix = |a: u8, b: u8| {
                    let (a, b) = (f32::from(a) / 255.0, f32::from(b) / 255.0);
                    a + (b - a) * luma
                };
                [
                    mix(shadow[0], highlight[0]),
                    mix(shadow[1], highlight[1]),
                    mix(shadow[2], highlight[2]),
                ]
            }
        }
    }
}

fn straight() -> Curve {
    STRAIGHT
}

impl Default for ColourCurves {
    fn default() -> Self {
        Self {
            master: STRAIGHT,
            red: STRAIGHT,
            green: STRAIGHT,
            blue: STRAIGHT,
            tone: Tone::None,
            wheels: ColourWheels::default(),
            mixer: ColourMixer::default(),
        }
    }
}

/// A curve's value at `x`, 0–1: a smooth Catmull-Rom line through the
/// handles, held to 0–1.
pub fn sample(curve: &Curve, x: f32) -> f32 {
    let x = if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let last = (HANDLES - 1) as f32;
    let at = x * last;
    let i = (at.floor() as usize).min(HANDLES - 2);
    let t = at - i as f32;
    let point = |k: isize| -> f32 {
        // Past the ends, the line carries straight on.
        if k < 0 {
            2.0 * curve[0] - curve[1]
        } else if k as usize >= HANDLES {
            2.0 * curve[HANDLES - 1] - curve[HANDLES - 2]
        } else {
            curve[k as usize]
        }
    };
    let i = i as isize;
    let (p0, p1, p2, p3) = (point(i - 1), point(i), point(i + 1), point(i + 2));
    let t2 = t * t;
    let t3 = t2 * t;
    let value = 0.5
        * ((2.0 * p1)
            + (-p0 + p2) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
            + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3);
    value.clamp(0.0, 1.0)
}

impl ColourCurves {
    /// Every handle held to 0–1; not-a-number becomes the straight line's
    /// value there.
    pub fn clamped(self) -> Self {
        let hold = |curve: Curve| {
            let mut out = curve;
            for (i, v) in out.iter_mut().enumerate() {
                *v = if v.is_finite() {
                    v.clamp(0.0, 1.0)
                } else {
                    STRAIGHT[i]
                };
            }
            out
        };
        Self {
            master: hold(self.master),
            red: hold(self.red),
            green: hold(self.green),
            blue: hold(self.blue),
            tone: self.tone,
            wheels: self.wheels.clamped(),
            mixer: self.mixer.clamped(),
        }
    }

    /// Whether all four curves are straight and there is no tone: nothing to
    /// draw.
    pub fn is_identity(&self) -> bool {
        self.is_straight()
            && self.tone.is_none()
            && self.wheels.is_neutral()
            && self.mixer.is_neutral()
    }

    /// Whether all four curves are straight lines, whatever the tone.
    pub fn is_straight(&self) -> bool {
        let close = |curve: &Curve| {
            curve
                .iter()
                .zip(STRAIGHT)
                .all(|(a, b)| (a - b).abs() < 1e-4)
        };
        close(&self.master) && close(&self.red) && close(&self.green) && close(&self.blue)
    }

    /// One display-encoded colour through the curves: the master first, then
    /// each channel's own.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let channel = |curve: &Curve, v: f32| sample(curve, sample(&self.master, v));
        let wheeled = self.mixer.apply(self.wheels.apply(rgb));
        self.tone.apply([
            channel(&self.red, wheeled[0]),
            channel(&self.green, wheeled[1]),
            channel(&self.blue, wheeled[2]),
        ])
    }

    /// The id of the table these curves make over `under`, the clip's own
    /// file LUT if it has one. Derived from both, so equal inputs share a
    /// table.
    pub fn lut_id(&self, under: Option<ClipLut>) -> LutId {
        use std::hash::{Hash, Hasher};
        let mut first = std::collections::hash_map::DefaultHasher::new();
        let mut second = std::collections::hash_map::DefaultHasher::new();
        "curves".hash(&mut first);
        "curves, again".hash(&mut second);
        for hasher in [&mut first, &mut second] {
            for curve in [&self.master, &self.red, &self.green, &self.blue] {
                for v in curve {
                    v.to_bits().hash(hasher);
                }
            }
            self.tone.hash(hasher);
            for band in self.mixer.bands {
                for v in [band.hue, band.saturation, band.lightness] {
                    v.to_bits().hash(hasher);
                }
            }
            for wheel in [
                self.wheels.shadows,
                self.wheels.midtones,
                self.wheels.highlights,
            ] {
                for v in wheel {
                    v.to_bits().hash(hasher);
                }
            }
            if let Some(lut) = under {
                lut.lut.hash(hasher);
                lut.strength.to_bits().hash(hasher);
            }
        }
        LutId::from_u128((u128::from(first.finish()) << 64) | u128::from(second.finish()))
    }

    /// The table: every grid colour through `under` (a file LUT at its
    /// strength) and then the curves.
    pub fn build_lut(&self, under: Option<(&CubeLut, f32)>) -> CubeLut {
        let mut table = CubeLut::identity(CURVE_LUT_SIZE);
        for entry in &mut table.table {
            let graded = match under {
                Some((base, strength)) => {
                    let looked = base.apply(*entry);
                    let s = strength.clamp(0.0, 1.0);
                    [
                        entry[0] + (looked[0] - entry[0]) * s,
                        entry[1] + (looked[1] - entry[1]) * s,
                        entry[2] + (looked[2] - entry[2]) * s,
                    ]
                }
                None => *entry,
            };
            *entry = self.apply(graded);
        }
        table.title = Some("curves".to_owned());
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_curve_changes_nothing() {
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            assert!((sample(&STRAIGHT, x) - x).abs() < 1e-5, "{x}");
        }
        let curves = ColourCurves::default();
        assert!(curves.is_identity());
        assert_eq!(curves.apply([0.2, 0.5, 0.9]), [0.2, 0.5, 0.9]);
    }

    /// The line passes through every handle.
    #[test]
    fn the_curve_passes_through_its_handles() {
        let s_curve: Curve = [0.0, 0.18, 0.5, 0.82, 1.0];
        for (i, v) in s_curve.iter().enumerate() {
            let x = i as f32 / 4.0;
            assert!((sample(&s_curve, x) - v).abs() < 1e-5);
        }
        // An S-curve darkens the shadows and brightens the highlights.
        assert!(sample(&s_curve, 0.15) < 0.15);
        assert!(sample(&s_curve, 0.85) > 0.85);
    }

    /// The master moves every channel; a channel curve only its own.
    #[test]
    fn master_and_channel_curves() {
        let lifted: Curve = [0.2, 0.4, 0.6, 0.8, 1.0];
        let master = ColourCurves {
            master: lifted,
            ..ColourCurves::default()
        };
        let out = master.apply([0.0, 0.0, 0.0]);
        assert!(out.iter().all(|c| (c - 0.2).abs() < 1e-4), "{out:?}");

        let red = ColourCurves {
            red: lifted,
            ..ColourCurves::default()
        };
        assert_eq!(red.apply([0.0, 0.0, 0.0]), [0.2, 0.0, 0.0]);
    }

    /// The baked table is the curves, entry for entry, and runs after the
    /// file LUT under it at that LUT's strength.
    #[test]
    fn the_table_is_the_curves_over_the_file_lut() {
        let curves = ColourCurves {
            green: [0.0, 0.1, 0.3, 0.6, 1.0],
            ..ColourCurves::default()
        };
        let table = curves.build_lut(None);
        assert_eq!(table.size, CURVE_LUT_SIZE);
        let probe = [0.5, 0.5, 0.5];
        let direct = curves.apply(probe);
        let looked = table.apply(probe);
        for c in 0..3 {
            assert!(
                (direct[c] - looked[c]).abs() < 0.02,
                "{direct:?} vs {looked:?}"
            );
        }

        // A file LUT that inverts, at half strength, is grey at 0.5 either way,
        // but pulls black to mid-grey before the curves.
        let mut invert = CubeLut::identity(2);
        for entry in &mut invert.table {
            *entry = entry.map(|v| 1.0 - v);
        }
        let over = ColourCurves::default().build_lut(Some((&invert, 0.5)));
        let black = over.apply([0.0, 0.0, 0.0]);
        assert!(black.iter().all(|c| (c - 0.5).abs() < 0.02), "{black:?}");
    }

    #[test]
    fn the_same_curves_share_an_id_and_different_ones_do_not() {
        let a = ColourCurves {
            master: [0.0, 0.2, 0.5, 0.8, 1.0],
            ..ColourCurves::default()
        };
        let b = ColourCurves {
            master: [0.0, 0.2, 0.5, 0.8, 0.95],
            ..ColourCurves::default()
        };
        assert_eq!(a.lut_id(None), a.lut_id(None));
        assert_ne!(a.lut_id(None), b.lut_id(None));
        let file = ClipLut::new(LutId::new());
        assert_ne!(a.lut_id(None), a.lut_id(Some(file)));
    }

    #[test]
    fn bad_handles_are_held() {
        let wild = ColourCurves {
            red: [f32::NAN, -1.0, 0.5, 2.0, 1.0],
            ..ColourCurves::default()
        }
        .clamped();
        assert_eq!(wild.red, [0.0, 0.0, 0.5, 1.0, 1.0]);
    }
}
