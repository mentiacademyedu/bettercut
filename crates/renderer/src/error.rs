//! Renderer errors.

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("invalid render resolution {width}x{height}")]
    InvalidResolution { width: u32, height: u32 },

    #[error("frame buffer is {got} bytes, needs {expected}")]
    FrameTooSmall { got: usize, expected: usize },

    /// A GPU-resident frame reached the software upload path.
    ///
    /// §5's target path hands the compositor a texture directly. Until that
    /// interop exists, only RAM frames arrive here, and anything else is a bug
    /// rather than a fallback.
    #[error("this frame is already on the GPU; hardware decode interop is not implemented")]
    UnsupportedFrameStorage,
}
