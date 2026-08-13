//! Export failures.

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Media(#[from] bettercut_media::MediaError),

    #[error(transparent)]
    Render(#[from] bettercut_renderer::RenderError),

    /// §4.2: wgpu failing to initialise is a supported outcome, not a crash.
    /// Export cannot proceed without a GPU, but the message says so plainly.
    #[error("no GPU available for export: {0}")]
    NoGpu(String),

    #[error("there is nothing in the selected range to export")]
    EmptyRange,

    /// Not really an error, but it has to travel the same path as one so the
    /// caller cleans up identically. The partial file is already deleted.
    #[error("export cancelled")]
    Cancelled,
}

impl ExportError {
    /// Whether this is the user stopping, rather than something going wrong.
    /// The interface reports the two differently — a cancelled export is not a
    /// failure to apologise for.
    pub fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}
