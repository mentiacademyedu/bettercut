//! Metadata extraction (§12, §84).
//!
//! Reads everything §12 lists, including the colour properties §21a needs. The
//! important detail is that colour is **resolved here, once**: an absent
//! `color_range` becomes an explicit `Limited`, not an `Unknown` that every
//! later stage has to re-guess (§21a.2).

use std::path::Path;

use bettercut_foundation::{FrameRate, MediaTime, Rational};
use rusty_ffmpeg::ffi;

use super::{InputContext, opt_cstr};
use crate::asset::{MediaAsset, MediaKind};
use crate::color::{ColorMatrix, ColorMetadata, ColorPrimaries, ColorRange, TransferFunction};
use crate::decoder::MediaProber;
use crate::error::MediaError;

/// FFmpeg's internal timebase for `AVFormatContext::duration`: microseconds.
const AV_TIME_BASE: i64 = 1_000_000;

#[derive(Debug, Default, Clone, Copy)]
pub struct FfmpegProber;

impl MediaProber for FfmpegProber {
    fn probe(&self, path: &Path) -> Result<MediaAsset, MediaError> {
        let ctx = InputContext::open(path)?;

        let mut video: Option<VideoInfo> = None;
        let mut audio: Option<AudioInfo> = None;

        for stream in ctx.streams() {
            // SAFETY: `streams()` filtered out nulls, and the stream lives as
            // long as `ctx`.
            let (codecpar, avg_frame_rate, r_frame_rate) = unsafe {
                let s = &*stream;
                (s.codecpar, s.avg_frame_rate, s.r_frame_rate)
            };
            if codecpar.is_null() {
                continue;
            }

            // SAFETY: non-null, and owned by the stream which outlives this loop.
            let par = unsafe { &*codecpar };

            if par.codec_type == ffi::AVMEDIA_TYPE_VIDEO && video.is_none() {
                video = Some(read_video(par, avg_frame_rate, r_frame_rate));
            } else if par.codec_type == ffi::AVMEDIA_TYPE_AUDIO && audio.is_none() {
                audio = Some(read_audio(par));
            }
        }

        // SAFETY: `ctx.as_ptr()` is non-null for the lifetime of `ctx`.
        let raw_duration = unsafe { (*ctx.as_ptr()).duration };
        let duration = if raw_duration > 0 {
            MediaTime::from_timebase(
                raw_duration,
                Rational::new(1, AV_TIME_BASE).unwrap_or_else(|| unreachable!("nonzero")),
            )
            .unwrap_or(MediaTime::ZERO)
        } else {
            MediaTime::ZERO
        };

        let is_image_container = ctx
            .format_name()
            .is_some_and(|name| is_image_demuxer(&name));

        let kind = match (&video, &audio) {
            // A still image is not a one-frame movie. FFmpeg's image demuxers
            // report a synthetic 25 fps rate, so the demuxer name is the
            // reliable signal, not the frame rate.
            (Some(_), _) if is_image_container => MediaKind::Image,
            (Some(_), _) => MediaKind::Video,
            (None, Some(_)) => MediaKind::Audio,
            (None, None) => return Err(MediaError::NoStream(path.to_path_buf())),
        };

        let mut asset = MediaAsset::new(kind, path, duration);
        asset.file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        if let Some(v) = video {
            // The shown size: a quarter turn swaps the sides.
            (asset.width, asset.height) = if v.rotation % 180 == 90 {
                (v.height, v.width)
            } else {
                (v.width, v.height)
            };
            asset.rotation = v.rotation;
            // A photo's orientation is not on the stream but on its one
            // decoded frame (EXIF, as FFmpeg reports it). One small decode.
            if kind == MediaKind::Image && asset.rotation == 0 {
                let turn = super::decode::FfmpegDecoder::first_frame_rotation(&asset);
                if turn % 180 == 90 {
                    (asset.width, asset.height) = (asset.height, asset.width);
                }
                asset.rotation = turn;
            }
            // A still image has no frame rate; recording the demuxer's
            // synthetic 25 fps would be a lie a later stage might act on.
            asset.frame_rate = if kind == MediaKind::Image {
                None
            } else {
                v.frame_rate
            };
            asset.video_codec = v.codec;
            // §21a.2: resolve colour now, so nothing downstream inspects source
            // metadata again.
            asset.color = v.color;
        }

        if let Some(a) = audio {
            asset.audio_codec = a.codec;
            asset.audio_sample_rate = a.sample_rate;
            asset.audio_channels = a.channels;
        }

        tracing::debug!(
            file = %asset.file_name,
            kind = ?asset.kind,
            width = asset.width,
            height = asset.height,
            duration_ticks = asset.duration.ticks(),
            "probed media"
        );

        Ok(asset)
    }
}

