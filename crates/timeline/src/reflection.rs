//! Reflections: a clip's picture folded onto itself — mirrored halves, four
//! quarters, or a kaleidoscope.
//!
//! Not the flip (`FlipAxis`), which turns the whole picture round. A reflection
//! keeps one part of the shot and repeats it, mirrored, over the rest: a face
//! made symmetrical, a city street folded into a canyon, a pattern out of
//! anything.
//!
//! Each kind is a mapping from where a texel is drawn to where in the source it
//! is read. That mapping lives here, as [`Reflection::source_uv`], and the
//! renderer's shader is a transcription of it — a GPU test holds the two to
//! each other, so what the reference says is what the picture does.

use serde::{Deserialize, Serialize};

/// Wedges in the kaleidoscope. Six, like the toy: a hexagon of mirrors.
pub const KALEIDOSCOPE_SEGMENTS: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reflection {
    /// The picture as shot.
    #[default]
    None,
    /// The left half, and its mirror image on the right.
    LeftRight,
    /// The top half, and its mirror image below.
    TopBottom,
    /// The top-left quarter, mirrored into all four.
    FourWay,
    /// A wedge from the centre, mirrored round the circle.
    Kaleidoscope,
}

impl Reflection {
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::LeftRight,
        Self::TopBottom,
        Self::FourWay,
        Self::Kaleidoscope,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Off",
            Self::LeftRight => "Left & right",
            Self::TopBottom => "Top & bottom",
            Self::FourWay => "Four-way",
            Self::Kaleidoscope => "Kaleidoscope",
        }
    }

    /// The number the shader reads for this kind. Must match `reflect.wgsl`.
    pub fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::LeftRight => 1,
            Self::TopBottom => 2,
            Self::FourWay => 3,
            Self::Kaleidoscope => 4,
        }
    }

    /// Where in the source the texel drawn at `uv` is read, both in 0..1 with
    /// the origin at the top left. `aspect` is the picture's width over its
    /// height, so the kaleidoscope's wedges are true angles rather than ones
    /// stretched with the frame.
    pub fn source_uv(self, [u, v]: [f32; 2], aspect: f32) -> [f32; 2] {
        match self {
            Self::None => [u, v],
            Self::LeftRight => [fold_half(u), v],
            Self::TopBottom => [u, fold_half(v)],
            Self::FourWay => [fold_half(u), fold_half(v)],
            Self::Kaleidoscope => {
                let aspect = if aspect.is_finite() && aspect > 0.0 {
                    aspect
                } else {
                    1.0
                };
                let (x, y) = ((u - 0.5) * aspect, v - 0.5);
                let radius = (x * x + y * y).sqrt();
                let wedge = std::f32::consts::TAU / KALEIDOSCOPE_SEGMENTS as f32;
                let mut angle = y.atan2(x);
                angle -= wedge * (angle / wedge).floor();
                if angle > wedge / 2.0 {
                    angle = wedge - angle;
                }
                // Past the source's edges the corners of the frame read back
                // into it, mirrored, rather than smearing the edge texels.
                [
                    fold_edges(radius * angle.cos() / aspect + 0.5),
                    fold_edges(radius * angle.sin() + 0.5),
                ]
            }
        }
    }
}

/// The first half of 0..1 mirrored onto the second.
fn fold_half(t: f32) -> f32 {
    0.5 - (t - 0.5).abs()
}

/// Any coordinate brought back into 0..1 by mirroring at each edge.
fn fold_edges(t: f32) -> f32 {
    let wrapped = t - 2.0 * (t / 2.0).floor();
    if wrapped > 1.0 {
        2.0 - wrapped
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5
    }

    #[test]
    fn none_reads_where_it_draws() {
        for uv in [[0.1, 0.9], [0.5, 0.5], [0.99, 0.01]] {
            assert_eq!(Reflection::None.source_uv(uv, 1.7), uv);
        }
    }

    #[test]
    fn halves_read_the_first_half_mirrored() {
        let r = Reflection::LeftRight;
        assert!(
            near(r.source_uv([0.2, 0.3], 1.0), [0.2, 0.3]),
            "left side kept"
        );
        assert!(
            near(r.source_uv([0.8, 0.3], 1.0), [0.2, 0.3]),
            "right side mirrors it"
        );
        assert!(near(
            Reflection::TopBottom.source_uv([0.3, 0.9], 1.0),
            [0.3, 0.1]
        ));
        assert!(near(
            Reflection::FourWay.source_uv([0.9, 0.75], 1.0),
            [0.1, 0.25]
        ));
    }

    /// Every wedge reads the same stretch of source: turning a point by one
    /// wedge, or mirroring it within its wedge, reads the same texel.
    #[test]
    fn kaleidoscope_wedges_repeat_and_mirror() {
        let r = Reflection::Kaleidoscope;
        let aspect = 16.0 / 9.0;
        let wedge = std::f32::consts::TAU / KALEIDOSCOPE_SEGMENTS as f32;
        let at = |radius: f32, angle: f32| {
            [
                radius * angle.cos() / aspect + 0.5,
                radius * angle.sin() + 0.5,
            ]
        };
        let base = r.source_uv(at(0.3, 0.2), aspect);
        for turn in 1..KALEIDOSCOPE_SEGMENTS {
            let turned = r.source_uv(at(0.3, 0.2 + wedge * turn as f32), aspect);
            assert!(near(turned, base), "wedge {turn}: {turned:?} vs {base:?}");
        }
        let mirrored = r.source_uv(at(0.3, wedge - 0.2), aspect);
        assert!(near(mirrored, base), "{mirrored:?} vs {base:?}");
        // And the first wedge itself is read unchanged.
        assert!(near(base, at(0.3, 0.2)));
    }

    #[test]
    fn everything_reads_inside_the_source() {
        for kind in Reflection::ALL {
            for i in 0..=20 {
                for j in 0..=20 {
                    let [u, v] = kind.source_uv([i as f32 / 20.0, j as f32 / 20.0], 16.0 / 9.0);
                    assert!(
                        (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v),
                        "{kind:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_saved_reflection_reads_back() {
        for kind in Reflection::ALL {
            let json = serde_json::to_string(&kind).expect("serialize");
            assert_eq!(
                serde_json::from_str::<Reflection>(&json).expect("parse"),
                kind
            );
        }
    }
}
