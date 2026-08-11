//! Filmstrip sheets for timeline clips (§53).
//!
//! A video clip drawn as a flat block says nothing about what is in it. A strip
//! of frames along it turns the timeline into something you can read at a
//! glance — which is most of what makes an editor feel like an editor.
//!
//! # One sheet, fixed tiles
//!
//! [`TILES`] frames evenly spaced across the media, packed side by side into a
//! single image and cached as one file. One file means one read and one GPU
//! texture per clip; per-tile files would mean dozens of both.
//!
//! The consequence is honest and worth knowing: the strip is **coarse**. On a
//! one-hour file the tiles are minutes apart, so zooming in shows the same
//! frame repeated across a stretch of timeline. Editors that avoid this render
//! more tiles on demand as you zoom; doing that well needs the zoom level to
//! drive cache keys, and this is the version that works everywhere first.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bettercut_cache::{CacheStore, Thumbnail};
use bettercut_foundation::{MediaId, MediaTime};
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::{FfmpegDecoder, FrameStorage, MediaAsset, MediaDecoder, MediaKind, SeekMode};

/// Frames per strip.
///
/// 32 is a compromise between how much of a long file you can see and what it
/// costs: at [`TILE_WIDTH`] each sheet is ~294 KB and takes 32 seeks to build.
pub const TILES: u32 = 32;

/// Width of one tile. 64 px is about what a clip shows before the eye stops
/// resolving detail at timeline scale.
pub const TILE_WIDTH: u32 = 64;

struct JobCancellation {
    cancelled: Arc<AtomicBool>,
}

impl bettercut_media::CancellationToken for JobCancellation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Build one filmstrip sheet.
pub struct FilmstripJob {
    asset: MediaAsset,
    output: std::path::PathBuf,
    threads: u32,
}

impl FilmstripJob {
    /// Prepare a job, or `None` when there is nothing to build.
    pub fn new(asset: &MediaAsset, cache: &CacheStore, threads: u32) -> Option<Self> {
        // Video only: audio has a waveform, and a still image is its own
        // thumbnail repeated 32 times.
        if asset.kind != MediaKind::Video || asset.missing || asset.duration.is_zero() {
            return None;
        }
        let output = cache.layout().filmstrip_file(asset.id, TILES, TILE_WIDTH);
        if output.exists() {
            return None;
        }

        Some(Self {
            asset: asset.clone(),
            output,
            threads: threads.max(1),
        })
    }

    pub fn media(&self) -> MediaId {
        self.asset.id
    }
}

impl Task for FilmstripJob {
    fn label(&self) -> String {
        format!("Filmstrip for {}", self.asset.file_name)
    }

    fn priority(&self) -> Priority {
        Priority::Background
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = JobCancellation {
            cancelled: Arc::clone(&cancelled),
        };

        let mut decoder = FfmpegDecoder::new(self.threads).map_err(|e| e.to_string())?;
        decoder.open(&self.asset).map_err(|e| e.to_string())?;

        let duration = self.asset.duration.ticks().max(1);
        let mut sheet: Option<Sheet> = None;

        for tile in 0..TILES {
            if ctx.is_cancelled() {
                cancelled.store(true, Ordering::Release);
                return Err("cancelled".to_owned());
            }

            // Sample at tile centres rather than edges: the last frame of a
            // file is often black, and starting at zero wastes a tile on a
            // fade-in.
            let at = MediaTime::from_ticks(
                duration * (2 * i64::from(tile) + 1) / (2 * i64::from(TILES)),
            );

            // `Scrub` accuracy: the nearest keyframe is fine for a 64 px tile,
            // and precise seeking 32 times would cost far more.
            if decoder.seek(at, SeekMode::Scrub).is_err() {
                continue;
            }
            let Ok(Some(frame)) = decoder.decode_frame(&token) else {
                continue;
            };
            let FrameStorage::System { data, stride } = &frame.storage else {
                continue;
            };

            let tile_image = crate::thumbnail_job::downscale(
                data,
                *stride as usize,
                frame.width,
                frame.height,
                TILE_WIDTH,
            )
            .map_err(|e| e.to_string())?;

            let sheet = sheet.get_or_insert_with(|| Sheet::new(tile_image.height));
            sheet.place(tile, &tile_image);

            ctx.progress((tile + 1) as f32 / TILES as f32);
        }

        let sheet = sheet.ok_or_else(|| "no frames decoded for the filmstrip".to_owned())?;
        sheet
            .finish()
            .map_err(|e| e.to_string())?
            .write(&self.output)
            .map_err(|e| e.to_string())?;

        tracing::debug!(file = %self.asset.file_name, "filmstrip ready");
        Ok(())
    }
}

