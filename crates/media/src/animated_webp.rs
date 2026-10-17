//! Animated WebP — the format web stickers and many GIF sites now hand out —
//! which the FFmpeg this program is built on cannot decode: it reads the
//! file as one still, finds no picture in it, and the clip is black.
//!
//! So an animated WebP is unpacked at import into numbered PNG frames at a
//! steady rate, and those come in as an image sequence
//! ([`crate::image_sequence`]): from then on it plays, cuts and exports like
//! any clip, transparency and all. Each of the file's frames is held for as
//! long as the file says, so a frame shown for half a second is fifteen of
//! the sequence's at 30 a second.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use crate::error::MediaError;

/// The rate the frames are laid out at.
pub const FRAMES_PER_SECOND: u32 = 30;

/// The most frames one file is unpacked into: two minutes at
/// [`FRAMES_PER_SECOND`]. A sticker loops in a second or two; anything
/// longer than this is cut off rather than filling the disk.
const MOST_FRAMES: u64 = 120 * FRAMES_PER_SECOND as u64;

/// Whether `path` is a WebP that moves: a RIFF/WEBP file whose extended
/// header (`VP8X`) has its animation flag set. Read from the first bytes
/// only, so it costs nothing for every other file.
pub fn is_animated(path: &Path) -> bool {
    let mut head = [0_u8; 21];
    let read = std::fs::File::open(path).and_then(|mut file| {
        use std::io::Read;
        file.read_exact(&mut head)
    });
    read.is_ok()
        && &head[0..4] == b"RIFF"
        && &head[8..12] == b"WEBP"
        && &head[12..16] == b"VP8X"
        && head[20] & 0x02 != 0
}

fn failed(path: &Path, why: impl std::fmt::Display) -> MediaError {
    MediaError::DecodeFailed(format!(
        "{}: an animated WebP that could not be read: {why}",
        path.display()
    ))
}

/// Unpack the animated WebP at `path` into `folder` as `frame_00001.png`
/// and on, and return the first frame's path — what an image-sequence
/// import is pointed at. Frames already there from an earlier import of the
/// same file are kept.
pub fn unpack(path: &Path, folder: &Path) -> Result<PathBuf, MediaError> {
    let first = folder.join(frame_name(1));
    if first.is_file() {
        return Ok(first);
    }
    let file = std::fs::File::open(path).map_err(|e| failed(path, e))?;
    let mut decoder =
        image_webp::WebPDecoder::new(BufReader::new(file)).map_err(|e| failed(path, e))?;
    let (width, height) = decoder.dimensions();
    let alpha = decoder.has_alpha();
    let size = decoder
        .output_buffer_size()
        .ok_or_else(|| failed(path, "it is too large"))?;
    let frames = decoder.num_frames().max(1);

    // Every frame of the file, encoded once, with how long it is shown.
    let mut pictures: Vec<(Vec<u8>, u64)> = Vec::new();
    let mut buffer = vec![0_u8; size];
    for _ in 0..frames {
        let shown = decoder
            .read_frame(&mut buffer)
            .map_err(|e| failed(path, e))?;
        pictures.push((
            png_of(&buffer, width, height, alpha).map_err(|e| failed(path, e))?,
            u64::from(shown),
        ));
    }
    std::fs::create_dir_all(folder).map_err(|e| failed(path, e))?;

    // The steady frames, each showing the file's frame whose time it falls
    // in. A frame said to last no time at all is given a tenth of a second,
    // as browsers do.
    let lengths: Vec<u64> = pictures
        .iter()
        .map(|(_, ms)| if *ms == 0 { 100 } else { *ms })
        .collect();
    let total: u64 = lengths.iter().sum();
    let count = (total * u64::from(FRAMES_PER_SECOND))
        .div_ceil(1000)
        .clamp(1, MOST_FRAMES);
    let (mut current, mut ends) = (0_usize, lengths[0]);
    for index in 0..count {
        let at = index * 1000 / u64::from(FRAMES_PER_SECOND);
        while at >= ends && current + 1 < pictures.len() {
            current += 1;
            ends += lengths[current];
        }
        let frame = folder.join(frame_name(index as u32 + 1));
        std::fs::write(&frame, &pictures[current].0).map_err(|e| failed(path, e))?;
    }
    Ok(first)
}

fn frame_name(number: u32) -> String {
    format!("frame_{number:05}.png")
}

fn png_of(
    pixels: &[u8],
    width: u32,
    height: u32,
    alpha: bool,
) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(if alpha {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.write_header()?.write_image_data(pixels)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_still_or_another_file_is_not_animated() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let still = dir.path().join("still.webp");
        // A plain lossy WebP header: RIFF, WEBP, then VP8 (no X).
        std::fs::write(&still, b"RIFF\x24\x00\x00\x00WEBPVP8 \x18\x00\x00\x00rest")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!is_animated(&still));
        let short = dir.path().join("short.webp");
        std::fs::write(&short, b"RIFF").unwrap_or_else(|e| panic!("{e}"));
        assert!(!is_animated(&short));
        assert!(!is_animated(&dir.path().join("none.webp")));
    }
}
