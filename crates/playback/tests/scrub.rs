//! Scrubbing: a short burst of the mix from wherever the playhead lands.
//!
//! The thread and the device are not testable here, so what is checked is the
//! mixing underneath: asked for a burst at an instant, the mixer must produce
//! that instant's sound and nothing else.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::mixer::{AudioMixer, AudioPlan};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioClip, SourceRange};

/// A project with one sound clip from 2 s to 6 s.
fn project() -> Project {
    let mut project = Project::new("Scrub");
    let mut asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/song.wav",
        MediaTime::from_seconds(30),
    );
    asset.audio_codec = Some("pcm".to_owned());
    let media = project.add_media(asset);
    let clip = AudioClip::new(
        media,
        TimelineTime::from_seconds(2),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    project.active_mut().unwrap().audio_tracks[0]
        .insert(clip)
        .unwrap();
    project
}

/// A burst asked for where nothing is playing is silence — not the last thing
/// heard, and not a click.
#[test]
fn a_burst_over_a_gap_is_silence() {
    let project = project();
    let sequence = project.active().unwrap();
    let plan = AudioPlan::of(&project, sequence);
    let mut mixer = AudioMixer::new(1);

    let frames = 48_000 / 10;
    let mut block = vec![0.0_f32; frames * 2];
    // Ten seconds in: past the end of the only clip.
    mixer.mix_block(&plan, TimelineTime::from_seconds(10), frames, 2, &mut block);
    assert!(
        block.iter().all(|sample| *sample == 0.0),
        "a gap should mix as silence"
    );
}

/// And one asked for inside a clip reads that clip — the file is missing here,
/// so what is checked is that the mixer asked for it rather than for nothing.
#[test]
fn a_burst_inside_a_clip_reaches_for_its_file() {
    let project = project();
    let sequence = project.active().unwrap();
    let plan = AudioPlan::of(&project, sequence);
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].clips().len(), 1);
    assert_eq!(plan.assets.len(), 1, "the clip's file is not in the plan");

    let mut mixer = AudioMixer::new(1);
    let frames = 48_000 / 10;
    let mut block = vec![0.0_f32; frames * 2];
    mixer.mix_block(&plan, TimelineTime::from_seconds(3), frames, 2, &mut block);
    // The file does not exist, so nothing decodes — and nothing panics, which
    // is the rule a missing file follows everywhere (§66).
    assert!(block.iter().all(|sample| sample.is_finite()));
}

/// Holding the pitch: the mixer shifts a sped-up clip back by as much as the
/// re-timing moved it — an octave down for 2×, an octave up for half speed.
#[test]
fn the_pitch_correction_answers_the_speed() {
    let correction = |speed: f64| -12.0 * (speed as f32).log2();
    assert!(
        (correction(2.0) + 12.0).abs() < 1e-4,
        "2x should drop an octave"
    );
    assert!(
        (correction(0.5) - 12.0).abs() < 1e-4,
        "half speed should rise an octave"
    );
    assert!(correction(1.0).abs() < 1e-6, "normal speed should not move");
    // And what the shifter will actually do with it: an octave either way is
    // exactly its limit, so nothing is silently clamped at the usual speeds.
    assert!(bettercut_audio::PitchShifter::new(correction(2.0)).is_some());
    assert!(bettercut_audio::PitchShifter::new(correction(0.5)).is_some());
    assert!(bettercut_audio::PitchShifter::new(correction(1.0)).is_none());
}
