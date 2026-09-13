//! Animated GIF: a short loop with no sound, for chats, web pages and anywhere
//! a video player is too much.
//!
//! ## Frames
//!
//! Rendered exactly as a video export renders them (§46) — originals, full
//! effect tier, [`crate::render_frame`] — at the size and rate the settings
//! ask for, which for a GIF are small: a GIF stores every pixel of every frame
//! and grows fast.
//!
//! ## Colours
//!
//! A GIF frame has at most 256 colours. Each frame gets its own table, chosen
//! by median cut over a 15-bit histogram of that frame: the colour space is
//! split, box by box, at the median of whichever box spans the widest range,
//! until there are 256 boxes, and each box's colour is the average of the
//! pixels in it. Every pixel's 15-bit bucket belongs to exactly one box, so
//! mapping a pixel is a table lookup rather than a nearest-colour search.
//!
//! ## The file
//!
//! Written by hand — the format is thirty years old and a few dozen bytes of
//! framing — with the image data compressed by `weezl`, the LZW coder the
//! image stack already links. The loop runs forever, and each frame's delay is
//! taken from the timeline in whole centiseconds with the remainder carried,
//! so a 15 fps loop stays in step with the timeline instead of drifting.

use std::io::Write;

use bettercut_media::{CancellationToken, SeekMode};
use bettercut_playback::{FrameSource, TextFrames};
use bettercut_project_format::Project;
use bettercut_renderer::{Compositor, RenderConfig};
use bettercut_timeline::Sequence;

use bettercut_foundation::{TICKS_PER_SECOND, TimelineTime, ticks_per_frame};

use crate::{
    ExportError, ExportProgress, ExportSettings, ExportSummary, open_device, readback, render_frame,
};

/// What the progress and the summary call this encoder.
pub const GIF_ENCODER: &str = "GIF (built in)";

/// Colours in a frame's table: the most a GIF allows.
pub const MAX_COLOURS: usize = 256;

/// Timeline ticks in one GIF centisecond.
const TICKS_PER_CENTISECOND: i64 = TICKS_PER_SECOND / 100;

/// Choose up to [`MAX_COLOURS`] colours for a frame of tightly packed RGBA and
/// map every pixel to one. Returns the table and one index per pixel.
pub fn quantize(rgba: &[u8]) -> (Vec<[u8; 3]>, Vec<u8>) {
    const BUCKETS: usize = 1 << 15;
    let bucket = |r: u8, g: u8, b: u8| {
        (usize::from(r >> 3) << 10) | (usize::from(g >> 3) << 5) | usize::from(b >> 3)
    };

    // How many pixels fall in each 15-bit bucket, and the sum of their real
    // colours, so a box's colour is the true average and not a bucket centre.
    let mut counts = vec![0_u32; BUCKETS];
    let mut sums = vec![[0_u64; 3]; BUCKETS];
    for pixel in rgba.chunks_exact(4) {
        let key = bucket(pixel[0], pixel[1], pixel[2]);
        counts[key] += 1;
        for channel in 0..3 {
            sums[key][channel] += u64::from(pixel[channel]);
        }
    }
    let used: Vec<u16> = (0..BUCKETS)
        .filter(|key| counts[*key] > 0)
        .map(|key| key as u16)
        .collect();

    let channel_of = |key: u16, channel: usize| (key >> (10 - 5 * channel)) & 31;
    let spread = |keys: &[u16]| -> (usize, u16) {
        (0..3)
            .map(|channel| {
                let (low, high) = keys.iter().fold((31, 0), |(low, high), key| {
                    let value = channel_of(*key, channel);
                    (low.min(value), high.max(value))
                });
                (channel, high.saturating_sub(low))
            })
            .max_by_key(|(_, range)| *range)
            .unwrap_or((0, 0))
    };

    let mut boxes: Vec<Vec<u16>> = if used.is_empty() {
        Vec::new()
    } else {
        vec![used]
    };
    while boxes.len() < MAX_COLOURS {
        // The box spanning the widest range of any channel, weighted by how
        // many pixels it holds — splitting a wide box of three stray pixels
        // buys less than splitting the sky.
        let Some((index, channel)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, keys)| keys.len() > 1)
            .map(|(index, keys)| {
                let (channel, range) = spread(keys);
                let weight: u64 = keys
                    .iter()
                    .map(|k| u64::from(counts[usize::from(*k)]))
                    .sum();
                (index, channel, u64::from(range) * weight)
            })
            .filter(|(_, _, score)| *score > 0)
            .max_by_key(|(_, _, score)| *score)
            .map(|(index, channel, _)| (index, channel))
        else {
            break;
        };
        let mut keys = boxes.swap_remove(index);
        keys.sort_unstable_by_key(|key| channel_of(*key, channel));
        // Split where half the pixels are on each side, but never leave a
        // side empty.
        let total: u64 = keys
            .iter()
            .map(|k| u64::from(counts[usize::from(*k)]))
            .sum();
        let mut running = 0;
        let mut split = keys.len() - 1;
        for (position, key) in keys.iter().enumerate() {
            running += u64::from(counts[usize::from(*key)]);
            if running * 2 >= total {
                split = position + 1;
                break;
            }
        }
        let split = split.clamp(1, keys.len() - 1);
        let upper = keys.split_off(split);
        boxes.push(keys);
        boxes.push(upper);
    }

    let mut palette = Vec::with_capacity(boxes.len());
    let mut lookup = vec![0_u8; BUCKETS];
    for (index, keys) in boxes.iter().enumerate() {
        let mut total = 0_u64;
        let mut colour = [0_u64; 3];
        for key in keys {
            let key = usize::from(*key);
            total += u64::from(counts[key]);
            for channel in 0..3 {
                colour[channel] += sums[key][channel];
            }
            lookup[key] = index as u8;
        }
        let total = total.max(1);
        palette.push(colour.map(|sum| ((sum + total / 2) / total) as u8));
    }
    if palette.is_empty() {
        palette.push([0, 0, 0]);
    }

    let indices = rgba
        .chunks_exact(4)
        .map(|pixel| lookup[bucket(pixel[0], pixel[1], pixel[2])])
        .collect();
    (palette, indices)
}

