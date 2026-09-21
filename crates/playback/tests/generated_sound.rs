//! Made sound through the mixer: a tone placed on a lane comes out of the mix
//! at the pitch and level it promises (`bettercut_media::generated_sound`).
//!
//! The module's own tests prove the samples are right. These prove the mixer
//! *uses* them — that a clip with no file behind it is read from the formula
//! rather than skipped as missing media, and that everything downstream (gain,
//! lanes, the meters) treats it as ordinary sound.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{AUDIO_SAMPLE_RATE, MediaTime, TICKS_PER_AUDIO_SAMPLE, TimelineTime};
use bettercut_media::{GeneratedSound, MediaAsset};
use bettercut_playback::{AudioMixer, AudioPlan};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioClip, SourceRange};

/// A project with `sound` on the first sound lane from timeline zero.
fn project_with(sound: GeneratedSound) -> Project {
    let mut project = Project::new("Made");
    let media = project.add_media(MediaAsset::generated_sound(
        sound,
        MediaTime::from_seconds(60),
    ));
    let clip = AudioClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    project.active_mut().unwrap().audio_tracks[0]
        .insert(clip)
        .unwrap();
    project
}

/// Mix `frames` from the start, in blocks of `block`, as interleaved stereo.
fn mix(project: &Project, frames: usize, block: usize) -> Vec<f32> {
    let sequence = project.active().unwrap();
    let plan = AudioPlan::of(project, sequence);
    let mut mixer = AudioMixer::new(1);
    let mut out = Vec::new();
    let mut done = 0;
    while done < frames {
        let want = block.min(frames - done);
        let mut buffer = vec![0.0_f32; want * 2];
        let at = TimelineTime::from_ticks(done as i64 * TICKS_PER_AUDIO_SAMPLE);
        mixer.mix_block(&plan, at, want, 2, &mut buffer);
        out.extend_from_slice(&buffer);
        done += want;
    }
    out
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |most, s| most.max(s.abs()))
}

/// The line-up tone reaches the mix at the level it is for: -18 dBFS, less
/// the centre pan the mixer applies to every mono-equal source.
#[test]
fn a_placed_tone_is_heard_at_its_stated_level() {
    let project = project_with(GeneratedSound::LINE_UP);
    let block = mix(&project, AUDIO_SAMPLE_RATE as usize / 4, 1_024);

    let expected =
        10.0_f32.powf(bettercut_media::LINE_UP_DB / 20.0) * std::f32::consts::FRAC_1_SQRT_2;
    assert!(
        (peak(&block) - expected).abs() < 0.005,
        "{} against {expected}",
        peak(&block)
    );
}

/// And at its pitch: a quarter second of 1 kHz rises through zero 250 times
/// on one side.
#[test]
fn a_placed_tone_is_heard_at_its_pitch() {
    let project = project_with(GeneratedSound::LINE_UP);
    let block = mix(&project, AUDIO_SAMPLE_RATE as usize / 4, 4_800);

    let left: Vec<f32> = block.chunks(2).map(|frame| frame[0]).collect();
    let rises = left
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    assert_eq!(rises, bettercut_media::LINE_UP_HZ as usize / 4);
}

/// §46: the same samples however the block is cut up. Made sound has no
/// decoder to keep in step, so this is the whole of what keeps preview and
/// export together for it.
#[test]
fn the_block_size_makes_no_difference() {
    let project = project_with(GeneratedSound::LINE_UP);
    assert_eq!(mix(&project, 9_600, 4_800), mix(&project, 9_600, 441));
}

/// Silence placed on a lane is silent — and still a clip, which is the point
/// of placing it.
#[test]
fn placed_silence_is_silent() {
    let project = project_with(GeneratedSound::Silence);
    let block = mix(&project, 4_800, 1_024);
    assert_eq!(block.len(), 4_800 * 2);
    assert!(block.iter().all(|sample| *sample == 0.0));
}

/// A tone is an ordinary clip: the lane's volume works on it like anything
/// else, which is how a tone gets laid in at the level a delivery wants.
#[test]
fn a_tone_obeys_the_lane_it_is_on() {
    let mut project = project_with(GeneratedSound::LINE_UP);
    let loud = peak(&mix(&project, 4_800, 1_024));

    project.active_mut().unwrap().audio_tracks[0].gain = 0.5;
    let quiet = peak(&mix(&project, 4_800, 1_024));

    assert!(
        (quiet - loud / 2.0).abs() < 0.002,
        "half volume gave {quiet} against {loud}"
    );
}

/// The lane meters read a made tone like any other sound: it is being heard,
/// so it shows (`bettercut_playback::lane_meters`).
#[test]
fn a_tone_registers_on_its_lanes_meter() {
    let project = project_with(GeneratedSound::LINE_UP);
    let sequence = project.active().unwrap();
    let plan = AudioPlan::of(&project, sequence);
    let mut mixer = AudioMixer::new(1);
    let mut buffer = vec![0.0_f32; 1_024 * 2];
    mixer.mix_block(&plan, TimelineTime::ZERO, 1_024, 2, &mut buffer);

    let lanes = mixer.lane_peaks();
    assert!(!lanes.is_empty(), "no lane reported anything");
    assert!(lanes[0].0 > 0.0 && lanes[0].1 > 0.0, "{:?}", lanes[0]);
}
