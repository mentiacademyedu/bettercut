//! Exporting sound only, as a WAV (`bettercut_export::wav`).
//!
//! No GPU is touched, so none of this skips.
//!
//! The claims: the file is a WAV every tool reads — the header says what the
//! data is and how long; it holds exactly the range asked for, sample for
//! sample; the samples are the timeline's mix, locked to §9's ticks so the
//! same instant gives the same sample whatever range it is exported in; and a
//! cancelled or impossible export leaves nothing behind.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;
use std::path::{Path, PathBuf};

use bettercut_export::{
    ExportError, ExportProgress, ExportSettings, VideoCodec, WavWriter, export,
};
use bettercut_foundation::{FrameRate, MediaTime, TimelineTime};
use bettercut_media::{CancellationToken, FfmpegProber, MediaProber, NeverCancelled};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioClip, Resolution, SourceRange, TimelineRange};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("bettercut-sound-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The fixture's sound on A1 from timeline zero.
fn project_with_sound() -> Project {
    let mut project = Project::new("Sound");
    let asset = FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    assert!(
        asset.audio_codec.is_some(),
        "setup: the fixture needs sound"
    );
    let duration = asset.duration;
    let media = project.add_media(asset);
    let sequence = project.active_mut().unwrap();
    sequence.audio_tracks[0]
        .insert(
            AudioClip::new(
                media,
                TimelineTime::ZERO,
                SourceRange::new(MediaTime::ZERO, duration).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    project
}

fn sound_settings(path: &Path, start_ms: i64, end_ms: i64) -> ExportSettings {
    ExportSettings {
        transparent: false,
        path: path.to_path_buf(),
        resolution: Resolution::new(640, 360),
        frame_rate: FrameRate::NTSC_29_97,
        codec: VideoCodec::H264,
        bitrate: None,
        rate_control: bettercut_export::RateControl::Variable,
        range: TimelineRange::new(
            TimelineTime::from_millis(start_ms),
            TimelineTime::from_millis(end_ms),
        )
        .unwrap(),
        threads: 1,
        sound_only: true,
        picture_only: false,
        gif: false,
        image_sequence: false,
    }
}

fn run(project: &Project, settings: &ExportSettings) -> Result<(), ExportError> {
    export(
        project,
        project.active().unwrap(),
        settings,
        &mut |_: ExportProgress| {},
        &NeverCancelled,
    )
    .map(|_| ())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

/// The 16-bit samples after the 44-byte header.
fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes[44..]
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

/// The header describes the data truthfully: PCM, stereo, 48 kHz, 16-bit, and
/// the sizes of what was actually written.
#[test]
fn the_header_describes_the_data() {
    let mut writer = WavWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.push(&[0.0, 0.0, 0.5, -0.5]).unwrap();
    writer.push(&[1.0, -1.0]).unwrap();
    let bytes = writer.finish().unwrap().into_inner();

    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(u32_at(&bytes, 4) as usize, bytes.len() - 8);
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(u16_at(&bytes, 20), 1, "not PCM");
    assert_eq!(u16_at(&bytes, 22), 2, "not stereo");
    assert_eq!(u32_at(&bytes, 24), 48_000);
    assert_eq!(u32_at(&bytes, 28), 48_000 * 4, "byte rate");
    assert_eq!(u16_at(&bytes, 32), 4, "block align");
    assert_eq!(u16_at(&bytes, 34), 16);
    assert_eq!(&bytes[36..40], b"data");
    assert_eq!(u32_at(&bytes, 40), 12, "six samples of two bytes");
    assert_eq!(bytes.len(), 44 + 12);
}

/// Full scale is full scale, silence is silence, and anything past full scale
/// is clamped rather than wrapped into a click.
#[test]
fn samples_convert_without_wrapping() {
    let mut writer = WavWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.push(&[0.0, 1.0, -1.0, 0.5, 2.0, -2.0]).unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert_eq!(
        samples(&bytes),
        vec![0, 32767, -32767, 16384, 32767, -32767]
    );
}

/// One second of sound is 48,000 stereo samples, exactly, and it is not
/// silence — the mix reached the file.
#[test]
fn a_range_exports_exactly_its_samples() {
    let out = Scratch::new("second.wav");
    let project = project_with_sound();

    run(&project, &sound_settings(&out.0, 0, 1_000)).unwrap();

    let bytes = std::fs::read(&out.0).unwrap();
    let samples = samples(&bytes);
    assert_eq!(samples.len(), 48_000 * 2);
    assert!(
        samples.iter().any(|s| s.unsigned_abs() > 100),
        "the file is silent; the mix did not reach it"
    );
}

/// Positions come from ticks, not from a running count: the half second from
/// 0.5 s on is the same samples whether it is exported alone or as the second
/// half of a longer range.
#[test]
fn the_same_instant_gives_the_same_samples_in_any_range() {
    let whole = Scratch::new("whole.wav");
    let half = Scratch::new("half.wav");
    let project = project_with_sound();

    run(&project, &sound_settings(&whole.0, 0, 1_000)).unwrap();
    run(&project, &sound_settings(&half.0, 500, 1_000)).unwrap();

    let whole = samples(&std::fs::read(&whole.0).unwrap());
    let half = samples(&std::fs::read(&half.0).unwrap());
    assert_eq!(half.len(), 24_000 * 2);
    let tail = &whole[whole.len() - half.len()..];
    let worst = tail
        .iter()
        .zip(&half)
        .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
        .max()
        .unwrap();
    assert!(
        worst <= 2,
        "the second half differs by up to {worst} when exported on its own"
    );
}

/// Every sound track off: refused with a reason, and no empty file left behind
/// looking like a broken WAV.
#[test]
fn no_sound_is_refused_without_leaving_a_file() {
    let out = Scratch::new("silent.wav");
    let mut project = project_with_sound();
    for track in &mut project.active_mut().unwrap().audio_tracks {
        track.enabled = false;
    }

    let err = run(&project, &sound_settings(&out.0, 0, 1_000)).unwrap_err();

    assert!(matches!(err, ExportError::NoSound), "{err}");
    assert!(!out.0.exists());
}

/// A cancelled sound export removes its partial file, as a video export does.
#[test]
fn cancelling_removes_the_partial_file() {
    struct Cancelled;
    impl CancellationToken for Cancelled {
        fn is_cancelled(&self) -> bool {
            true
        }
    }
    let out = Scratch::new("cancelled.wav");
    let project = project_with_sound();

    let err = export(
        &project,
        project.active().unwrap(),
        &sound_settings(&out.0, 0, 1_000),
        &mut |_: ExportProgress| {},
        &Cancelled,
    )
    .unwrap_err();

    assert!(err.is_cancellation(), "{err}");
    assert!(!out.0.exists(), "the partial WAV was left behind");
}

/// A reversed clip's sound is its material played backwards: the samples of
/// a reversed export are the forward export's, end first. Allowed a sample of
/// slack either way, for where the read window's edges fall.
#[test]
fn a_reversed_clip_sounds_backwards() {
    let forward = Scratch::new("forward.wav");
    let backward = Scratch::new("backward.wav");
    let project = project_with_sound();
    let mut reversed = project.clone();
    {
        let sequence = reversed.active_mut().unwrap();
        let clip = sequence.audio_tracks[0].clips()[0].id;
        let clip = sequence.audio_tracks[0].get_mut(clip).unwrap();
        // One second of material, so the whole clip is exported either way.
        clip.source.end = MediaTime::from_seconds(1);
        clip.timeline.end = TimelineTime::from_seconds(1);
        clip.reversed = true;
    }
    let mut trimmed = project.clone();
    {
        let sequence = trimmed.active_mut().unwrap();
        let clip = sequence.audio_tracks[0].clips()[0].id;
        let clip = sequence.audio_tracks[0].get_mut(clip).unwrap();
        clip.source.end = MediaTime::from_seconds(1);
        clip.timeline.end = TimelineTime::from_seconds(1);
    }

    run(&trimmed, &sound_settings(&forward.0, 0, 1_000)).unwrap();
    run(&reversed, &sound_settings(&backward.0, 0, 1_000)).unwrap();

    let forward = samples(&std::fs::read(&forward.0).unwrap());
    let backward = samples(&std::fs::read(&backward.0).unwrap());
    assert_eq!(forward.len(), backward.len());

    // Stereo frames, end first. Compare away from the very edges, where a
    // window boundary can shift by a sample.
    let frames = forward.len() / 2;
    let frame = |s: &[i16], i: usize| [s[i * 2], s[i * 2 + 1]];
    let mut worst = i32::MAX;
    for shift in -2_i64..=2 {
        let mut error = 0;
        for i in 100..frames - 100 {
            let j = (frames as i64 - 1 - i as i64 + shift) as usize;
            let (a, b) = (frame(&forward, i), frame(&backward, j));
            error = error.max((i32::from(a[0]) - i32::from(b[0])).abs());
        }
        worst = worst.min(error);
    }
    assert!(
        forward.iter().any(|s| s.unsigned_abs() > 100),
        "setup: the forward sound is silent"
    );
    assert!(
        worst <= 64,
        "the reversed sound is not the forward sound backwards: off by {worst}"
    );
}

/// Voice clean-up is one continuous process whatever size of blocks the sound
/// is mixed in: the preview's device buffers and the export's tenth-of-a-second
/// blocks give the same samples (§46). And it changes the sound — a clean-up
/// that did nothing would pass the first half of this.
#[test]
fn a_cleaned_clip_sounds_the_same_in_any_block_size() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};

    let mut project = project_with_sound();
    let plain_project = project.clone();
    {
        let sequence = project.active_mut().unwrap();
        let clip = sequence.audio_tracks[0].clips()[0].id;
        sequence.audio_tracks[0].get_mut(clip).unwrap().denoise = 100.0;
    }

    let mix = |project: &Project, block: usize| {
        let sequence = project.active().unwrap();
        let plan = AudioPlan::of(project, sequence);
        let mut mixer = AudioMixer::new(1);
        let total = 24_000;
        let mut out = Vec::new();
        let mut done = 0;
        while done < total {
            let frames = block.min(total - done);
            let mut buffer = vec![0.0_f32; frames * 2];
            let at = TimelineTime::from_ticks(done as i64 * TICKS_PER_AUDIO_SAMPLE);
            mixer.mix_block(&plan, at, frames, 2, &mut buffer);
            out.extend_from_slice(&buffer);
            done += frames;
        }
        out
    };

    let big = mix(&project, 4_800);
    let small = mix(&project, 441);
    assert_eq!(big, small, "the clean-up depends on the block size");

    let plain = mix(&plain_project, 4_800);
    assert_ne!(big, plain, "the clean-up changed nothing");
}

/// The equaliser runs in the mixer the way the clean-up does: the same samples
/// in any block size, and a sound that differs from the flat one.
#[test]
fn an_equalised_clip_sounds_the_same_in_any_block_size() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};

    let mut project = project_with_sound();
    let plain_project = project.clone();
    {
        let sequence = project.active_mut().unwrap();
        let clip = sequence.audio_tracks[0].clips()[0].id;
        sequence.audio_tracks[0].get_mut(clip).unwrap().eq = bettercut_timeline::ClipEq {
            low_cut: 200.0,
            high_cut: 3_000.0,
            presence: 6.0,
            hum: 0.0,
        };
    }

    let mix = |project: &Project, block: usize| {
        let sequence = project.active().unwrap();
        let plan = AudioPlan::of(project, sequence);
        let mut mixer = AudioMixer::new(1);
        let total = 24_000;
        let mut out = Vec::new();
        let mut done = 0;
        while done < total {
            let frames = block.min(total - done);
            let mut buffer = vec![0.0_f32; frames * 2];
            let at = TimelineTime::from_ticks(done as i64 * TICKS_PER_AUDIO_SAMPLE);
            mixer.mix_block(&plan, at, frames, 2, &mut buffer);
            out.extend_from_slice(&buffer);
            done += frames;
        }
        out
    };

    let big = mix(&project, 4_800);
    let small = mix(&project, 441);
    assert_eq!(big, small, "the equaliser depends on the block size");
    assert_ne!(
        big,
        mix(&plain_project, 4_800),
        "the equaliser changed nothing"
    );

    // A jump back to the start begins the filters afresh: the same mixer
    // playing the opening again gives the opening's samples, not ones coloured
    // by where it had got to.
    let sequence = project.active().unwrap();
    let plan = AudioPlan::of(&project, sequence);
    let mut mixer = AudioMixer::new(1);
    let mut first = vec![0.0_f32; 4_800 * 2];
    mixer.mix_block(&plan, TimelineTime::ZERO, 4_800, 2, &mut first);
    let mut later = vec![0.0_f32; 4_800 * 2];
    mixer.mix_block(
        &plan,
        TimelineTime::from_ticks(4_800 * TICKS_PER_AUDIO_SAMPLE),
        4_800,
        2,
        &mut later,
    );
    let mut again = vec![0.0_f32; 4_800 * 2];
    mixer.mix_block(&plan, TimelineTime::ZERO, 4_800, 2, &mut again);
    assert_eq!(first, again, "the filters carried on across a jump");
}