/// Writes an animated, endlessly looping GIF.
pub struct GifWriter<W: Write> {
    out: W,
    width: u16,
    height: u16,
}

impl<W: Write> GifWriter<W> {
    /// Write the header for a `width` x `height` loop.
    pub fn new(mut out: W, width: u16, height: u16) -> std::io::Result<Self> {
        out.write_all(b"GIF89a")?;
        out.write_all(&width.to_le_bytes())?;
        out.write_all(&height.to_le_bytes())?;
        // No global colour table: every frame brings its own.
        out.write_all(&[0, 0, 0])?;
        // Loop forever: the Netscape application extension, count 0.
        out.write_all(&[0x21, 0xFF, 0x0B])?;
        out.write_all(b"NETSCAPE2.0")?;
        out.write_all(&[0x03, 0x01, 0x00, 0x00, 0x00])?;
        Ok(Self { out, width, height })
    }

    /// Add one frame of tightly packed RGBA, shown for `delay` centiseconds.
    pub fn push(&mut self, rgba: &[u8], delay: u16) -> std::io::Result<()> {
        let pixels = usize::from(self.width) * usize::from(self.height);
        if rgba.len() < pixels * 4 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "frame is smaller than the GIF",
            ));
        }
        let (palette, indices) = quantize(&rgba[..pixels * 4]);

        // Graphic control: keep the frame on screen for `delay`; each frame
        // covers the whole picture, so nothing needs disposing.
        self.out.write_all(&[0x21, 0xF9, 0x04, 0x04])?;
        self.out.write_all(&delay.to_le_bytes())?;
        self.out.write_all(&[0x00, 0x00])?;

        // The image: the whole frame, with a 256-entry local colour table.
        self.out.write_all(&[0x2C, 0, 0, 0, 0])?;
        self.out.write_all(&self.width.to_le_bytes())?;
        self.out.write_all(&self.height.to_le_bytes())?;
        self.out.write_all(&[0x80 | 0x07])?;
        for entry in 0..MAX_COLOURS {
            self.out
                .write_all(&palette.get(entry).copied().unwrap_or([0, 0, 0]))?;
        }

        const MIN_CODE_SIZE: u8 = 8;
        let compressed = weezl::encode::Encoder::new(weezl::BitOrder::Lsb, MIN_CODE_SIZE)
            .encode(&indices)
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        self.out.write_all(&[MIN_CODE_SIZE])?;
        for block in compressed.chunks(255) {
            self.out.write_all(&[block.len() as u8])?;
            self.out.write_all(block)?;
        }
        self.out.write_all(&[0x00])
    }

    /// Write the trailer and hand back the output.
    pub fn finish(mut self) -> std::io::Result<W> {
        self.out.write_all(&[0x3B])?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// How long frame `index` shows, in centiseconds, when frames are
/// `ticks_per_frame` apart: the difference between where it and the next frame
/// start, each rounded to a centisecond, so rounding never accumulates.
pub fn frame_delay(index: u64, ticks_per_frame: i64) -> u16 {
    let at = |frame: u64| {
        (frame as i64 * ticks_per_frame + TICKS_PER_CENTISECOND / 2) / TICKS_PER_CENTISECOND
    };
    (at(index + 1) - at(index)).clamp(1, i64::from(u16::MAX)) as u16
}

/// Write `settings.range` of the sequence to `settings.path` as a GIF, at
/// `settings.resolution` and `settings.frame_rate`.
///
/// Blocking, for a worker thread. A cancelled or failed export removes its file.
pub fn export_gif(
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
    let (Ok(width), Ok(height)) = (
        u16::try_from(settings.resolution.width),
        u16::try_from(settings.resolution.height),
    ) else {
        return Err(ExportError::Write(
            "a GIF is at most 65535 pixels on a side".to_owned(),
        ));
    };
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

    let file = std::fs::File::create(&settings.path)
        .map_err(|err| ExportError::Write(format!("{}: {err}", settings.path.display())))?;
    let result = (|| {
        let mut writer = GifWriter::new(std::io::BufWriter::new(file), width, height)
            .map_err(|err| ExportError::Write(err.to_string()))?;
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
            writer
                .push(&rgba, frame_delay(index, ticks_per_frame))
                .map_err(|err| ExportError::Write(err.to_string()))?;
            on_progress(ExportProgress {
                frames_done: index + 1,
                frames_total: total_frames,
                encoder: GIF_ENCODER,
            });
        }
        writer
            .finish()
            .map_err(|err| ExportError::Write(err.to_string()))?;
        Ok(())
    })();

    if let Err(err) = result {
        let _ = std::fs::remove_file(&settings.path);
        return Err(err);
    }
    Ok(ExportSummary {
        path: settings.path.clone(),
        frames: total_frames,
        encoder: GIF_ENCODER,
        hardware: false,
    })
}
