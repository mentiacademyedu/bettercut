//! Importing numbered stills as one clip (`bettercut_media::image_sequence`).

use std::path::Path;

use bettercut_foundation::{FrameRate, MediaId};
use bettercut_media::FfmpegProber;

use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Import the run of numbered stills `path` belongs to as one video
    /// asset at `rate`. Like [`Self::import_file`], not undoable: it adds a
    /// library entry and touches nothing on the timeline.
    pub fn import_image_sequence(
        &mut self,
        path: &Path,
        rate: FrameRate,
    ) -> Result<MediaId, EditorError> {
        let asset = FfmpegProber.probe_image_sequence(path, rate)?;
        Ok(self.import_media(asset))
    }
}
