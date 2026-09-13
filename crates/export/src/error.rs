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

    /// A still or a WAV could not be written: the folder is not writable, the
    /// disk is full. Carries the path, because "could not save" without one is
    /// a message the user cannot act on.
    #[error("could not write {0}")]
    Write(String),

    /// Sound only was asked for, and every sound track is off or there are
    /// none. An empty WAV would look like a file that failed to play.
    #[error("there is no sound to export — every sound track is off")]
    NoSound,

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
