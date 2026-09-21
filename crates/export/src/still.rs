//! One frame, saved as a picture: a thumbnail for the upload, a still for the
//! poster, a frame to send someone asking "this one?".
//!
//! ## The export's frame, not the preview's
//!
//! The preview draws at a reduced size from proxies (§14, §16). A still saved
//! from it would be a 540p proxy frame scaled up — visibly soft beside the
//! video it came from. So a still goes through exactly what an export does
//! (§46): originals, the sequence's full resolution, the full effect tier, the
//! same [`crate::render_frame`]. The one difference is how the frame is found:
//! an export walks forward and never seeks, while a still is one instant,
//! anywhere, so it asks for a frame-accurate seek.
//!
//! ## PNG
//!
//! Lossless, so the still is the rendered frame to the bit; with an alpha
//! channel that is always opaque, because the composite is. The encoder is the
//! `png` crate the windowing stack already links, not a new dependency.

use std::path::{Path, PathBuf};

use bettercut_foundation::{SequenceId, TimelineTime};
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::{CancellationToken, SeekMode};
use bettercut_playback::{FrameSource, TextFrames};
use bettercut_project_format::Project;
use bettercut_renderer::{Compositor, RenderConfig};
use bettercut_timeline::{Resolution, Sequence};

use crate::{ExportError, open_device, readback, render_frame};

/// Render the frame at `position` at the sequence's full resolution, as
/// tightly packed RGBA rows, top row first.
pub fn render_still(
    project: &Project,
    sequence: &Sequence,
    position: TimelineTime,
    cancel: &dyn CancellationToken,
) -> Result<(Resolution, Vec<u8>), ExportError> {
    let resolution = sequence.resolution;
    let (device, queue) = open_device()?;
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(resolution),
    )?;
    let mut readback = readback::Readback::new(&device, resolution);
    bettercut_playback::load_luts(project, &mut std::collections::HashSet::new(), |id, lut| {
        compositor.load_lut(id, lut);
    });
    // §14: originals, as an export reads them. One thread is plenty for one
    // frame.
    let mut frames = FrameSource::new(1);
    let mut titles = TextFrames::new();

    let rgba = render_frame(
        project,
        sequence,
        position,
        SeekMode::Precise,
        &mut frames,
        &mut titles,
        &mut compositor,
        &mut readback,
        &device,
        &queue,
        cancel,
    )?;
    Ok((resolution, rgba))
}

/// Render the frame at `position` and write it to `path` as a PNG.
pub fn save_still(
    project: &Project,
    sequence: &Sequence,
    position: TimelineTime,
    path: &Path,
    cancel: &dyn CancellationToken,
) -> Result<Resolution, ExportError> {
    let (resolution, rgba) = render_still(project, sequence, position, cancel)?;
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    write_png(path, resolution, &rgba)?;
    Ok(resolution)
}

/// Encode tightly packed RGBA as an 8-bit PNG.
pub fn write_png(path: &Path, size: Resolution, rgba: &[u8]) -> Result<(), ExportError> {
    let failed =
        |err: &dyn std::fmt::Display| ExportError::Write(format!("{}: {err}", path.display()));
    let file = std::fs::File::create(path).map_err(|err| failed(&err))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size.width, size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // §21a: the working space is sRGB, so say so and no viewer has to guess.
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(|err| failed(&err))?;
    let result = writer
        .write_image_data(rgba)
        .and_then(|()| writer.finish())
        .map_err(|err| failed(&err));
    if result.is_err() {
        // A half-written PNG is a file that looks like a picture and is not.
        let _ = std::fs::remove_file(path);
    }
    result
}

/// Saving a still, as a background job: opening a GPU device and seeking a
/// decoder is not something the interface waits on (§74).
pub struct StillJob {
    project: Project,
    sequence: SequenceId,
    position: TimelineTime,
    path: PathBuf,
}

impl StillJob {
    /// `project` is taken whole, as an export takes it, so editing on while the
    /// still renders cannot change the frame that was asked for.
    pub fn new(
        project: Project,
        sequence: SequenceId,
        position: TimelineTime,
        path: PathBuf,
    ) -> Self {
        Self {
            project,
            sequence,
            position,
            path,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// A rendered frame waiting to be collected: its size and its RGBA rows.
pub type RenderedFrame = std::sync::Arc<std::sync::Mutex<Option<(Resolution, Vec<u8>)>>>;

/// Rendering a frame to hand back rather than to save — for the clipboard.
/// The same render as [`StillJob`]; the pixels land in `slot`, for whoever
/// submitted the job to collect when it finishes.
pub struct FrameGrabJob {
    project: Project,
    sequence: SequenceId,
    position: TimelineTime,
    slot: RenderedFrame,
}

impl FrameGrabJob {
    pub fn new(project: Project, sequence: SequenceId, position: TimelineTime) -> Self {
        Self {
            project,
            sequence,
            position,
            slot: RenderedFrame::default(),
        }
    }

    /// Where the frame will be once the job has finished.
    pub fn slot(&self) -> RenderedFrame {
        std::sync::Arc::clone(&self.slot)
    }
}

impl Task for FrameGrabJob {
    fn label(&self) -> String {
        format!("Copying the frame at {}", self.position.format_timecode())
    }

    fn priority(&self) -> Priority {
        Priority::Export
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let Some(sequence) = self.project.sequence(self.sequence) else {
            return Err("that sequence is no longer in the project".to_owned());
        };
        let cancel = JobCancel(ctx);
        match render_still(&self.project, sequence, self.position, &cancel) {
            Ok(frame) => {
                if let Ok(mut slot) = self.slot.lock() {
                    *slot = Some(frame);
                }
                Ok(())
            }
            Err(ExportError::Cancelled) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}

/// The scheduler's cancellation, as the media crate asks for it.
struct JobCancel<'a>(&'a JobContext);

impl CancellationToken for JobCancel<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

impl Task for StillJob {
    fn label(&self) -> String {
        let name = self
            .path
            .file_name()
            .map_or_else(|| "frame".to_owned(), |n| n.to_string_lossy().into_owned());
        format!("Saving {name}")
    }

    fn priority(&self) -> Priority {
        // The user pressed a button and is waiting for one file.
        Priority::Export
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let Some(sequence) = self.project.sequence(self.sequence) else {
            return Err("that sequence is no longer in the project".to_owned());
        };
        let cancel = JobCancel(ctx);
        match save_still(&self.project, sequence, self.position, &self.path, &cancel) {
            Ok(_) | Err(ExportError::Cancelled) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}
