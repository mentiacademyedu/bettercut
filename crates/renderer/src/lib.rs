//! The render graph (§21, §22, §46).
//!
//! **One implementation, two configurations.** §74 lists "introduce a second
//! renderer implementation" among the things an agent must not do, and §46
//! explains why: two renderers will diverge, and the divergence shows up as an
//! export that does not match what the user approved on screen.
//!
//! So preview and export differ only by [`RenderConfig`] — resolution, which
//! media is read, the effect quality tier, and where the pixels go.
//!
//! This crate deliberately does not depend on egui. The UI hands it egui's
//! `wgpu::Device`, which is what makes §4.1's zero-copy preview work, but
//! export has no UI and must drive the same code.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod composite {
    //! Re-exported for callers that want the shader source, e.g. for tests.
    pub const SHADER: &str = include_str!("composite.wgsl");
}

pub mod compositor;
pub mod config;
pub mod error;

pub use compositor::{Compositor, Layer};
pub use config::{MediaSourceMode, PreviewQuality, QualityTier, RenderConfig, RenderTarget};
pub use error::RenderError;

/// The wgpu this crate was built against.
///
/// Re-exported so the UI cannot accidentally pair a texture from one wgpu
/// version with a device from another — which compiles, then fails at runtime
/// in a way that reads like a driver bug.
pub use eframe::wgpu;
