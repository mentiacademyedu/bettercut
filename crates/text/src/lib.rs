//! Text shaping and rasterization (§26).
//!
//! §26.1 is the rule this crate exists to keep:
//!
//! > Text must be shaped and rasterized by the `text/` crate into a GPU
//! > texture, used identically by preview and export.
//!
//! If the preview drew text with the UI toolkit and the export drew it with
//! something else, the two would never match — different hinting, different
//! subpixel positioning, different fallback fonts — and §46 would break in the
//! place users notice first. So there is one rasterizer, it produces a plain
//! RGBA bitmap, and both configurations upload that same bitmap.
//!
//! The UI layer edits text *parameters*. It never rasterizes text that will
//! appear in the output (§74).
pub mod error;
pub mod mask;
pub mod raster;
pub mod style;

pub use error::TextError;
pub use raster::{TextBitmap, TextRenderer};
pub use style::{
    Alignment, Background, FontFamily, FontWeight, MAX_SIZE, MIN_SIZE, Rgba, Shadow, Stroke,
    TextStyle,
};
