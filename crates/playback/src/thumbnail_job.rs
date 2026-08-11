//! Poster thumbnails as a background job (§12, §19).
//!
//! One frame, decoded once, scaled down and cached. Cheap compared with a proxy
//! encode, so §69 puts it at `Background` — it is not heavy, and the scheduler
//! is free to run several while a proxy encode holds the one heavy slot.
//!
//! Decoding happens through the ordinary [`bettercut_media::MediaDecoder`]
//! rather than a bespoke FFmpeg path: that already resolves colour correctly at
//! the boundary (§21a.2), so a thumbnail cannot drift from what the preview
//! shows for the same frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bettercut_cache::{CacheStore, Thumbnail};
use bettercut_foundation::{MediaId, MediaTime};
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::{FfmpegDecoder, FrameStorage, MediaAsset, MediaDecoder, MediaKind, SeekMode};

/// Bridges the scheduler's cancellation to the media crate's token (§86).
struct JobCancellation {
    cancelled: Arc<AtomicBool>,
}

impl bettercut_media::CancellationToken for JobCancellation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Generate one poster thumbnail.
pub struct ThumbnailJob {
    asset: MediaAsset,
    width: u32,
    output: std::path::PathBuf,
    threads: u32,
}

impl ThumbnailJob {
    /// Prepare a job, or `None` when there is nothing to do.
    ///
    /// Declines audio (there is no picture), missing media, and anything
    /// already cached at this width.
    pub fn new(asset: &MediaAsset, width: u32, cache: &CacheStore, threads: u32) -> Option<Self> {
        if asset.kind == MediaKind::Audio || asset.missing || width == 0 {
            return None;
        }
        let output = cache.layout().thumbnail_file(asset.id, width);
        if output.exists() {
            return None;
        }

        Some(Self {
            asset: asset.clone(),
            width,
            output,
            threads: threads.max(1),
        })
    }

    pub fn media(&self) -> MediaId {
        self.asset.id
    }

    /// Where in the media to grab the poster frame.
    ///
    /// A tenth of the way in, not frame zero: the first frame of real footage
    /// is very often black, a slate, or a fade from black, all of which make a
    /// useless thumbnail. Still images have no duration, so they take frame
    /// zero — which is also their only frame.
    fn poster_time(&self) -> MediaTime {
        let ticks = self.asset.duration.ticks();
        if ticks <= 0 {
            return MediaTime::ZERO;
        }
        MediaTime::from_ticks(ticks / 10)
    }
}

impl Task for ThumbnailJob {
    fn label(&self) -> String {
        format!("Thumbnail for {}", self.asset.file_name)
    }

    fn priority(&self) -> Priority {
        Priority::Background
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        if ctx.is_cancelled() {
            cancelled.store(true, Ordering::Release);
        }
        let token = JobCancellation {
            cancelled: Arc::clone(&cancelled),
        };

        let mut decoder = FfmpegDecoder::new(self.threads).map_err(|e| e.to_string())?;
        decoder.open(&self.asset).map_err(|e| e.to_string())?;

        // Scrub accuracy is right here: the nearest keyframe is a perfectly
        // good poster, and decoding forward to an exact frame would cost far
        // more for no visible benefit.
        decoder
            .seek(self.poster_time(), SeekMode::Scrub)
            .map_err(|e| e.to_string())?;
        ctx.progress(0.4);

        let frame = decoder
            .decode_frame(&token)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no frame at the poster position".to_owned())?;

        let FrameStorage::System { data, stride } = &frame.storage else {
            // §5's hardware path would need a readback; nothing produces it yet.
            return Err("thumbnails need a system-memory frame".to_owned());
        };

        if ctx.is_cancelled() {
            return Err("cancelled".to_owned());
        }
        ctx.progress(0.8);

        let thumbnail = downscale(
            data,
            *stride as usize,
            frame.width,
            frame.height,
            self.width,
        )
        .map_err(|e| e.to_string())?;

        thumbnail.write(&self.output).map_err(|e| e.to_string())?;
        ctx.progress(1.0);

        tracing::debug!(
            file = %self.asset.file_name,
            width = thumbnail.width,
            height = thumbnail.height,
            "thumbnail ready"
        );
        Ok(())
    }
}

