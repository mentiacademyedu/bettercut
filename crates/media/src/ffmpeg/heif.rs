//! HEIF / HEIC photos — the iPhone's default.
//!
//! FFmpeg reads them with its MP4 demuxer, and hands over two things a plain
//! photo does not have:
//!
//! * **A crop.** The picture is coded at a size the codec likes (a multiple
//!   of 8 or 64) and a note says how much to trim; see [`crop_of`].
//! * **A tile grid.** A phone photo is not one picture but dozens of 512 px
//!   tiles, each its own HEVC image, with a grid saying where each one goes
//!   on the canvas and which part of the canvas is the photo. FFmpeg's command
//!   line stitches them; the libraries only describe them. [`decode_grid`]
//!   does the stitching.

use rusty_ffmpeg::ffi;

use super::InputContext;
use super::raii::{CodecContext, Frame, Packet, Scaler};
use crate::error::MediaError;

/// Whether a file name is a HEIF-family photo, which FFmpeg's MP4 demuxer
/// reads like a one-frame movie.
pub(crate) fn is_heif_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "heic" | "heif" | "hif"))
}

/// How much to trim off a stream's coded picture: `(top, bottom, left,
/// right)` in pixels. All zero when the stream carries no crop.
pub(crate) fn crop_of(par: &ffi::AVCodecParameters) -> (u32, u32, u32, u32) {
    // SAFETY: the side-data array and its count come from the same
    // parameters; FFmpeg returns null when there is no crop, and a crop is
    // four little-endian 32-bit values.
    unsafe {
        let side = ffi::av_packet_side_data_get(
            par.coded_side_data,
            par.nb_coded_side_data,
            ffi::AV_PKT_DATA_FRAME_CROPPING,
        );
        if side.is_null() || (*side).data.is_null() || (*side).size < 16 {
            return (0, 0, 0, 0);
        }
        let bytes = std::slice::from_raw_parts((*side).data, 16);
        let at =
            |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        (at(0), at(4), at(8), at(12))
    }
}

/// A photo made of tiles.
#[derive(Debug, Clone)]
pub(crate) struct Grid {
    /// The canvas the tiles are placed on.
    pub(crate) canvas: (u32, u32),
    /// The part of the canvas that is the photo: `(x, y, width, height)`.
    pub(crate) shown: (u32, u32, u32, u32),
    /// Each tile: the stream it is in, and where its top left goes.
    pub(crate) tiles: Vec<(i32, u32, u32)>,
    /// What shows where no tile covers the canvas.
    pub(crate) background: [u8; 4],
    /// The clockwise turn the grid asks for, 0, 90, 180 or 270.
    pub(crate) rotation: u16,
}

/// The tile grid in `input`, if it has one.
pub(crate) fn grid_of(input: &InputContext) -> Option<Grid> {
    // SAFETY: the context is open; FFmpeg owns the group array, its groups,
    // their tile grids and offsets, which all live as long as the context.
    unsafe {
        let ctx = &*input.as_ptr();
        if ctx.stream_groups.is_null() {
            return None;
        }
        for g in 0..ctx.nb_stream_groups as usize {
            let group = *ctx.stream_groups.add(g);
            if group.is_null() || (*group).type_ != ffi::AV_STREAM_GROUP_PARAMS_TILE_GRID {
                continue;
            }
            let grid = (*group).params.tile_grid;
            if grid.is_null() || (*grid).offsets.is_null() || (*grid).nb_tiles == 0 {
                continue;
            }
            let grid = &*grid;
            let mut tiles = Vec::with_capacity(grid.nb_tiles as usize);
            for t in 0..grid.nb_tiles as usize {
                let offset = *grid.offsets.add(t);
                if offset.idx >= (*group).nb_streams {
                    return None;
                }
                let stream = *(*group).streams.add(offset.idx as usize);
                if stream.is_null() {
                    return None;
                }
                tiles.push((
                    (*stream).index,
                    offset.horizontal.max(0) as u32,
                    offset.vertical.max(0) as u32,
                ));
            }
            let rotation = {
                let side = ffi::av_packet_side_data_get(
                    grid.coded_side_data,
                    grid.nb_coded_side_data,
                    ffi::AV_PKT_DATA_DISPLAYMATRIX,
                );
                if side.is_null() || (*side).data.is_null() || (*side).size < 36 {
                    0
                } else {
                    super::matrix_rotation((*side).data as *const i32)
                }
            };
            return Some(Grid {
                canvas: (
                    grid.coded_width.max(0) as u32,
                    grid.coded_height.max(0) as u32,
                ),
                shown: (
                    grid.horizontal_offset.max(0) as u32,
                    grid.vertical_offset.max(0) as u32,
                    grid.width.max(0) as u32,
                    grid.height.max(0) as u32,
                ),
                tiles,
                background: grid.background,
                rotation,
            });
        }
        None
    }
}

