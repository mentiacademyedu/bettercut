//! Shapes: a rectangle, a rounded rectangle, an ellipse — the plain blocks a
//! short video is built from. A box behind a caption, a highlight over part of
//! a shot, a coloured bar for a lower third.
//!
//! # Drawn like a title
//!
//! A shape rides on the title lane (§26): the same clip, the same placement,
//! scale, rotation, entrance and exit, the same handles in the preview. Only
//! the picture differs, and this is where it is made — a bitmap at sequence
//! pixels, exactly as a title's is, so the preview and the export draw the same
//! pixels (§46).
//!
//! # Edges
//!
//! Every pixel's coverage comes from its distance to the shape's edge, so
//! edges are anti-aliased at any size without supersampling, and an outline is
//! simply a band of that distance.

use serde::{Deserialize, Serialize};

use crate::raster::TextBitmap;
use crate::style::{Rgba, Stroke};

/// Which shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle,
    Ellipse,
}

impl ShapeKind {
    pub const ALL: [Self; 3] = [Self::Rectangle, Self::RoundedRectangle, Self::Ellipse];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::RoundedRectangle => "Rounded",
            Self::Ellipse => "Ellipse",
        }
    }
}

/// The largest side a shape may have, in sequence pixels: comfortably past a
/// 4K frame, and a limit on what a hand-edited file can make us allocate.
pub const MAX_SHAPE_SIDE: f32 = 8192.0;

/// A shape and how it is drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub kind: ShapeKind,
    /// Size in sequence pixels, before the clip's own scale.
    pub width: f32,
    pub height: f32,
    pub fill: Rgba,
    /// An outline around the edge, drawn inside the shape's own size.
    #[serde(default)]
    pub outline: Option<Stroke>,
    /// Corner radius for a rounded rectangle, in pixels; ignored otherwise.
    #[serde(default = "default_radius")]
    pub corner_radius: f32,
}

fn default_radius() -> f32 {
    24.0
}

impl Shape {
    /// A new shape of `kind`: a medium white block, big enough to see and
    /// small enough to place.
    pub fn new(kind: ShapeKind) -> Self {
        Self {
            kind,
            width: 480.0,
            height: 270.0,
            fill: Rgba::WHITE,
            outline: None,
            corner_radius: default_radius(),
        }
    }

    /// What the bitmap cache keys this shape by.
    pub fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "shape".hash(&mut hasher);
        self.kind.hash(&mut hasher);
        for value in [self.width, self.height, self.corner_radius] {
            value.to_bits().hash(&mut hasher);
        }
        self.fill.hash(&mut hasher);
        if let Some(outline) = &self.outline {
            outline.width.to_bits().hash(&mut hasher);
            outline.color.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Draw the shape: sRGB-encoded RGBA, straight alpha, the size of the
    /// shape itself (at least a pixel each way).
    pub fn rasterize(&self) -> TextBitmap {
        let clamp_side = |v: f32| {
            if v.is_finite() {
                v.clamp(1.0, MAX_SHAPE_SIDE)
            } else {
                1.0
            }
        };
        let (w, h) = (clamp_side(self.width), clamp_side(self.height));
        let (width, height) = (w.ceil() as u32, h.ceil() as u32);
        let (half_w, half_h) = (w / 2.0, h / 2.0);
        let radius = match self.kind {
            ShapeKind::RoundedRectangle => {
                if self.corner_radius.is_finite() {
                    self.corner_radius.clamp(0.0, half_w.min(half_h))
                } else {
                    0.0
                }
            }
            ShapeKind::Rectangle | ShapeKind::Ellipse => 0.0,
        };
        let outline = self
            .outline
            .filter(|o| o.width.is_finite() && o.width > 0.0)
            .map(|o| Stroke {
                width: o.width.min(half_w.min(half_h)),
                color: o.color,
            });

        let mut pixels = vec![0_u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                // From the shape's centre, at the pixel's centre.
                let px = x as f32 + 0.5 - half_w;
                let py = y as f32 + 0.5 - half_h;
                let distance = self.signed_distance(px, py, half_w, half_h, radius);
                // Inside is negative; a half-pixel ramp either side of the edge.
                let inside = (0.5 - distance).clamp(0.0, 1.0);
                if inside <= 0.0 {
                    continue;
                }
                let colour = match outline {
                    Some(stroke) => {
                        // The band from the edge inward by the outline's width.
                        let band = (0.5 - (-distance - stroke.width)).clamp(0.0, 1.0);
                        mix(self.fill, stroke.color, band)
                    }
                    None => self.fill,
                };
                let at = ((y * width + x) * 4) as usize;
                let alpha = (f32::from(colour.a) * inside).round() as u8;
                pixels[at..at + 4].copy_from_slice(&[colour.r, colour.g, colour.b, alpha]);
            }
        }
        TextBitmap {
            width,
            height,
            pixels,
        }
    }

    /// Distance from (px, py) to the edge, negative inside.
    fn signed_distance(&self, px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
        match self.kind {
            ShapeKind::Rectangle | ShapeKind::RoundedRectangle => {
                // The rounded box: a box shrunk by the radius, grown back
                // round the corners.
                let qx = px.abs() - (half_w - radius);
                let qy = py.abs() - (half_h - radius);
                let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
                outside + qx.max(qy).min(0.0) - radius
            }
            ShapeKind::Ellipse => {
                // Scaled to a unit circle and back by the local stretch — close
                // to the true distance near the edge, which is all the
                // anti-aliasing and the outline need.
                let nx = px / half_w;
                let ny = py / half_h;
                let k = (nx * nx + ny * ny).sqrt();
                if k <= f32::EPSILON {
                    return -half_w.min(half_h);
                }
                let gradient = ((nx / half_w).powi(2) + (ny / half_h).powi(2)).sqrt() / k;
                (k - 1.0) / gradient.max(f32::EPSILON)
            }
        }
    }
}

/// `a` towards `b` by `t`, alpha included.
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgba::new(
        lerp(a.r, b.r),
        lerp(a.g, b.g),
        lerp(a.b, b.b),
        lerp(a.a, b.a),
    )
}