/// "Left only" puts the left channel in both ears: the mixed right side is
/// the left side, and "Right only" the other way round.
#[test]
fn a_one_sided_recording_is_heard_in_both_ears() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};
    use bettercut_timeline::ChannelMode;

    let mix = |mode: ChannelMode| {
        let mut project = project_with_sound();
        {
            let sequence = project.active_mut().unwrap();
            let clip = sequence.audio_tracks[0].clips()[0].id;
            sequence.audio_tracks[0].get_mut(clip).unwrap().channels = mode;
        }
        let sequence = project.active().unwrap();
        let plan = AudioPlan::of(&project, sequence);
        let mut mixer = AudioMixer::new(1);
        let mut buffer = vec![0.0_f32; 4_800 * 2];
        mixer.mix_block(
            &plan,
            TimelineTime::from_ticks(2_400 * TICKS_PER_AUDIO_SAMPLE),
            4_800,
            2,
            &mut buffer,
        );
        buffer
    };
    let stereo = mix(ChannelMode::Stereo);
    let side = |buffer: &[f32], channel: usize| -> Vec<f32> {
        buffer.chunks_exact(2).map(|f| f[channel]).collect()
    };
    assert!(
        side(&stereo, 0).iter().any(|s| s.abs() > 1e-3),
        "setup: silent"
    );

    let left = mix(ChannelMode::LeftToBoth);
    assert_eq!(side(&left, 0), side(&left, 1), "the two ears differ");
    assert_eq!(side(&left, 0), side(&stereo, 0), "the left side changed");

    let right = mix(ChannelMode::RightToBoth);
    assert_eq!(
        side(&right, 0),
        side(&stereo, 1),
        "the right side was not copied"
    );

    let mono = mix(ChannelMode::Mono);
    assert_eq!(side(&mono, 0), side(&mono, 1));
}

