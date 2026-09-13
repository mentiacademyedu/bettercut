//! Voice clean-up (`bettercut_audio::voice`), on signals whose answer is known.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_audio::VoiceCleaner;

const RATE: f32 = 48_000.0;

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

fn db(ratio: f32) -> f32 {
    20.0 * ratio.max(1e-9).log10()
}

/// A fixed pseudo-random hiss in -1..1, the same every run.
fn hiss(frames: usize, level: f32) -> Vec<f32> {
    let mut state = 0x2545_F491_u32;
    (0..frames)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32 * 2.0 - 1.0) * level
        })
        .collect()
}

fn tone(frames: usize, hz: f32, level: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| (i as f32 / RATE * hz * std::f32::consts::TAU).sin() * level)
        .collect()
}

fn clean(amount: f32, planes: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let mut out = planes.to_vec();
    VoiceCleaner::new(amount).expect("on").process(&mut out);
    out
}

/// Nothing set is no processing at all.
#[test]
fn nothing_is_no_cleaner() {
    assert!(VoiceCleaner::new(0.0).is_none());
    assert!(VoiceCleaner::new(f32::NAN).is_none());
}

/// Room hiss on its own — the gaps between words — comes down by well over
/// ten decibels at full strength, once the floor has been learned.
#[test]
fn background_hiss_is_pulled_down() {
    let noise = hiss(48_000 * 3, 0.01);
    let out = clean(100.0, std::slice::from_ref(&noise));
    let settled = 48_000 * 2..48_000 * 3;
    let before = rms(&noise[settled.clone()]);
    let after = rms(&out[0][settled]);
    assert!(
        db(after / before) < -10.0,
        "hiss only came down {:.1} dB",
        db(after / before)
    );
}

/// A voice-level tone well above the noise passes at its own level: the
/// expander opens for it and the high-pass does not touch 440 Hz.
#[test]
fn speech_level_sound_keeps_its_level() {
    let frames = 48_000 * 3;
    let noise = hiss(frames, 0.005);
    // A second of hiss to learn the floor, then the "voice" over it.
    let mut input: Vec<f32> = noise.clone();
    let voice = tone(frames, 440.0, 0.3);
    for i in 48_000..frames {
        input[i] += voice[i];
    }
    let out = clean(100.0, &[input.clone()]);
    let span = 48_000 * 2..frames;
    let change = db(rms(&out[0][span.clone()]) / rms(&input[span]));
    assert!(
        change.abs() < 1.0,
        "the voice level changed by {change:.2} dB"
    );
}

/// Rumble below the voice is cut; a mid-range tone is left alone.
///
/// Each tone follows a second of quiet hiss, as speech follows room tone: a
/// steady sound with no quieter moment at all is indistinguishable from
/// background noise, and a voice clean-up is right to turn it down.
#[test]
fn rumble_goes_and_the_midrange_stays() {
    let frames = 48_000 * 3;
    let span = 48_000 * 2..frames;
    let after_quiet = |mut sound: Vec<f32>| {
        let room = hiss(frames, 0.003);
        for (i, sample) in sound.iter_mut().enumerate() {
            if i < 48_000 {
                *sample = room[i];
            } else {
                *sample += room[i];
            }
        }
        sound
    };
    let low = after_quiet(tone(frames, 30.0, 0.3));
    let mid = after_quiet(tone(frames, 1_000.0, 0.3));
    let low_out = clean(100.0, std::slice::from_ref(&low));
    let mid_out = clean(100.0, std::slice::from_ref(&mid));
    let low_change = db(rms(&low_out[0][span.clone()]) / rms(&low[span.clone()]));
    let mid_change = db(rms(&mid_out[0][span.clone()]) / rms(&mid[span]));
    assert!(low_change < -12.0, "30 Hz only fell {low_change:.1} dB");
    assert!(
        mid_change.abs() < 0.5,
        "1 kHz changed by {mid_change:.2} dB"
    );
}

/// The same sound cleaned in one block or in many small ones is the same
/// samples — which is what lets preview and export, with different block
/// sizes, agree.
#[test]
fn block_size_does_not_change_the_result() {
    let mut input = hiss(20_000, 0.02);
    for (i, sample) in tone(20_000, 220.0, 0.2).iter().enumerate().skip(8_000) {
        input[i] += sample;
    }
    let whole = clean(60.0, &[input.clone(), input.clone()]);

    let mut cleaner = VoiceCleaner::new(60.0).unwrap();
    let mut pieces: Vec<f32> = Vec::new();
    for chunk in input.chunks(517) {
        let mut planes = vec![chunk.to_vec(), chunk.to_vec()];
        cleaner.process(&mut planes);
        pieces.extend_from_slice(&planes[0]);
    }
    assert_eq!(whole[0], pieces);
}
