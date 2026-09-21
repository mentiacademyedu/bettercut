//! An image sequence: every frame of the range as a numbered PNG.
//!
//! What a visual-effects artist, a colourist, or a web page that animates on
//! scroll asks for: no codec, no compression artefacts, one lossless picture
//! per frame. Rendered exactly as an export is (§46) — the same render at the
//! same size and rate — and written with the still's own PNG encoder.
//!
//! The frames go in a folder named after the file, beside where the file
//! would have gone: `trip.png` becomes `trip/trip_00001.png`,
//! `trip/trip_00002.png` and so on, numbered from one and padded so they sort
//! in order everywhere. A cancelled or failed export takes back the frames it
//! wrote — and only those, never anything else in the folder.

use std::path::{Path, PathBuf};

use bettercut_foundation::{TimelineTime, ticks_per_frame};
use bettercut_media::{CancellationToken, SeekMode};
use bettercut_playback::{FrameSource, TextFrames};
use bettercut_project_format::Project;
use bettercut_renderer::{Compositor, RenderConfig};
use bettercut_timeline::Sequence;

use crate::{
    ExportError, ExportProgress, ExportSettings, ExportSummary, open_device, readback,
    render_frame, write_png,
};

/// What the progress and the summary call this writer.
pub const FRAMES_ENCODER: &str = "PNG frames (built in)";

/// The folder an image sequence for `path` goes in: `path` without its
/// extension.
pub fn frames_folder(path: &Path) -> PathBuf {
    path.with_extension("")
}

/// The file for frame `index` (from zero) of the sequence for `path`.
pub fn frame_file(path: &Path, index: u64) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "frame".to_owned(), |s| s.to_string_lossy().into_owned());
    frames_folder(path).join(format!("{stem}_{:05}.png", index + 1))
}

/// Write the range as numbered PNGs (see the module notes).
pub fn export_frames(
    project: &Project,
    sequence: &Sequence,
    settings: &ExportSettings,
    on_progress: &mut dyn FnMut(ExportProgress),
    cancel: &dyn CancellationToken,
) -> Result<ExportSummary, ExportError> {
    let Some(ticks_per_frame) = ticks_per_frame(settings.frame_rate).filter(|t| *t > 0) else {
        return Err(ExportError::EmptyRange);
    };
    let span = settings.range.end.ticks() - settings.range.start.ticks();
    if span <= 0 {
        return Err(ExportError::EmptyRange);
    }
    let total_frames = ((span + ticks_per_frame - 1) / ticks_per_frame).max(1) as u64;

    let (device, queue) = open_device()?;
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        RenderConfig::export_to_texture(settings.resolution),
    )?;
    let mut readback = readback::Readback::new(&device, settings.resolution);
    bettercut_playback::load_luts(project, &mut std::collections::HashSet::new(), |id, lut| {
        compositor.load_lut(id, lut);
    });
    let mut frames = FrameSource::new(settings.threads);
    let mut titles = TextFrames::new();

    let folder = frames_folder(&settings.path);
    std::fs::create_dir_all(&folder)
        .map_err(|err| ExportError::Write(format!("{}: {err}", folder.display())))?;

    let mut written: Vec<PathBuf> = Vec::new();
    let result = (|| {
        for index in 0..total_frames {
            if cancel.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            let position = TimelineTime::from_ticks(
                settings.range.start.ticks() + index as i64 * ticks_per_frame,
            );
            let rgba = render_frame(
                project,
                sequence,
                position,
                SeekMode::Playback,
                &mut frames,
                &mut titles,
                &mut compositor,
                &mut readback,
                &device,
                &queue,
                cancel,
            )?;
            let file = frame_file(&settings.path, index);
            write_png(&file, settings.resolution, &rgba)?;
            written.push(file);
            on_progress(ExportProgress {
                frames_done: index + 1,
                frames_total: total_frames,
                encoder: FRAMES_ENCODER,
            });
        }
        Ok(())
    })();

    if let Err(err) = result {
        for file in &written {
            let _ = std::fs::remove_file(file);
        }
        // Only when nothing else is in it: `remove_dir` refuses a folder
        // that is not empty.
        let _ = std::fs::remove_dir(&folder);
        return Err(err);
    }
    Ok(ExportSummary {
        path: folder,
        frames: total_frames,
        encoder: FRAMES_ENCODER,
        hardware: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_numbered_from_one_in_a_folder_named_for_the_file() {
        let path = Path::new("/out/trip.png");
        assert_eq!(frames_folder(path), PathBuf::from("/out/trip"));
        assert_eq!(
            frame_file(path, 0),
            PathBuf::from("/out/trip/trip_00001.png")
        );
        assert_eq!(
            frame_file(path, 41),
            PathBuf::from("/out/trip/trip_00042.png")
        );
    }
}
