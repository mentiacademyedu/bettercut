//! Mixing one sound lane down to a file (`bettercut_export::bounce`).
//!
//! The claims: a bounce holds the lane it names and nothing from the lanes
//! beside it; it is as long as the lane's own sound, not as long as the
//! sequence; it carries the lane's level, because that level is reset when the
//! clip is put down; and the master volume is left out, because it is applied
//! again on the way to the speakers.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_export::{
    ExportProgress, ExportSettings, VideoCodec, export_sound, only_this_lane, track_span,
};
use bettercut_foundation::{FrameRate, MediaTime, TimelineTime, TrackId};
use bettercut_media::{FfmpegProber, MediaProber, NeverCancelled};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioClip, AudioTrack, Resolution, SourceRange, TimelineRange};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("bettercut-bounce-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The fixture's sound on A1 from two seconds in, and a second, empty lane.
fn project_with_two_lanes() -> (Project, TrackId, TrackId) {
    let mut project = Project::new("Bounce");
    let asset = FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    let duration = asset.duration;
    let media = project.add_media(asset);

    let sequence = project.active_mut().unwrap();
    sequence.audio_tracks.push(AudioTrack::new("A2"));
    sequence.audio_tracks[0]
        .insert(
            AudioClip::new(
                media,
                TimelineTime::from_seconds(2),
                SourceRange::new(MediaTime::ZERO, duration).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let (first, second) = (sequence.audio_tracks[0].id, sequence.audio_tracks[1].id);
    (project, first, second)
}

fn settings(path: &Path, range: TimelineRange) -> ExportSettings {
    ExportSettings {
        transparent: false,
        path: path.to_path_buf(),
        resolution: Resolution::new(640, 360),
        frame_rate: FrameRate::NTSC_29_97,
        codec: VideoCodec::H264,
        bitrate: None,
        rate_control: bettercut_export::RateControl::Variable,
        range,
        threads: 1,
        sound_only: true,
        picture_only: false,
        gif: false,
        image_sequence: false,
        loudness_target: None,
        audio_bitrate: None,
    }
}

/// Write the bounce of `track` and hand back its samples.
fn bounce(project: &Project, track: TrackId, path: &Path) -> Vec<i16> {
    let sequence = project.active().unwrap().id;
    let range = track_span(project, sequence, track).expect("a span");
    let silenced = only_this_lane(project.clone(), sequence, track).expect("the lane");
    export_sound(
        &silenced,
        silenced.active().unwrap(),
        &settings(path, range),
        &mut |_: ExportProgress| {},
        &NeverCancelled,
    )
    .expect("bounced");
    let bytes = std::fs::read(path).expect("read back");
    bytes[44..]
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

fn loudest(samples: &[i16]) -> i16 {
    samples.iter().copied().map(i16::abs).max().unwrap_or(0)
}

/// The span is the lane's own sound, not the sequence: a bounce of a lane that
/// starts two seconds in starts two seconds in.
#[test]
fn the_span_is_the_lanes_own_sound() {
    let (project, first, second) = project_with_two_lanes();
    let sequence = project.active().unwrap().id;

    let span = track_span(&project, sequence, first).expect("a span");
    assert_eq!(span.start, TimelineTime::from_seconds(2));
    assert!(span.end > span.start);

    assert!(
        track_span(&project, sequence, second).is_none(),
        "an empty lane has nothing to bounce"
    );
}

/// Every other lane is silenced, and the one being bounced is turned on
/// however it was left — including through someone else's solo.
#[test]
fn only_the_lane_being_bounced_is_heard() {
    let (mut project, first, second) = project_with_two_lanes();
    let sequence = project.active().unwrap().id;
    {
        let active = project.active_mut().unwrap();
        active.audio_tracks[0].enabled = false;
        active.audio_tracks[1].solo = true;
        active.master_volume = 0.25;
    }

    let silenced = only_this_lane(project.clone(), sequence, first).expect("the lane");
    let lanes = &silenced.active().unwrap().audio_tracks;
    assert!(lanes[0].enabled, "the lane being bounced must be heard");
    assert!(!lanes[0].solo);
    assert!(!lanes[1].enabled, "the other lane must not be");
    assert!(!lanes[1].solo, "and its solo must not silence the bounce");
    assert_eq!(
        silenced.active().unwrap().master_volume,
        1.0,
        "the master is applied again later"
    );

    assert!(
        only_this_lane(project, sequence, second).is_some(),
        "an empty lane is still a lane"
    );
}

/// The file holds the sound of the lane, and the lane's own level is in it.
#[test]
fn the_bounce_carries_the_lanes_level() {
    let (mut project, first, _second) = project_with_two_lanes();
    let loud = Scratch::new("loud.wav");
    let quiet = Scratch::new("quiet.wav");

    let full = bounce(&project, first, &loud.0);
    assert!(loudest(&full) > 1_000, "the bounce came out silent");

    project.active_mut().unwrap().audio_tracks[0].gain = 0.25;
    let turned_down = bounce(&project, first, &quiet.0);

    assert_eq!(
        full.len(),
        turned_down.len(),
        "the same stretch, however loud"
    );
    let (before, after) = (f32::from(loudest(&full)), f32::from(loudest(&turned_down)));
    assert!(
        (after / before - 0.25).abs() < 0.05,
        "the lane's gain did not reach the file: {before} then {after}"
    );
}

/// And the lanes beside it are not in it.
#[test]
fn a_bounce_of_an_empty_lane_is_silence() {
    let (project, _first, second) = project_with_two_lanes();
    let sequence = project.active().unwrap().id;
    let scratch = Scratch::new("empty.wav");

    // Nothing on it, so there is no span — and nothing is written.
    assert!(track_span(&project, sequence, second).is_none());

    // Given a range anyway, what comes out is silence: the other lane is off.
    let silenced = only_this_lane(project, sequence, second).expect("the lane");
    export_sound(
        &silenced,
        silenced.active().unwrap(),
        &settings(
            &scratch.0,
            TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_seconds(1)).unwrap(),
        ),
        &mut |_: ExportProgress| {},
        &NeverCancelled,
    )
    .expect("written");
    let bytes = std::fs::read(&scratch.0).expect("read back");
    let samples: Vec<i16> = bytes[44..]
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    assert_eq!(loudest(&samples), 0, "a lane that is off is not silent");
}