/// Tiles packed left to right into one image.
struct Sheet {
    height: u32,
    rgba: Vec<u8>,
}

impl Sheet {
    fn new(height: u32) -> Self {
        let height = height.max(1);
        Self {
            height,
            rgba: vec![0_u8; (TILES * TILE_WIDTH * height * 4) as usize],
        }
    }

    /// Copy one tile into its slot. A tile that decoded at a different height
    /// is clipped rather than skipped, so an odd frame does not leave a hole.
    fn place(&mut self, index: u32, tile: &Thumbnail) {
        let sheet_width = (TILES * TILE_WIDTH) as usize;
        let x0 = (index * TILE_WIDTH) as usize;

        for y in 0..tile.height.min(self.height) as usize {
            for x in 0..tile.width.min(TILE_WIDTH) as usize {
                let from = (y * tile.width as usize + x) * 4;
                let to = ((y * sheet_width) + x0 + x) * 4;
                let (Some(src), Some(dst)) =
                    (tile.rgba.get(from..from + 4), self.rgba.get_mut(to..to + 4))
                else {
                    continue;
                };
                dst.copy_from_slice(src);
            }
        }
    }

    fn finish(self) -> Result<Thumbnail, bettercut_cache::CacheError> {
        Thumbnail::new(TILES * TILE_WIDTH, self.height, self.rgba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(colour: u8, width: u32, height: u32) -> Thumbnail {
        Thumbnail::new(width, height, vec![colour; (width * height * 4) as usize]).expect("valid")
    }

    #[test]
    fn tiles_land_in_their_own_slots() {
        let mut sheet = Sheet::new(4);
        sheet.place(0, &tile(11, TILE_WIDTH, 4));
        sheet.place(2, &tile(22, TILE_WIDTH, 4));

        let image = sheet.finish().expect("valid sheet");
        let row = TILES as usize * TILE_WIDTH as usize;

        // First pixel of tile 0, tile 1 (never written) and tile 2.
        assert_eq!(image.rgba[0], 11);
        assert_eq!(image.rgba[TILE_WIDTH as usize * 4], 0, "tile 1 was written");
        assert_eq!(image.rgba[2 * TILE_WIDTH as usize * 4], 22);
        assert_eq!(image.rgba.len(), row * 4 * 4);
    }

    /// A frame that decodes at an unexpected size must not corrupt neighbours.
    #[test]
    fn an_oversized_tile_is_clipped_not_smeared() {
        let mut sheet = Sheet::new(4);
        sheet.place(0, &tile(99, TILE_WIDTH * 3, 4));

        let image = sheet.finish().expect("valid sheet");
        assert_eq!(image.rgba[0], 99);
        assert_eq!(
            image.rgba[TILE_WIDTH as usize * 4],
            0,
            "an oversized tile spilled into the next slot"
        );
    }

    #[test]
    fn the_sheet_is_the_expected_size() {
        let sheet = Sheet::new(36).finish().expect("valid");
        assert_eq!(sheet.width, TILES * TILE_WIDTH);
        assert_eq!(sheet.height, 36);
    }
}
