//! A watermark: a logo in a corner of every frame, over everything.
//!
//! On the sequence rather than bolted onto the export, so the preview shows the
//! frame the file will have (§46) — a logo that only appeared in the export
//! would be found covering something on the day it was posted.

use bettercut_foundation::MediaId;
use serde::{Deserialize, Serialize};

/// Which corner the logo sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

impl WatermarkCorner {
    pub const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top left",
            Self::TopRight => "Top right",
            Self::BottomLeft => "Bottom left",
            Self::BottomRight => "Bottom right",
        }
    }
}

/// The smallest and largest a logo may be, as a share of the frame's width.
pub const MIN_WATERMARK_SIZE: f32 = 0.03;
pub const MAX_WATERMARK_SIZE: f32 = 0.5;

/// The gap between the logo and the frame's edges, as a share of the width.
pub const WATERMARK_MARGIN: f32 = 0.025;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Watermark {
    /// The picture: a photo or logo in the project's media.
    pub media: MediaId,
    #[serde(default)]
    pub corner: WatermarkCorner,
    /// How wide, as a share of the frame's width.
    pub size: f32,
    /// How solid, 0–1.
    pub opacity: f32,
}

impl Watermark {
    /// `media` in the bottom-right corner, a seventh of the width, mostly solid.
    pub fn new(media: MediaId) -> Self {
        Self {
            media,
            corner: WatermarkCorner::default(),
            size: 0.14,
            opacity: 0.85,
        }
    }

    /// Brought into range, as every value a project file can carry is.
    pub fn clamped(self) -> Self {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        Self {
            size: finite(self.size, 0.14).clamp(MIN_WATERMARK_SIZE, MAX_WATERMARK_SIZE),
            opacity: finite(self.opacity, 1.0).clamp(0.0, 1.0),
            ..self
        }
    }

    /// Where the logo's corner goes and which of its corners that is, for a
    /// frame `output_aspect` wide over high: `(position, anchor)` in the
    /// transform's units.
    pub fn placement(&self, output_aspect: f32) -> ([f32; 2], [f32; 2]) {
        let x_margin = WATERMARK_MARGIN;
        // The same gap in pixels on both edges: a share of the width is more
        // of the height on a wide frame.
        let y_margin = WATERMARK_MARGIN * output_aspect.max(0.01);
        let (left, top) = match self.corner {
            WatermarkCorner::TopLeft => (true, true),
            WatermarkCorner::TopRight => (false, true),
            WatermarkCorner::BottomLeft => (true, false),
            WatermarkCorner::BottomRight => (false, false),
        };
        let x = if left {
            -0.5 + x_margin
        } else {
            0.5 - x_margin
        };
        let y = if top { -0.5 + y_margin } else { 0.5 - y_margin };
        (
            [x, y],
            [if left { 0.0 } else { 1.0 }, if top { 0.0 } else { 1.0 }],
        )
    }
}
