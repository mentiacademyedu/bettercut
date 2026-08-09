//! What the GPU actually is (§49, §50).
//!
//! Until this existed, every claim about graphics performance was made on one
//! machine with a discrete RTX 4060 — the opposite of §52.1's target, which has
//! integrated graphics and no dedicated VRAM at all. A bug report from another
//! machine is only useful if it says which adapter and backend were picked, so
//! that goes in the log at startup and on screen in the System panel.

use crate::wgpu;

/// The one distinction that changes expectations.
///
/// Integrated graphics share bandwidth with the CPU, which is why §16's preview
/// scaling and §17's adaptive quality exist; a software adapter means wgpu found
/// no usable GPU at all and everything will be slow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuKind {
    Discrete,
    Integrated,
    /// A virtual GPU, as seen in VMs.
    Virtual,
    /// Software rasterization — lavapipe, WARP, llvmpipe.
    Software,
    Other,
}

impl GpuKind {
    /// True when there is no real GPU behind this adapter.
    ///
    /// §50 requires the editor to keep running rather than refusing to start,
    /// but the user should be told why everything feels slow.
    pub fn is_software(self) -> bool {
        matches!(self, Self::Software)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Discrete => "discrete",
            Self::Integrated => "integrated",
            Self::Virtual => "virtual",
            Self::Software => "software",
            Self::Other => "unknown",
        }
    }
}

/// A human-readable summary of the adapter in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuDescription {
    pub name: String,
    pub backend: String,
    pub kind: GpuKind,
    /// Driver name and version, when the backend reports them. Intel and AMD
    /// driver versions matter for graphics bugs, so they are worth carrying.
    pub driver: String,
}

impl GpuDescription {
    pub fn describe(adapter: &wgpu::Adapter) -> Self {
        let info = adapter.get_info();

        let kind = match info.device_type {
            wgpu::DeviceType::DiscreteGpu => GpuKind::Discrete,
            wgpu::DeviceType::IntegratedGpu => GpuKind::Integrated,
            wgpu::DeviceType::VirtualGpu => GpuKind::Virtual,
            wgpu::DeviceType::Cpu => GpuKind::Software,
            wgpu::DeviceType::Other => GpuKind::Other,
        };

        let driver = match (info.driver.is_empty(), info.driver_info.is_empty()) {
            (true, true) => "unknown".to_owned(),
            (false, true) => info.driver.clone(),
            (true, false) => info.driver_info.clone(),
            (false, false) => format!("{} {}", info.driver, info.driver_info),
        };

        Self {
            name: info.name,
            backend: info.backend.to_string(),
            kind,
            driver,
        }
    }
}

impl std::fmt::Display for GpuDescription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({}, {}) via {}",
            self.name,
            self.kind.label(),
            self.driver,
            self.backend
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_is_the_only_kind_that_counts_as_no_gpu() {
        assert!(GpuKind::Software.is_software());
        for kind in [
            GpuKind::Discrete,
            GpuKind::Integrated,
            GpuKind::Virtual,
            GpuKind::Other,
        ] {
            assert!(!kind.is_software(), "{kind:?} should not read as software");
        }
    }

    #[test]
    fn every_kind_has_a_label() {
        for kind in [
            GpuKind::Discrete,
            GpuKind::Integrated,
            GpuKind::Virtual,
            GpuKind::Software,
            GpuKind::Other,
        ] {
            assert!(!kind.label().is_empty());
        }
    }
}