impl FfmpegProber {
    /// The numbered run `path` is one frame of, as one video asset at `rate`:
    /// the first frame probed for its size and codec, the count for its
    /// length. Refused for a file that is not part of a run, or not a still.
    pub fn probe_image_sequence(
        &self,
        path: &Path,
        rate: FrameRate,
    ) -> Result<MediaAsset, MediaError> {
        let sequence = crate::image_sequence::sequence_at(path)
            .ok_or_else(|| MediaError::UnsupportedFormat(path.to_path_buf()))?;
        let first = path.with_file_name(
            crate::image_sequence::numbered(path)
                .map(|parts| parts.name_of(sequence.start))
                .unwrap_or_default(),
        );
        let mut asset = self.probe(&first)?;
        if !asset.is_still() {
            return Err(MediaError::UnsupportedFormat(path.to_path_buf()));
        }
        asset.kind = MediaKind::Video;
        asset.frame_rate = Some(rate);
        asset.duration =
            MediaTime::from_frames(i64::from(sequence.count), rate).unwrap_or(MediaTime::ZERO);
        asset.sequence = Some(sequence);
        Ok(asset)
    }
}

/// Demuxers FFmpeg uses for still images.
///
/// `image2` covers numbered sequences and single files; the `*_pipe` demuxers
/// (`png_pipe`, `jpeg_pipe`, `webp_pipe`, …) cover raw single images.
fn is_image_demuxer(name: &str) -> bool {
    name == "image2" || name.ends_with("_pipe")
}

struct VideoInfo {
    width: u32,
    height: u32,
    rotation: u16,
    frame_rate: Option<FrameRate>,
    codec: Option<String>,
    color: ColorMetadata,
}

struct AudioInfo {
    codec: Option<String>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
}

fn read_video(
    par: &ffi::AVCodecParameters,
    avg_frame_rate: ffi::AVRational,
    r_frame_rate: ffi::AVRational,
) -> VideoInfo {
    let width = par.width.max(0) as u32;
    let height = par.height.max(0) as u32;
    let rotation = super::display_rotation(par);

    // `avg_frame_rate` is authoritative when present. Fall back to
    // `r_frame_rate` (the "real base" rate), which containers fill in more
    // reliably for short files.
    let frame_rate =
        rational_to_frame_rate(avg_frame_rate).or_else(|| rational_to_frame_rate(r_frame_rate));

    // SAFETY: avcodec_get_name accepts any codec id and returns a static string.
    let codec =
        unsafe { opt_cstr(ffi::avcodec_get_name(par.codec_id)) }.filter(|name| name != "none");

    VideoInfo {
        width,
        height,
        rotation,
        frame_rate,
        codec,
        color: read_color(par, height),
    }
}

fn read_audio(par: &ffi::AVCodecParameters) -> AudioInfo {
    // SAFETY: as above.
    let codec =
        unsafe { opt_cstr(ffi::avcodec_get_name(par.codec_id)) }.filter(|name| name != "none");

    AudioInfo {
        codec,
        sample_rate: (par.sample_rate > 0).then_some(par.sample_rate as u32),
        channels: (par.ch_layout.nb_channels > 0).then_some(par.ch_layout.nb_channels as u16),
    }
}

/// Resolve source colour into explicit values (§21a.2).
///
/// **Range is the one that bites.** Most camera and phone H.264 is
/// limited-range and says so nowhere. Defaulting an unspecified range to
/// `Full` produces crushed blacks and clipped whites, and the error is small
/// enough to go unnoticed until a user compares against another player — so
/// unspecified means `Limited`, per §21a.2.
pub(crate) fn resolve_color(par: &ffi::AVCodecParameters, height: u32) -> ColorMetadata {
    read_color(par, height)
}

