//! Removing flicker from a real file, through the real job scheduler: a file
//! whose brightness pulses is encoded, measured, and the gains that come back
//! must even the pulse out — darkening the bright frames, brightening the
//! dim ones — while a file that does not flicker is left alone.
//!
//! Generated rather than committed, as `scene_detection.rs` explains: the
//! pulse is two lines of code here, so what the test expects is visible.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_foundation::{FrameRate, MediaTime};
use bettercut_jobs::{JobEvent, JobScheduler};
use bettercut_media::{
    ExportFormat, FfmpegProber, MediaProber, RateControl, VideoCodec, VideoWriter,
};
use bettercut_playback::FlickerJob;
use bettercut_timeline::deflicker::FlickerGains;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FPS: i64 = 30;

/// A file that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("bettercut-flicker-{name}.mp4"));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Two seconds of a grey ground with a bar drifting across it, the ground's
/// level for each frame from `level`.
fn encode(path: &Path, level: impl Fn(i64) -> u8) {
    let mut writer = VideoWriter::create(
        path,
        ExportFormat {
            transparent: false,
            prores: false,
            width: WIDTH,
            height: HEIGHT,
            frame_rate: FrameRate::FPS_30,
            codec: VideoCodec::H264,
            bitrate: Some(4_000_000),
            rate_control: RateControl::Variable,
            channels: 0,
            audio_bitrate: None,
            threads: 2,
        },
    )
    .expect("open the generated file");
    let mut rgba = vec![0_u8; (WIDTH * HEIGHT * 4) as usize];
    for index in 0..(2 * FPS) {
        rgba.fill(level(index));
        let at = 40 + index as u32 * 2;
        for y in 60..120 {
            for x in at..at + 30 {
                let pixel = ((y * WIDTH + x) * 4) as usize;
                rgba[pixel..pixel + 4].copy_from_slice(&[200, 120, 60, 255]);
            }
        }
        writer.push_frame(&rgba).expect("write a frame");
    }
    writer.finish().expect("finish the generated file");
}

/// Run a job to completion and give back the gains.
fn measure(path: &Path) -> FlickerGains {
    let asset = FfmpegProber.probe(path).expect("probe the generated file");
    let (scheduler, events) = JobScheduler::new(1);
    let (job, report) = FlickerJob::new(
        bettercut_foundation::ClipId::new(),
        &asset,
        MediaTime::ZERO,
        asset.duration,
        2,
    )
    .expect("a video file");
    let id = scheduler.submit(Box::new(job));
    loop {
        match events.recv().expect("the scheduler hung up") {
            JobEvent::Finished { id: done } if done == id => break,
            JobEvent::Failed { message, .. } => panic!("the job failed: {message}"),
            _ => {}
        }
    }
    report
        .take()
        .expect("a finished job has an answer")
        .expect("gains")
}

/// Where frame `index` is in the file.
fn frame(index: i64) -> MediaTime {
    MediaTime::from_ticks(index * MediaTime::from_seconds(1).ticks() / FPS)
}

#[test]
fn a_pulsing_file_is_evened_out() {
    let file = Scratch::new("pulse");
    // Two frames bright, two dim: lights beating against the frame rate.
    let pulse = |index: i64| if index % 4 < 2 { 150 } else { 110 };
    encode(&file.0, pulse);
    let gains = measure(&file.0);
    assert_eq!(gains.gains.len(), 60, "a gain for every frame");
    for index in 12..48 {
        let gain = gains.gain_at(frame(index));
        if pulse(index) == 150 {
            assert!(gain < 0.9, "bright frame {index} was not darkened: {gain}");
        } else {
            assert!(gain > 1.1, "dim frame {index} was not brightened: {gain}");
        }
    }
}

#[test]
fn a_steady_file_is_left_alone() {
    let file = Scratch::new("steady");
    encode(&file.0, |_| 130);
    let gains = measure(&file.0);
    for index in 0..60 {
        let gain = gains.gain_at(frame(index));
        assert!((gain - 1.0).abs() < 0.02, "frame {index} got {gain}");
    }
}
