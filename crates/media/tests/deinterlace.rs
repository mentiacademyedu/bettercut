//! Deinterlacing (`MediaAsset::deinterlace`): yadif in front of the scaler,
//! one frame out per frame in, so a flagged file decodes exactly as many
//! frames as before, at the same size and colour.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_media::{
    FfmpegDecoder, FfmpegProber, FrameStorage, MediaDecoder, MediaProber, NeverCancelled,
};

#[test]
fn a_flagged_still_decodes_whole_and_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let (w, h) = (8u32, 6u32);
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for _ in 0..w * h {
        ppm.extend_from_slice(&[200, 40, 20]);
    }
    let path = dir.path().join("frame.ppm");
    std::fs::write(&path, ppm).unwrap();

    let mut asset = FfmpegProber.probe(&path).unwrap();
    assert!(!asset.deinterlace, "off unless asked");
    asset.deinterlace = true;

    let mut decoder = FfmpegDecoder::new(1).unwrap();
    decoder.open(&asset).unwrap();
    let frame = decoder
        .decode_frame(&NeverCancelled)
        .unwrap()
        .expect("a frame");
    assert_eq!((frame.width, frame.height), (w, h));
    let FrameStorage::System { data, .. } = &frame.storage else {
        panic!("expected a RAM frame");
    };
    for (got, want) in data[..3].iter().zip([200u8, 40, 20]) {
        assert!(got.abs_diff(want) <= 3, "colour changed: {:?}", &data[..4]);
    }
}