/// Decode every tile of `grid` and put them together: the photo as RGBA,
/// cropped to the part the grid says is shown. Not turned; that is the
/// caller's, as it is for every other picture.
pub(crate) fn decode_grid(
    input: &InputContext,
    grid: &Grid,
) -> Result<(Vec<u8>, u32, u32), MediaError> {
    let (canvas_w, canvas_h) = grid.canvas;
    let (shown_x, shown_y, shown_w, shown_h) = grid.shown;
    if canvas_w == 0 || canvas_h == 0 || shown_w == 0 || shown_h == 0 {
        return Err(MediaError::DecodeFailed(
            "the photo's grid is empty".to_owned(),
        ));
    }

    // Every tile's data first: one read through the file.
    let wanted: std::collections::HashSet<i32> = grid.tiles.iter().map(|t| t.0).collect();
    let mut packets: std::collections::HashMap<i32, Vec<Packet>> = Default::default();
    let mut packet = Packet::new()?;
    loop {
        packet.unref();
        // SAFETY: both pointers are valid and owned here.
        if unsafe { ffi::av_read_frame(input.as_mut_ptr(), packet.as_ptr()) } < 0 {
            break;
        }
        let index = packet.stream_index();
        if wanted.contains(&index) {
            let kept = Packet::new()?;
            // SAFETY: both packets are valid; the new one takes a reference.
            if unsafe { ffi::av_packet_ref(kept.as_ptr(), packet.as_ptr()) } >= 0 {
                packets.entry(index).or_default().push(kept);
            }
        }
    }

    let streams = input.streams();
    let stream_of = |index: i32| {
        streams
            .iter()
            .copied()
            // SAFETY: `streams()` filtered nulls; they live as long as `input`.
            .find(|s| unsafe { (**s).index } == index)
    };

    let mut canvas = Vec::with_capacity((canvas_w * canvas_h * 4) as usize);
    for _ in 0..canvas_w * canvas_h {
        canvas.extend_from_slice(&grid.background);
    }

    let mut frame = Frame::new()?;
    for &(index, x, y) in &grid.tiles {
        let Some(stream) = stream_of(index) else {
            continue;
        };
        // SAFETY: the stream and its parameters live as long as `input`.
        let par = unsafe { (*stream).codecpar };
        if par.is_null() {
            continue;
        }
        let codec = CodecContext::open(par, 1)?;
        for tile_packet in packets.get(&index).map(Vec::as_slice).unwrap_or(&[]) {
            // SAFETY: both pointers are valid.
            unsafe { ffi::avcodec_send_packet(codec.as_ptr(), tile_packet.as_ptr()) };
        }
        // End of this tile's stream: the decoder hands over what it holds.
        // SAFETY: a null packet is how FFmpeg is told to drain.
        unsafe { ffi::avcodec_send_packet(codec.as_ptr(), std::ptr::null_mut()) };
        frame.unref();
        // SAFETY: both pointers are valid.
        if unsafe { ffi::avcodec_receive_frame(codec.as_ptr(), frame.as_ptr()) } < 0 {
            // A tile that will not decode leaves the background showing
            // rather than losing the whole photo.
            tracing::warn!(tile = index, "a photo tile did not decode");
            continue;
        }

        let (tile_w, tile_h, format) = {
            let f = frame.as_ref();
            (f.width, f.height, f.format)
        };
        // SAFETY: `par` is valid, as above.
        let (full_range, space) = unsafe {
            (
                (*par).color_range == ffi::AVCOL_RANGE_JPEG,
                super::decode::sws_colorspace_of((*par).color_space),
            )
        };
        let mut scaler = Scaler::to_rgba(tile_w, tile_h, format, full_range, space)?;
        let mut tile = vec![0u8; (tile_w * tile_h * 4).max(0) as usize];
        scaler.convert(&frame, &mut tile)?;

        // Onto the canvas, clipped at its edges.
        let (tile_w, tile_h) = (tile_w.max(0) as u32, tile_h.max(0) as u32);
        let rows = tile_h.min(canvas_h.saturating_sub(y));
        let cols = tile_w.min(canvas_w.saturating_sub(x)) as usize;
        for row in 0..rows {
            let from = (row * tile_w * 4) as usize;
            let to = (((y + row) * canvas_w + x) * 4) as usize;
            canvas[to..to + cols * 4].copy_from_slice(&tile[from..from + cols * 4]);
        }
    }

    // The part that is the photo.
    let shown_w = shown_w.min(canvas_w.saturating_sub(shown_x));
    let shown_h = shown_h.min(canvas_h.saturating_sub(shown_y));
    let mut photo = Vec::with_capacity((shown_w * shown_h * 4) as usize);
    for row in 0..shown_h {
        let from = (((shown_y + row) * canvas_w + shown_x) * 4) as usize;
        photo.extend_from_slice(&canvas[from..from + (shown_w * 4) as usize]);
    }
    Ok((photo, shown_w, shown_h))
}

/// `pixels`, an RGBA picture, brought down to fit within `edge` on its longer
/// side — how a phone's 12-megapixel photo becomes the size every other still
/// is held at. Unchanged when it already fits.
pub(crate) fn fit_rgba(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    edge: u32,
) -> Result<(Vec<u8>, u32, u32), MediaError> {
    let (out_w, out_h) = crate::fit_within(width, height, edge);
    if (out_w, out_h) == (width, height) {
        return Ok((pixels, width, height));
    }
    let source = Frame::video(width as i32, height as i32, ffi::AV_PIX_FMT_RGBA)?;
    {
        let f = source.as_ref();
        let stride = f.linesize[0] as usize;
        for row in 0..height as usize {
            // SAFETY: the frame was allocated for `height` rows of at least
            // `width * 4` bytes, `stride` apart.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pixels.as_ptr().add(row * width as usize * 4),
                    f.data[0].add(row * stride),
                    width as usize * 4,
                );
            }
        }
    }
    let mut scaler = Scaler::to_rgba_sized(
        (width as i32, height as i32),
        (out_w as i32, out_h as i32),
        ffi::AV_PIX_FMT_RGBA,
        true,
        super::decode::sws_colorspace_of(ffi::AVCOL_SPC_RGB),
    )?;
    let mut out = vec![0u8; (out_w * out_h * 4) as usize];
    scaler.convert(&source, &mut out)?;
    Ok((out, out_w, out_h))
}
