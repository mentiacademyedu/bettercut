//! Numbered stills as one clip (`bettercut_media::image_sequence`): found
//! from any frame, probed as a video, and decoded frame by frame in order.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_foundation::{FrameRate, MediaTime};
use bettercut_media::{
    FfmpegDecoder, FfmpegProber, FrameStorage, MediaDecoder, MediaKind, NeverCancelled, sequence_at,
};

/// A 2×2 PPM, one flat colour: the simplest still FFmpeg reads.
fn write_frame(dir: &Path, number: u32, red: u8) -> PathBuf {
    let mut ppm = b"P6\n2 2\n255\n".to_vec();
    for _ in 0..4 {
        ppm.extend_from_slice(&[red, 40, 20]);
    }
    let path = dir.join(format!("frame_{number:04}.ppm"));
    std::fs::write(&path, ppm).unwrap();
    path
}

/// Frames 3–7, a gap, then 9: picking 5 finds 3 through 7.
#[test]
fn the_run_around_a_frame_stops_at_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    for number in [3, 4, 5, 6, 7, 9] {
        write_frame(dir.path(), number, 100);
    }
    let picked = dir.path().join("frame_0005.ppm");
    let sequence = sequence_at(&picked).unwrap();
    assert_eq!((sequence.start, sequence.count, sequence.digits), (3, 5, 4));

    let alone = dir.path().join("frame_0009.ppm");
    assert!(sequence_at(&alone).is_none(), "one still is a still");
}

/// Probed as a video the length of its frames at the rate asked for, the
/// size of its first frame.
#[test]
fn a_sequence_probes_as_a_video_at_its_rate() {
    let dir = tempfile::tempdir().unwrap();
    for number in 1..=5 {
        write_frame(dir.path(), number, 100);
    }
    let asset = FfmpegProber
        .probe_image_sequence(&dir.path().join("frame_0002.ppm"), FrameRate::PAL_25)
        .unwrap();
    assert_eq!(asset.kind, MediaKind::Video);
    assert!(!asset.is_still());
    assert_eq!(asset.frame_rate, Some(FrameRate::PAL_25));
    assert_eq!(asset.duration, MediaTime::from_millis(200));
    assert_eq!((asset.width, asset.height), (2, 2));
    assert_eq!(asset.sequence.unwrap().count, 5);
    // The asset's own path is the first frame, a real file.
    assert!(asset.path.ends_with("frame_0001.ppm"));

    let lone = write_frame(dir.path(), 40, 1);
    assert!(
        FfmpegProber
            .probe_image_sequence(&lone, FrameRate::PAL_25)
            .is_err(),
        "a lone still is not a sequence"
    );
}

/// Decoding walks the frames in order, one a frame at the rate, each its own
/// picture — starting from the first frame's number, not from zero.
#[test]
fn a_sequence_decodes_its_frames_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let reds = [10u8, 60, 110, 160, 210];
    for (index, red) in reds.iter().enumerate() {
        write_frame(dir.path(), 100 + index as u32, *red);
    }
    let asset = FfmpegProber
        .probe_image_sequence(&dir.path().join("frame_0102.ppm"), FrameRate::PAL_25)
        .unwrap();
    let mut decoder = FfmpegDecoder::new(1).unwrap();
    decoder.open(&asset).unwrap();

    let mut seen = Vec::new();
    while let Some(frame) = decoder.decode_frame(&NeverCancelled).unwrap() {
        let FrameStorage::System { data, .. } = &frame.storage else {
            panic!("expected a RAM frame");
        };
        seen.push((frame.timestamp, data[0]));
        if seen.len() > reds.len() {
            break;
        }
    }
    assert_eq!(seen.len(), reds.len(), "{seen:?}");
    for (index, (timestamp, red)) in seen.iter().enumerate() {
        assert_eq!(
            *timestamp,
            MediaTime::from_frames(index as i64, FrameRate::PAL_25).unwrap(),
            "frame {index} at the wrong time: {seen:?}"
        );
        assert!(
            red.abs_diff(reds[index]) <= 2,
            "frame {index} is the wrong picture: {seen:?}"
        );
    }
}