fn read_color(par: &ffi::AVCodecParameters, height: u32) -> ColorMetadata {
    // Start from the height-based guess, then overwrite whatever the file
    // actually declares.
    let mut color = ColorMetadata::guess_from_height(height);

    // These are C enums, so the binding exposes them as integer constants
    // rather than Rust enum variants — hence `if`/`else if` rather than `match`.
    if par.color_range == ffi::AVCOL_RANGE_JPEG {
        color.range = ColorRange::Full;
    } else if par.color_range == ffi::AVCOL_RANGE_MPEG {
        color.range = ColorRange::Limited;
    }
    // AVCOL_RANGE_UNSPECIFIED keeps the guessed default: Limited.

    color.primaries = if par.color_primaries == ffi::AVCOL_PRI_BT709 {
        ColorPrimaries::Bt709
    } else if par.color_primaries == ffi::AVCOL_PRI_BT470BG {
        ColorPrimaries::Bt601Pal
    } else if par.color_primaries == ffi::AVCOL_PRI_SMPTE170M {
        ColorPrimaries::Bt601Ntsc
    } else if par.color_primaries == ffi::AVCOL_PRI_BT2020 {
        ColorPrimaries::Bt2020
    } else if par.color_primaries == ffi::AVCOL_PRI_UNSPECIFIED {
        color.primaries
    } else {
        ColorPrimaries::Unknown
    };

    color.transfer = if par.color_trc == ffi::AVCOL_TRC_BT709 {
        TransferFunction::Bt709
    } else if par.color_trc == ffi::AVCOL_TRC_IEC61966_2_1 {
        TransferFunction::Srgb
    } else if par.color_trc == ffi::AVCOL_TRC_SMPTE2084 {
        TransferFunction::Pq
    } else if par.color_trc == ffi::AVCOL_TRC_ARIB_STD_B67 {
        TransferFunction::Hlg
    } else if par.color_trc == ffi::AVCOL_TRC_UNSPECIFIED {
        color.transfer
    } else {
        TransferFunction::Unknown
    };

    color.matrix = if par.color_space == ffi::AVCOL_SPC_BT709 {
        ColorMatrix::Bt709
    } else if par.color_space == ffi::AVCOL_SPC_BT470BG
        || par.color_space == ffi::AVCOL_SPC_SMPTE170M
    {
        ColorMatrix::Bt601
    } else if par.color_space == ffi::AVCOL_SPC_BT2020_NCL {
        ColorMatrix::Bt2020Ncl
    } else if par.color_space == ffi::AVCOL_SPC_UNSPECIFIED {
        color.matrix
    } else {
        ColorMatrix::Unknown
    };

    color.bit_depth = bit_depth(par.format).unwrap_or(8);
    color
}

/// Bits per component, read from the pixel format descriptor.
///
/// Drives §13's proxy decision: anything above 8-bit needs normalizing.
fn bit_depth(format: i32) -> Option<u8> {
    if format < 0 {
        return None;
    }
    // SAFETY: av_pix_fmt_desc_get accepts any value and returns null for
    // unknown formats, which is checked below.
    let desc = unsafe { ffi::av_pix_fmt_desc_get(format) };
    if desc.is_null() {
        return None;
    }
    // SAFETY: non-null; `comp` is a fixed-size array of 4 components and
    // nb_components is at least 1 for every real format.
    unsafe {
        let d = &*desc;
        (d.nb_components > 0).then(|| d.comp[0].depth as u8)
    }
}