/// A muted clip is silent where it stands, and the same clip unmuted is heard.
#[test]
fn a_muted_clip_is_not_heard() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};

    let loudest = |muted: bool| {
        let mut project = project_with_sound();
        {
            let sequence = project.active_mut().unwrap();
            let clip = sequence.audio_tracks[0].clips()[0].id;
            sequence.audio_tracks[0].get_mut(clip).unwrap().muted = muted;
        }
        let sequence = project.active().unwrap();
        let plan = AudioPlan::of(&project, sequence);
        let mut mixer = AudioMixer::new(1);
        let mut buffer = vec![0.0_f32; 4_800 * 2];
        mixer.mix_block(
            &plan,
            TimelineTime::from_ticks(2_400 * TICKS_PER_AUDIO_SAMPLE),
            4_800,
            2,
            &mut buffer,
        );
        buffer.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    };
    assert!(loudest(false) > 1e-3, "setup: the clip is silent anyway");
    assert_eq!(loudest(true), 0.0, "a muted clip was heard");
}

/// The leveller runs in the mixer like the other sound stages: the same
/// samples in any block size, and a sound that differs from the recording.
#[test]
fn a_levelled_clip_sounds_the_same_in_any_block_size() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};

    let mut project = project_with_sound();
    let plain_project = project.clone();
    {
        let sequence = project.active_mut().unwrap();
        let clip = sequence.audio_tracks[0].clips()[0].id;
        sequence.audio_tracks[0].get_mut(clip).unwrap().leveller = 80.0;
    }
    let mix = |project: &Project, block: usize| {
        let sequence = project.active().unwrap();
        let plan = AudioPlan::of(project, sequence);
        let mut mixer = AudioMixer::new(1);
        let total = 24_000;
        let mut out = Vec::new();
        let mut done = 0;
        while done < total {
            let frames = block.min(total - done);
            let mut buffer = vec![0.0_f32; frames * 2];
            let at = TimelineTime::from_ticks(done as i64 * TICKS_PER_AUDIO_SAMPLE);
            mixer.mix_block(&plan, at, frames, 2, &mut buffer);
            out.extend_from_slice(&buffer);
            done += frames;
        }
        out
    };
    let big = mix(&project, 4_800);
    assert_eq!(
        big,
        mix(&project, 441),
        "the leveller depends on the block size"
    );
    assert_ne!(
        big,
        mix(&plain_project, 4_800),
        "the leveller changed nothing"
    );
}

