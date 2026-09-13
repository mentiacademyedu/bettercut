//! Reading sound after a seek (`AudioSource::read`).
//!
//! Sound read after jumping back must be the same samples as sound read by
//! playing up to that point: an editor rewinds constantly — scrubbing,
//! looping, and reversed clips, which rewind on every block.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_foundation::MediaTime;
use bettercut_media::{FfmpegDecoder, FfmpegProber, MediaProber};
use bettercut_playback::AudioSource;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

fn open() -> AudioSource {
    let asset = FfmpegProber.probe(&fixture("ntsc-2997.mp4")).unwrap();
    AudioSource::open(&asset, Box::new(FfmpegDecoder::new(1).unwrap())).unwrap()
}

fn worst(a: &[Vec<f32>], b: &[Vec<f32>]) -> f32 {
    a[0].iter()
        .zip(&b[0])
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

/// Rewinding to an instant reads the same samples as arriving there by
/// playing — anywhere in the file, including its very start.
#[test]
fn a_rewind_reads_the_same_samples_as_playing_there() {
    for at in [0_i64, 100, 250, 500] {
        let from = MediaTime::from_millis(at);

        let mut played = open();
        if at > 0 {
            played.read(MediaTime::ZERO, (at * 48) as usize).unwrap();
        }
        let reference = played.read(from, 4_800).unwrap();

        let mut rewound = open();
        rewound.read(MediaTime::from_millis(800), 4_800).unwrap();
        let after_rewind = rewound.read(from, 4_800).unwrap();

        assert_eq!(reference[0].len(), after_rewind[0].len(), "at {at} ms");
        let error = worst(&reference, &after_rewind);
        assert!(
            error < 0.01,
            "at {at} ms a rewind read different sound: off by {error}"
        );
    }
}