/// Box-filter downscale of a tightly-packed RGBA frame.
///
/// Averaging over each source block rather than sampling one pixel: nearest
/// neighbour on a 10x reduction aliases badly, and a thumbnail is looked at
/// closely precisely because it is small. Integer arithmetic throughout, so the
/// result is identical on every machine — golden-image tests depend on that.
pub(crate) fn downscale(
    data: &[u8],
    stride: usize,
    src_width: u32,
    src_height: u32,
    target_width: u32,
) -> Result<Thumbnail, bettercut_cache::CacheError> {
    let (sw, sh) = (src_width as usize, src_height as usize);
    let dw = (target_width as usize).min(sw).max(1);
    // Preserve aspect. `max(1)` because a very wide, short source would
    // otherwise round to zero rows.
    let dh = ((dw * sh) / sw.max(1)).max(1);

    let mut out = vec![0_u8; dw * dh * 4];

    for y in 0..dh {
        // Source rows covered by this output row.
        let y0 = y * sh / dh;
        let y1 = (((y + 1) * sh) / dh).max(y0 + 1).min(sh);

        for x in 0..dw {
            let x0 = x * sw / dw;
            let x1 = (((x + 1) * sw) / dw).max(x0 + 1).min(sw);

            let (mut r, mut g, mut b, mut a) = (0_u32, 0_u32, 0_u32, 0_u32);
            let mut count = 0_u32;

            for sy in y0..y1 {
                let row = sy * stride;
                for sx in x0..x1 {
                    let i = row + sx * 4;
                    // A frame whose stride disagrees with its height would
                    // otherwise index out of bounds.
                    let Some(px) = data.get(i..i + 4) else {
                        continue;
                    };
                    r += u32::from(px[0]);
                    g += u32::from(px[1]);
                    b += u32::from(px[2]);
                    a += u32::from(px[3]);
                    count += 1;
                }
            }

            if count == 0 {
                continue;
            }
            let o = (y * dw + x) * 4;
            out[o] = (r / count) as u8;
            out[o + 1] = (g / count) as u8;
            out[o + 2] = (b / count) as u8;
            out[o + 3] = (a / count) as u8;
        }
    }

    Thumbnail::new(dw as u32, dh as u32, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid colour must survive averaging exactly — any drift here would
    /// show as a tint on every thumbnail.
    #[test]
    fn a_solid_image_downscales_to_the_same_colour() {
        let (w, h) = (64_usize, 32_usize);
        let mut data = vec![0_u8; w * h * 4];
        for px in data.chunks_exact_mut(4) {
            px.copy_from_slice(&[10, 200, 30, 255]);
        }

        let thumb = downscale(&data, w * 4, w as u32, h as u32, 16).expect("valid");
        assert_eq!((thumb.width, thumb.height), (16, 8), "aspect not preserved");
        for px in thumb.rgba.chunks_exact(4) {
            assert_eq!(px, [10, 200, 30, 255]);
        }
    }

    /// Two halves must stay two halves, not blur into one average.
    #[test]
    fn a_split_image_keeps_its_two_sides() {
        let (w, h) = (64_usize, 64_usize);
        let mut data = vec![0_u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let colour = if x < w / 2 { 0 } else { 255 };
                data[i..i + 4].copy_from_slice(&[colour, colour, colour, 255]);
            }
        }

        let thumb = downscale(&data, w * 4, w as u32, h as u32, 8).expect("valid");
        let row = &thumb.rgba[..8 * 4];
        assert_eq!(row[0], 0, "left edge should stay black");
        assert_eq!(row[7 * 4], 255, "right edge should stay white");
    }

    /// Upscaling is not a thumbnail's job; a small source stays its own size.
    #[test]
    fn a_source_smaller_than_the_target_is_not_enlarged() {
        let (w, h) = (8_usize, 4_usize);
        let data = vec![128_u8; w * h * 4];

        let thumb = downscale(&data, w * 4, w as u32, h as u32, 160).expect("valid");
        assert_eq!((thumb.width, thumb.height), (8, 4));
    }

    /// Padded strides are normal from FFmpeg; reading them as tightly packed
    /// would skew every row progressively further.
    #[test]
    fn a_padded_stride_is_respected() {
        let (w, h) = (4_usize, 4_usize);
        let stride = w * 4 + 16;
        let mut data = vec![0_u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * stride + x * 4;
                data[i..i + 4].copy_from_slice(&[77, 77, 77, 255]);
            }
        }

        let thumb = downscale(&data, stride, w as u32, h as u32, 2).expect("valid");
        for px in thumb.rgba.chunks_exact(4) {
            assert_eq!(px, [77, 77, 77, 255], "padding leaked into the image");
        }
    }

    #[test]
    fn an_extremely_wide_source_still_produces_at_least_one_row() {
        let (w, h) = (1000_usize, 2_usize);
        let data = vec![50_u8; w * h * 4];
        let thumb = downscale(&data, w * 4, w as u32, h as u32, 10).expect("valid");
        assert!(thumb.height >= 1);
    }
}
