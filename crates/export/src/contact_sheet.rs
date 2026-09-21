//! A contact sheet: the whole cut as one picture, a grid of frames in order.
//!
//! The thing a photographer prints a roll onto before choosing. For an edit it
//! answers "what is in this video" in one image — for a thumbnail hunt, for
//! showing a client what they are getting, for an email that cannot carry a
//! video, for checking that a twenty-minute cut does not sit on one shot for
//! three minutes.
//!
//! # The frames are the export's
//!
//! Each tile is [`crate::render_still`]'s frame — originals, full resolution,
//! full effect tier (§46) — taken through one device and one compositor for
//! the sheet rather than one each, and then shrunk. A thumbnail of the source
//! files would show the footage; this shows the *edit*.
//!
//! # Where the frames are taken
//!
//! At the middle of each slice, not at its start. A cut usually begins on
//! black or on a title, and a sheet whose first tile is black says nothing —
//! the middle of the slice is the frame most likely to be the shot.

use std::path::{Path, PathBuf};

use bettercut_foundation::{SequenceId, TimelineTime};
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::{CancellationToken, SeekMode};
use bettercut_playback::{FrameSource, TextFrames};
use bettercut_project_format::Project;
use bettercut_renderer::{Compositor, RenderConfig};
use bettercut_timeline::{Resolution, Sequence, TimelineRange};

use crate::{ExportError, ExportProgress, ExportSummary, open_device, readback, render_frame};

/// The gap between tiles and around the sheet, in pixels of the sheet.
const GUTTER: u32 = 6;

/// How many frames a sheet may hold. Past this the tiles are too small to read
/// and the render takes longer than watching the video would.
pub const MAX_TILES: u32 = 64;

/// What a sheet should look like.
#[derive(Debug, Clone)]
pub struct SheetSettings {
    pub path: PathBuf,
    /// How many frames to take, evenly spaced across `range`.
    pub tiles: u32,
    /// How many tiles across. The rows follow from the count.
    pub columns: u32,
    /// How wide each tile is drawn, in pixels.
    pub tile_width: u32,
    /// The stretch of the timeline the sheet covers.
    pub range: TimelineRange,
}

impl SheetSettings {
    /// A sheet of `range`, at the sizes that read well on a screen.
    pub fn of(path: PathBuf, range: TimelineRange) -> Self {
        Self {
            path,
            tiles: 12,
            columns: 4,
            tile_width: 480,
            range,
        }
    }

    /// Every instant a tile is taken at: the middle of each slice.
    pub fn instants(&self) -> Vec<TimelineTime> {
        let tiles = self.tiles.clamp(1, MAX_TILES) as i64;
        let span = (self.range.end.ticks() - self.range.start.ticks()).max(1);
        (0..tiles)
            .map(|index| {
                let middle = span * (2 * index + 1) / (2 * tiles);
                TimelineTime::from_ticks(self.range.start.ticks() + middle)
            })
            .collect()
    }

    /// The sheet's size for tiles of `frame`'s shape, and how tall a tile is.
    pub fn layout(&self, frame: Resolution) -> (Resolution, u32) {
        let tiles = self.tiles.clamp(1, MAX_TILES);
        let columns = self.columns.clamp(1, tiles);
        let rows = tiles.div_ceil(columns);
        let tile_width = self.tile_width.max(16);
        // The tiles keep the frame's shape: a squashed contact sheet is harder
        // to read than a small one.
        let tile_height = (tile_width as u64 * u64::from(frame.height.max(1))
            / u64::from(frame.width.max(1)))
        .max(1) as u32;
        let width = columns * tile_width + (columns + 1) * GUTTER;
        let height = rows * tile_height + (rows + 1) * GUTTER;
        (Resolution::new(width, height), tile_height)
    }
}

/// Shrink `rgba` (`from` pixels) to `to` pixels by averaging each source block.
///
/// A box filter rather than nearest: a contact sheet of a busy shot sampled at
/// one pixel in eight is a mess of aliasing, and the average is what the eye
/// would see from far enough away — which is what a tile is.
pub fn shrink(rgba: &[u8], from: Resolution, to: Resolution) -> Vec<u8> {
    let (fw, fh) = (from.width.max(1) as usize, from.height.max(1) as usize);
    let (tw, th) = (to.width.max(1) as usize, to.height.max(1) as usize);
    let mut out = vec![0_u8; tw * th * 4];
    for y in 0..th {
        let (y0, y1) = (
            (y * fh / th),
            (((y + 1) * fh / th).max(y * fh / th + 1)).min(fh),
        );
        for x in 0..tw {
            let (x0, x1) = (
                (x * fw / tw),
                (((x + 1) * fw / tw).max(x * fw / tw + 1)).min(fw),
            );
            let mut sum = [0_u32; 4];
            let mut count = 0_u32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let at = (sy * fw + sx) * 4;
                    if at + 3 >= rgba.len() {
                        continue;
                    }
                    for channel in 0..4 {
                        sum[channel] += u32::from(rgba[at + channel]);
                    }
                    count += 1;
                }
            }
            let at = (y * tw + x) * 4;
            for channel in 0..4 {
                // A block with nothing in it is a block off the edge of the
                // frame, which is nothing to draw.
                out[at + channel] = sum[channel].checked_div(count).unwrap_or(0) as u8;
            }
        }
    }
    out
}

