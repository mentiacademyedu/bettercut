//! The web build's MP4 writer, read back by FFmpeg: a real file's H.264 and
//! AAC, taken out of it packet by packet and put into a new file by
//! `mp4_mux`, must open, last as long, and decode to every frame.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ffi::CString;
use std::path::PathBuf;

use bettercut_media::mp4_mux::{Mp4Sample, Mp4Track, Mp4TrackKind, mux};
use bettercut_media::{FfmpegDecoder, FfmpegProber, MediaDecoder, MediaProber, NeverCancelled};
use rusty_ffmpeg::ffi;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Every packet of the file's first picture and sound streams, as tracks.
fn tracks_of(path: &std::path::Path) -> Vec<Mp4Track> {
    let name = CString::new(path.to_str().unwrap()).unwrap();
    let mut input: *mut ffi::AVFormatContext = std::ptr::null_mut();
    // SAFETY: plain FFmpeg reading, every pointer checked or owned here, and
    // everything opened is closed before returning.
    unsafe {
        assert_eq!(
            ffi::avformat_open_input(&mut input, name.as_ptr(), std::ptr::null(), std::ptr::null_mut()),
            0
        );
        assert!(ffi::avformat_find_stream_info(input, std::ptr::null_mut()) >= 0);

        let streams = std::slice::from_raw_parts((*input).streams, (*input).nb_streams as usize);
        let mut tracks: Vec<(i32, Mp4Track)> = Vec::new();
        for (index, &stream) in streams.iter().enumerate() {
            let par = &*(*stream).codecpar;
            let base = (*stream).time_base;
            assert_eq!(base.num, 1, "a time base this test can use as a timescale");
            let extra = if par.extradata.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(par.extradata, par.extradata_size as usize).to_vec()
            };
            let kind = if par.codec_type == ffi::AVMEDIA_TYPE_VIDEO
                && par.codec_id == ffi::AV_CODEC_ID_H264
                && !tracks.iter().any(|(_, t)| matches!(t.kind, Mp4TrackKind::Video { .. }))
            {
                Mp4TrackKind::Video {
                    width: par.width as u32,
                    height: par.height as u32,
                    avcc: extra,
                }
            } else if par.codec_type == ffi::AVMEDIA_TYPE_AUDIO
                && par.codec_id == ffi::AV_CODEC_ID_AAC
                && !tracks.iter().any(|(_, t)| matches!(t.kind, Mp4TrackKind::Aac { .. }))
            {
                Mp4TrackKind::Aac {
                    channels: par.ch_layout.nb_channels as u16,
                    sample_rate: par.sample_rate as u32,
                    config: extra,
                    bitrate: par.bit_rate as u32,
                }
            } else {
                continue;
            };
            tracks.push((
                index as i32,
                Mp4Track {
                    kind,
                    timescale: base.den as u32,
                    samples: Vec::new(),
                },
            ));
        }

        let mut packet = ffi::av_packet_alloc();
        while ffi::av_read_frame(input, packet) >= 0 {
            let p = &*packet;
            if let Some((_, track)) = tracks.iter_mut().find(|(i, _)| *i == p.stream_index) {
                track.samples.push(Mp4Sample {
                    data: std::slice::from_raw_parts(p.data, p.size as usize).to_vec(),
                    duration: p.duration as u32,
                    sync: p.flags & ffi::AV_PKT_FLAG_KEY as i32 != 0,
                });
            }
            ffi::av_packet_unref(packet);
        }
        ffi::av_packet_free(&mut packet);
        ffi::avformat_close_input(&mut input);
        tracks.into_iter().map(|(_, t)| t).collect()
    }
}

#[test]
fn a_remuxed_file_opens_lasts_as_long_and_decodes_every_frame() {
    let source = fixture("ntsc-2997.mp4");
    let tracks = tracks_of(&source);
    assert!(
        tracks.iter().any(|t| matches!(t.kind, Mp4TrackKind::Video { .. })),
        "the fixture has H.264"
    );
    let frames = tracks
        .iter()
        .find(|t| matches!(t.kind, Mp4TrackKind::Video { .. }))
        .map(|t| t.samples.len())
        .unwrap();

    let out = std::env::temp_dir().join("bettercut-mp4-mux-test.mp4");
    std::fs::write(&out, mux(&tracks)).unwrap();

    let original = FfmpegProber.probe(&source).expect("probe the fixture");
    let written = FfmpegProber.probe(&out).expect("probe the written file");
    assert_eq!((written.width, written.height), (original.width, original.height));
    assert!(original.audio_channels.is_some(), "the fixture has sound");
    assert!(
        tracks.iter().any(|t| matches!(t.kind, Mp4TrackKind::Aac { .. })),
        "the fixture's sound is AAC"
    );
    assert_eq!(written.audio_channels, original.audio_channels);
    let drift = (written.duration.ticks() - original.duration.ticks()).abs();
    assert!(
        drift < original.duration.ticks() / 20,
        "{:?} against {:?}",
        written.duration,
        original.duration
    );

    let mut decoder = FfmpegDecoder::new(2).unwrap();
    decoder.open(&written).expect("open the written file");
    let mut decoded = 0;
    while decoder.decode_frame(&NeverCancelled).expect("decode").is_some() {
        decoded += 1;
    }
    let _ = std::fs::remove_file(&out);
    assert_eq!(decoded, frames);
}
