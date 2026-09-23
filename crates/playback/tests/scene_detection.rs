//! Scene detection against a real file, through the real job scheduler.
//!
//! The unit tests in `playback::scenes` feed the detector digests directly,
//! which proves the rule but not the plumbing: decoding, seeking, sampling and
//! the timestamps that come back. This encodes a file whose cuts are known to
//! the frame and asks for them back.
//!
//! The file is generated rather than committed. A fixture with real cuts in it
//! would be a large binary in the repository whose cuts nobody could check by
//! reading; here the shots are three lines of code, so what the test expects is
//! visible in the test.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_foundation::{FrameRate, MediaTime};
use bettercut_jobs::{JobEvent, JobScheduler};
use bettercut_media::{
    ExportFormat, FfmpegProber, MediaProber, RateControl, VideoCodec, VideoWriter,
};
use bettercut_playback::{SceneJob, SceneSettings};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FPS: i64 = 30;

/// A file that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("bettercut-scenes-{name}.mp4"));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// One frame: a bright bar `at` pixels from the left on a ground of `ground`.
///
/// Moving the bar is what a shot does from frame to frame; changing the ground
/// as well is what a cut does.
fn frame(ground: u8, at: u32, rgba: &mut [u8]) {
    rgba.fill(ground);
    let bar = WIDTH / 6;
    for y in 0..HEIGHT {
        for x in at..(at + bar).min(WIDTH) {
            let index = ((y * WIDTH + x) * 4) as usize;
            rgba[index..index + 4].copy_from_slice(&[240, 240, 240, 255]);
        }
    }
}

/// Encode `shots` seconds-long shots, each with its own ground, and the bar
/// drifting within each one so that "something is moving" is not by itself a
/// cut.
fn three_shots(path: &Path, shots: &[u8], seconds: i64) {
    let mut writer = VideoWriter::create(
        path,
        ExportFormat {
            transparent: false,
            width: WIDTH,
            height: HEIGHT,
            frame_rate: FrameRate::FPS_30,
            codec: VideoCodec::H264,
            bitrate: Some(3_000_000),
            rate_control: RateControl::Variable,
            channels: 0,
            audio_bitrate: None,
            threads: 2,
        },
    )
    .expect("open the generated file");

    let mut rgba = vec![0_u8; (WIDTH * HEIGHT * 4) as usize];
    for (shot, &ground) in shots.iter().enumerate() {
        for index in 0..(seconds * FPS) {
            // A slow drift across the shot, starting from a different place in
            // each: enough movement to be footage, far less than a cut.
            let at = (WIDTH / 4) + (shot as u32 * 20) + (index as u32);
            frame(ground, at.min(WIDTH - WIDTH / 6 - 1), &mut rgba);
            writer.push_frame(&rgba).expect("write a frame");
        }
    }
    writer.finish().expect("finish the generated file");
}

/// Run a job to completion and give back its report.
fn detect(path: &Path, settings: SceneSettings) -> Vec<MediaTime> {
    let asset = FfmpegProber.probe(path).expect("probe the generated file");
    let duration = asset.duration;
    let (scheduler, events) = JobScheduler::new(1);
    let (job, report) =
        SceneJob::new(&asset, MediaTime::ZERO, duration, settings, 2).expect("a video file");

    let id = scheduler.submit(Box::new(job));
    loop {
        match events.recv().expect("the scheduler hung up") {
            JobEvent::Finished { id: done } if done == id => break,
            JobEvent::Failed { message, .. } => panic!("the job failed: {message}"),
            _ => {}
        }
    }
    report.cuts().expect("a finished job has an answer")
}

#[test]
fn the_cuts_in_a_file_are_found_where_they_are() {
    let file = Scratch::new("three-shots");
    three_shots(file.path(), &[20, 140, 60], 1);

    let cuts = detect(file.path(), SceneSettings::default());
    let millis: Vec<i64> = cuts.iter().map(|t| t.ticks() * 1000 / 960_000).collect();
    assert_eq!(cuts.len(), 2, "expected two cuts, got {millis:?} ms");

    // Inside a frame of the shot boundary — 20 ms is less than the 33 ms
    // between frames, so a cut reported one frame either side of the right one
    // fails here. That is the mistake worth catching: a split placed a frame
    // late leaves one frame of the new shot at the end of the old clip, which
    // is exactly the flash frame the whole thing is supposed to remove.
    for (found, expected) in millis.iter().zip([1000_i64, 2000]) {
        assert!(
            (found - expected).abs() <= 20,
            "a cut landed at {found} ms, not near {expected} ms (all: {millis:?})"
        );
    }
}

#[test]
fn one_continuous_shot_has_no_cuts() {
    let file = Scratch::new("one-shot");
    three_shots(file.path(), &[20], 3);

    let cuts = detect(file.path(), SceneSettings::default());
    assert!(
        cuts.is_empty(),
        "a single moving shot was cut into pieces at {cuts:?}"
    );
}

#[test]
fn a_still_image_has_nothing_to_detect() {
    let asset = bettercut_media::MediaAsset::new(
        bettercut_media::MediaKind::Image,
        "C:/photos/a.jpg",
        MediaTime::ZERO,
    );
    assert!(
        SceneJob::new(
            &asset,
            MediaTime::ZERO,
            MediaTime::from_seconds(5),
            SceneSettings::default(),
            1,
        )
        .is_none(),
        "a photo was queued for scene detection"
    );
}