/// The de-esser runs in the mixer like the other sound stages: the same
/// samples in any block size, and — on a recording with no sibilance in it
/// — the recording, untouched. The dip itself is proved on a 7 kHz tone in
/// `bettercut_audio::deesser`; what matters here is that a voice with nothing
/// to de-ess is not quietly dulled by leaving the control up.
#[test]
fn a_de_essed_clip_sounds_the_same_in_any_block_size() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};

    let mut project = project_with_sound();
    let plain_project = project.clone();
    {
        let sequence = project.active_mut().unwrap();
        let clip = sequence.audio_tracks[0].clips()[0].id;
        sequence.audio_tracks[0].get_mut(clip).unwrap().de_ess = 100.0;
    }
    let mix = |project: &Project, block: usize| {
        let sequence = project.active().unwrap();
        let plan = AudioPlan::of(project, sequence);
        let mut mixer = AudioMixer::new(1);
        let total = 24_000;
        let mut out = Vec::new();
        let mut done = 0;
        while done < total {
            let frames = block.min(total - done);
            let mut buffer = vec![0.0_f32; frames * 2];
            let at = TimelineTime::from_ticks(done as i64 * TICKS_PER_AUDIO_SAMPLE);
            mixer.mix_block(&plan, at, frames, 2, &mut buffer);
            out.extend_from_slice(&buffer);
            done += frames;
        }
        out
    };
    let big = mix(&project, 4_800);
    assert_eq!(
        big,
        mix(&project, 441),
        "the de-esser depends on the block size"
    );
    assert_eq!(
        big,
        mix(&plain_project, 4_800),
        "a fixture with no sibilance in it came out changed"
    );
}