/// Write the contact sheet described by `settings`.
///
/// Blocking, for a worker thread: it renders a frame per tile, each a seek and
/// a composite.
pub fn contact_sheet(
    project: &Project,
    sequence: &Sequence,
    settings: &SheetSettings,
    on_progress: &mut dyn FnMut(ExportProgress),
    cancel: &dyn CancellationToken,
) -> Result<ExportSummary, ExportError> {
    let instants = settings.instants();
    if instants.is_empty() || settings.range.end <= settings.range.start {
        return Err(ExportError::EmptyRange);
    }

    let frame_size = sequence.resolution;
    let (sheet_size, tile_height) = settings.layout(frame_size);
    let tile_size = Resolution::new(settings.tile_width.max(16), tile_height);

    // One device and one compositor for the whole sheet: opening a device per
    // tile is most of the time a sheet takes.
    let (device, queue) = open_device()?;
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(frame_size),
    )?;
    let mut readback = readback::Readback::new(&device, frame_size);
    bettercut_playback::load_luts(project, &mut std::collections::HashSet::new(), |id, lut| {
        compositor.load_lut(id, lut);
    });
    let mut frames = FrameSource::new(1);
    let mut titles = TextFrames::new();

    // Mid grey behind the tiles: white would blow out a dark cut and black
    // would hide a dark shot's edges.
    let mut sheet = vec![40_u8; sheet_size.width as usize * sheet_size.height as usize * 4];
    for pixel in sheet.chunks_exact_mut(4) {
        pixel[3] = 255;
    }

    let columns = settings.columns.clamp(1, settings.tiles.max(1));
    for (index, at) in instants.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let rgba = render_frame(
            project,
            sequence,
            *at,
            SeekMode::Precise,
            &mut frames,
            &mut titles,
            &mut compositor,
            &mut readback,
            &device,
            &queue,
            cancel,
        )?;
        let tile = shrink(&rgba, frame_size, tile_size);

        let (column, row) = (index as u32 % columns, index as u32 / columns);
        let left = GUTTER + column * (tile_size.width + GUTTER);
        let top = GUTTER + row * (tile_height + GUTTER);
        for y in 0..tile_height {
            let from = (y as usize) * tile_size.width as usize * 4;
            let to = ((top + y) as usize * sheet_size.width as usize + left as usize) * 4;
            let run = tile_size.width as usize * 4;
            if to + run <= sheet.len() && from + run <= tile.len() {
                sheet[to..to + run].copy_from_slice(&tile[from..from + run]);
            }
        }

        on_progress(ExportProgress {
            frames_done: index as u64 + 1,
            frames_total: instants.len() as u64,
            encoder: "contact sheet",
        });
    }

    crate::write_png(&settings.path, sheet_size, &sheet)?;
    Ok(ExportSummary {
        path: settings.path.clone(),
        frames: instants.len() as u64,
        encoder: "contact sheet",
        hardware: false,
    })
}

/// A contact sheet on the job scheduler (§74).
pub struct ContactSheetJob {
    project: Project,
    sequence: SequenceId,
    settings: SheetSettings,
}

impl ContactSheetJob {
    pub fn new(project: Project, sequence: SequenceId, settings: SheetSettings) -> Self {
        Self {
            project,
            sequence,
            settings,
        }
    }

    pub fn path(&self) -> &Path {
        &self.settings.path
    }
}

impl Task for ContactSheetJob {
    fn label(&self) -> String {
        format!("Making a contact sheet of {} frames", self.settings.tiles)
    }

    fn priority(&self) -> Priority {
        Priority::Export
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let Some(sequence) = self.project.sequence(self.sequence) else {
            return Err("that sequence is no longer in the project".to_owned());
        };
        let cancel = JobCancel(ctx);
        let mut progress = |progress: ExportProgress| ctx.progress(progress.fraction());
        match contact_sheet(
            &self.project,
            sequence,
            &self.settings,
            &mut progress,
            &cancel,
        ) {
            Ok(_) => Ok(()),
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