/// An `AVRational` frame rate, or `None` when it is absent or nonsensical.
fn rational_to_frame_rate(r: ffi::AVRational) -> Option<FrameRate> {
    if r.num <= 0 || r.den <= 0 {
        return None;
    }
    let rate = FrameRate::new(r.num as i64, r.den as i64)?;
    // Guard against containers reporting 1000 fps for still images or broken
    // headers — placing such a clip would produce nonsense timing.
    (rate.as_f64() > 0.0 && rate.as_f64() <= 1000.0).then_some(rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn probe(name: &str) -> MediaAsset {
        FfmpegProber
            .probe(&fixture(name))
            .unwrap_or_else(|e| panic!("probing {name} failed: {e}"))
    }

    /// A phone photo's EXIF orientation is read from its frame.
    #[test]
    fn an_exif_rotated_photo_probes_as_portrait() {
        let asset = probe("exif-6.jpg");
        assert_eq!(asset.rotation, 90);
        assert_eq!((asset.width, asset.height), (32, 64));
        assert_eq!(probe("still.png").rotation, 0);
    }

    /// The display rotation is read, and the size is the size as shown.
    #[test]
    fn a_rotated_file_probes_as_portrait() {
        let asset = probe("rotated-90.mp4");
        assert_eq!(asset.rotation, 270, "FFmpeg's +90 is anticlockwise");
        assert_eq!((asset.width, asset.height), (360, 640));
        assert_eq!(probe("ntsc-2997.mp4").rotation, 0);
    }

    #[test]
    fn probes_an_ntsc_video_exactly() {
        let asset = probe("ntsc-2997.mp4");

        assert_eq!(asset.kind, MediaKind::Video);
        assert_eq!((asset.width, asset.height), (640, 360));
        assert_eq!(asset.video_codec.as_deref(), Some("h264"));

        // The point of §9: 29.97 must survive as an exact rational.
        let rate = asset.frame_rate.expect("frame rate");
        assert_eq!(rate.as_rational().num(), 30_000);
        assert_eq!(rate.as_rational().den(), 1001);
        assert_eq!(
            bettercut_foundation::ticks_per_frame(rate),
            Some(32_032),
            "probed rate does not divide the timebase"
        );

        // 2 seconds, within one frame.
        let expected = MediaTime::from_seconds(2).ticks();
        assert!(
            (asset.duration.ticks() - expected).abs() < 32_032,
            "duration {} not within a frame of {expected}",
            asset.duration.ticks()
        );
    }

    /// §21a.2's headline bug. This file is tagged limited-range, and the probe
    /// must say so rather than defaulting to full.
    #[test]
    fn detects_limited_range() {
        let asset = probe("ntsc-2997.mp4");
        assert_eq!(asset.color.range, ColorRange::Limited);
        assert!(asset.color.range.needs_expansion());
        assert_eq!(asset.color.bit_depth, 8);
    }

    #[test]
    fn probes_audio_streams() {
        let asset = probe("ntsc-2997.mp4");
        assert_eq!(asset.audio_codec.as_deref(), Some("aac"));
        assert_eq!(asset.audio_sample_rate, Some(48_000));
        assert_eq!(asset.audio_channels, Some(2));
    }

    #[test]
    fn probes_an_audio_only_file_as_audio() {
        let asset = probe("tone-48k.wav");
        assert_eq!(asset.kind, MediaKind::Audio);
        assert_eq!(asset.audio_sample_rate, Some(48_000));
        assert_eq!(asset.width, 0);
        assert!(asset.frame_rate.is_none());
        assert!(asset.duration.ticks() > 0);
    }

    #[test]
    fn probes_a_still_image_as_image() {
        let asset = probe("still.png");
        assert_eq!(asset.kind, MediaKind::Image);
        assert_eq!((asset.width, asset.height), (320, 180));
    }

    /// §13's proxy rules, driven by real probed metadata rather than fixtures.
    #[test]
    fn a_probed_sd_h264_file_does_not_need_a_proxy() {
        let asset = probe("ntsc-2997.mp4");
        assert!(!asset.should_generate_proxy());
    }

    #[test]
    fn probing_a_missing_file_reports_not_found() {
        let result = FfmpegProber.probe(&fixture("nope.mp4"));
        assert!(matches!(result, Err(MediaError::FileNotFound(_))));
    }

    #[test]
    fn probing_the_same_file_twice_is_stable() {
        // Repeated probes must not leak handles or drift; this also fails if
        // avformat_close_input is ever dropped from the Drop impl on Windows,
        // because the second open would hit a locked file.
        let first = probe("ntsc-2997.mp4");
        for _ in 0..20 {
            let again = probe("ntsc-2997.mp4");
            assert_eq!(again.width, first.width);
            assert_eq!(again.duration, first.duration);
        }
    }
}