/// The voice changer runs in the mixer like the other sound stages: the same
/// samples in any block size, and a sound that differs from the voice as
/// recorded.
#[test]
fn a_voice_changed_clip_sounds_the_same_in_any_block_size() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};

    let mut project = project_with_sound();
    let plain_project = project.clone();
    {
        let sequence = project.active_mut().unwrap();
        let clip = sequence.audio_tracks[0].clips()[0].id;
        sequence.audio_tracks[0].get_mut(clip).unwrap().pitch = 7.0;
    }
    let mix = |project: &Project, block: usize| {
        let sequence = project.active().unwrap();
        let plan = AudioPlan::of(project, sequence);
        let mut mixer = AudioMixer::new(1);
        let total = 24_000;
        let mut out = Vec::new();
        let mut done = 0;
        while done < total {
            let frames = block.min(total - done);
            let mut buffer = vec![0.0_f32; frames * 2];
            let at = TimelineTime::from_ticks(done as i64 * TICKS_PER_AUDIO_SAMPLE);
            mixer.mix_block(&plan, at, frames, 2, &mut buffer);
            out.extend_from_slice(&buffer);
            done += frames;
        }
        out
    };
    let big = mix(&project, 4_800);
    assert_eq!(
        big,
        mix(&project, 441),
        "the voice changer depends on the block size"
    );
    assert_ne!(
        big,
        mix(&plain_project, 4_800),
        "the voice changer changed nothing"
    );
}

/// Echo and reverb run in the mixer the way the equaliser does: the same
/// samples in any block size, a sound that differs from the dry one, and
/// delay lines that start empty after a jump rather than ringing on.
#[test]
fn a_reverberant_clip_sounds_the_same_in_any_block_size() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};

    let mut project = project_with_sound();
    let dry_project = project.clone();
    {
        let sequence = project.active_mut().unwrap();
        let clip = sequence.audio_tracks[0].clips()[0].id;
        sequence.audio_tracks[0].get_mut(clip).unwrap().space = bettercut_timeline::ClipSpace {
            kind: bettercut_timeline::SpaceKind::Hall,
            mix: 0.5,
        };
    }

    let mix = |project: &Project, block: usize| {
        let sequence = project.active().unwrap();
        let plan = AudioPlan::of(project, sequence);
        let mut mixer = AudioMixer::new(1);
        let total = 24_000;
        let mut out = Vec::new();
        let mut done = 0;
        while done < total {
            let frames = block.min(total - done);
            let mut buffer = vec![0.0_f32; frames * 2];
            let at = TimelineTime::from_ticks(done as i64 * TICKS_PER_AUDIO_SAMPLE);
            mixer.mix_block(&plan, at, frames, 2, &mut buffer);
            out.extend_from_slice(&buffer);
            done += frames;
        }
        out
    };

    let big = mix(&project, 4_800);
    let small = mix(&project, 441);
    assert_eq!(big, small, "the reverb depends on the block size");
    assert_ne!(big, mix(&dry_project, 4_800), "the reverb changed nothing");

    let sequence = project.active().unwrap();
    let plan = AudioPlan::of(&project, sequence);
    let mut mixer = AudioMixer::new(1);
    let mut first = vec![0.0_f32; 4_800 * 2];
    mixer.mix_block(&plan, TimelineTime::ZERO, 4_800, 2, &mut first);
    let mut later = vec![0.0_f32; 4_800 * 2];
    mixer.mix_block(
        &plan,
        TimelineTime::from_ticks(4_800 * TICKS_PER_AUDIO_SAMPLE),
        4_800,
        2,
        &mut later,
    );
    let mut again = vec![0.0_f32; 4_800 * 2];
    mixer.mix_block(&plan, TimelineTime::ZERO, 4_800, 2, &mut again);
    assert_eq!(first, again, "the reverb rang on across a jump");
}

/// A clip that ends partway through a mixed block stops there: the rest of
/// the block is silence, not more of the file past the clip's end.
#[test]
fn a_clip_stops_at_its_end_inside_a_block() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};

    let mut project = Project::new("End");
    let asset = FfmpegProber.probe(&fixture("ntsc-2997.mp4")).unwrap();
    let media = project.add_media(asset);
    let sequence = project.active_mut().unwrap();
    // Half a second of sound from 0.3 s into the file, placed at zero.
    sequence.audio_tracks[0]
        .insert(
            AudioClip::new(
                media,
                TimelineTime::ZERO,
                SourceRange::new(MediaTime::from_millis(300), MediaTime::from_millis(800)).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let sequence = project.active().unwrap();
    let plan = AudioPlan::of(&project, sequence);
    let mut mixer = AudioMixer::new(1);

    // One block from 0.4 s to 0.6 s: the clip ends at 0.5 s, in its middle.
    let frames = 9_600;
    let mut out = vec![0.0_f32; frames * 2];
    mixer.mix_block(
        &plan,
        TimelineTime::from_ticks(19_200 * TICKS_PER_AUDIO_SAMPLE),
        frames,
        2,
        &mut out,
    );
    let before: f32 = out[..4_800 * 2].iter().map(|s| s.abs()).sum();
    let after: f32 = out[4_800 * 2..].iter().map(|s| s.abs()).sum();
    assert!(before > 1.0, "setup: the clip is silent");
    assert_eq!(after, 0.0, "the clip went on sounding past its end");
}

/// A crossfade changes the sound only across the cut — half its length either
/// side — and is the same in any block size; outside that stretch the mix is
/// exactly the plain cut's.
#[test]
fn a_crossfade_blends_only_across_the_cut() {
    use bettercut_foundation::TICKS_PER_AUDIO_SAMPLE;
    use bettercut_playback::{AudioMixer, AudioPlan};

    let asset = FfmpegProber.probe(&fixture("ntsc-2997.mp4")).unwrap();
    let file = asset.duration.ticks();
    let build = |crossfade: TimelineTime| {
        let mut project = Project::new("Crossfade");
        let media = project.add_media(asset.clone());
        let sequence = project.active_mut().unwrap();
        // Two stretches of the file back to back, each with material either
        // side of it to blend with.
        let piece = |from: i64, to: i64| {
            SourceRange::new(
                MediaTime::from_ticks(file * from / 20),
                MediaTime::from_ticks(file * to / 20),
            )
            .unwrap()
        };
        let mut first = AudioClip::new(media, TimelineTime::ZERO, piece(4, 9)).unwrap();
        first.crossfade_out = crossfade;
        let cut = first.timeline.end;
        let second = AudioClip::new(media, cut, piece(11, 16)).unwrap();
        sequence.audio_tracks[0].insert(first).unwrap();
        sequence.audio_tracks[0].insert(second).unwrap();
        (project, cut)
    };
    let mix = |project: &Project, block: usize, total: usize| {
        let sequence = project.active().unwrap();
        let plan = AudioPlan::of(project, sequence);
        let mut mixer = AudioMixer::new(1);
        let mut out = Vec::new();
        let mut done = 0;
        while done < total {
            let frames = block.min(total - done);
            let mut buffer = vec![0.0_f32; frames * 2];
            let at = TimelineTime::from_ticks(done as i64 * TICKS_PER_AUDIO_SAMPLE);
            mixer.mix_block(&plan, at, frames, 2, &mut buffer);
            out.extend_from_slice(&buffer);
            done += frames;
        }
        out
    };

    let length = TimelineTime::from_millis(200);
    let (blended, cut) = build(length);
    let (plain, _) = build(TimelineTime::ZERO);
    let total = (cut.ticks() * 2 / TICKS_PER_AUDIO_SAMPLE) as usize;
    let a = mix(&blended, 4_800, total);
    let b = mix(&plain, 4_800, total);
    assert_eq!(
        a,
        mix(&blended, 441, total),
        "the crossfade depends on the block size"
    );

    let cut_frame = (cut.ticks() / TICKS_PER_AUDIO_SAMPLE) as usize;
    let half = (length.ticks() / 2 / TICKS_PER_AUDIO_SAMPLE) as usize;
    let (from, to) = ((cut_frame - half) * 2, (cut_frame + half) * 2);
    assert_eq!(
        a[..from - 2],
        b[..from - 2],
        "the mix changed before the crossfade"
    );
    assert_eq!(
        a[to + 2..],
        b[to + 2..],
        "the mix changed after the crossfade"
    );
    assert_ne!(
        a[from..to],
        b[from..to],
        "the crossfade changed nothing at the cut"
    );
}
