//! The clip equaliser (`bettercut_audio::eq`), measured on pure tones.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_audio::{Equalizer, PRESENCE_HZ};

const RATE: f32 = 48_000.0;

fn tone(hz: f32) -> Vec<f32> {
    (0..48_000)
        .map(|i| (i as f32 / RATE * hz * std::f32::consts::TAU).sin() * 0.5)
        .collect()
}

/// The change in level, in decibels, over the last half second (after the
/// filters have settled).
fn change_db(eq: (f32, f32, f32), hz: f32) -> f32 {
    let input = tone(hz);
    let mut planes = vec![input.clone()];
    Equalizer::new(eq.0, eq.1, eq.2)
        .expect("not flat")
        .process(&mut planes);
    let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
    20.0 * (rms(&planes[0][24_000..]) / rms(&input[24_000..])).log10()
}

#[test]
fn flat_is_nothing_to_run() {
    assert!(Equalizer::new(0.0, 0.0, 0.0).is_none());
    assert!(Equalizer::new(f32::NAN, -5.0, 0.01).is_none());
}

#[test]
fn a_low_cut_takes_out_rumble_and_leaves_the_voice() {
    assert!(
        change_db((150.0, 0.0, 0.0), 30.0) < -20.0,
        "30 Hz got through"
    );
    assert!(
        change_db((150.0, 0.0, 0.0), 1_000.0).abs() < 0.5,
        "1 kHz was touched"
    );
}

#[test]
fn a_high_cut_softens_the_top_and_leaves_the_middle() {
    assert!(
        change_db((0.0, 3_000.0, 0.0), 15_000.0) < -20.0,
        "15 kHz got through"
    );
    assert!(
        change_db((0.0, 3_000.0, 0.0), 300.0).abs() < 0.5,
        "300 Hz was touched"
    );
}

#[test]
fn presence_lifts_and_dips_its_band_only() {
    let lift = change_db((0.0, 0.0, 6.0), PRESENCE_HZ);
    assert!(
        (lift - 6.0).abs() < 0.5,
        "a 6 dB lift came out at {lift:.2} dB"
    );
    let dip = change_db((0.0, 0.0, -6.0), PRESENCE_HZ);
    assert!(
        (dip + 6.0).abs() < 0.5,
        "a 6 dB dip came out at {dip:.2} dB"
    );
    assert!(
        change_db((0.0, 0.0, 6.0), 100.0).abs() < 0.5,
        "the lows moved too"
    );
}

/// One block or many small ones: the same samples.
#[test]
fn block_size_does_not_change_the_result() {
    let input = tone(440.0);
    let mut whole = vec![input.clone(), input.clone()];
    Equalizer::new(120.0, 8_000.0, 4.0)
        .unwrap()
        .process(&mut whole);

    let mut eq = Equalizer::new(120.0, 8_000.0, 4.0).unwrap();
    let mut pieces = Vec::new();
    for chunk in input.chunks(333) {
        let mut planes = vec![chunk.to_vec(), chunk.to_vec()];
        eq.process(&mut planes);
        pieces.extend_from_slice(&planes[1]);
    }
    assert_eq!(whole[1], pieces);
}

/// The change in level over the last half second, with a hum filter running.
fn hum_change_db(hum_hz: f32, hz: f32) -> f32 {
    let input = tone(hz);
    let mut planes = vec![input.clone()];
    Equalizer::with_hum(0.0, 0.0, 0.0, hum_hz)
        .expect("not flat")
        .process(&mut planes);
    let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
    20.0 * (rms(&planes[0][24_000..]) / rms(&input[24_000..])).log10()
}

/// The hum itself, and the harmonics that make it a buzz rather than a rumble.
#[test]
fn the_hum_and_its_harmonics_go() {
    for (mains, harmonics) in [
        (50.0_f32, [50.0, 100.0, 150.0]),
        (60.0, [60.0, 120.0, 180.0]),
    ] {
        for hz in harmonics {
            let change = hum_change_db(mains, hz);
            assert!(
                change < -20.0,
                "{mains} Hz filter left {hz} Hz at {change:.1} dB"
            );
        }
    }
}

/// And the voice over it does not: a notch is narrow, or it would take the
/// bottom out of everything it was meant to save.
#[test]
fn the_voice_over_the_hum_is_left_alone() {
    for hz in [220.0_f32, 440.0, 1_000.0, 3_000.0] {
        let change = hum_change_db(50.0, hz);
        assert!(
            change.abs() < 1.0,
            "the 50 Hz filter moved {hz} Hz by {change:.1} dB"
        );
    }
    // Even close by: 80 Hz is the bottom of a male voice, and it survives.
    let change = hum_change_db(50.0, 80.0);
    assert!(change.abs() < 3.0, "80 Hz moved by {change:.1} dB");
}

/// A 60 Hz filter is not a 50 Hz one: choosing the wrong mains rate leaves the
/// hum where it was, which is why the control names both.
#[test]
fn the_wrong_mains_rate_leaves_the_hum() {
    let change = hum_change_db(60.0, 50.0);
    assert!(
        change > -3.0,
        "a 60 Hz filter took out 50 Hz hum ({change:.1} dB), so the choice does not matter"
    );
}

#[test]
fn no_hum_rate_is_nothing_to_run() {
    assert!(Equalizer::with_hum(0.0, 0.0, 0.0, 0.0).is_none());
    assert!(Equalizer::with_hum(0.0, 0.0, 0.0, f32::NAN).is_none());
    assert!(Equalizer::with_hum(0.0, 0.0, 0.0, 50.0).is_some());
}

/// The hum filter runs beside the rest of the equaliser rather than instead of
/// it: a clip can have a low cut *and* a hum notch.
#[test]
fn the_hum_filter_joins_the_others() {
    let input = tone(50.0);
    let mut planes = vec![input.clone()];
    Equalizer::with_hum(0.0, 8_000.0, 4.0, 50.0)
        .expect("not flat")
        .process(&mut planes);
    let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
    let change = 20.0 * (rms(&planes[0][24_000..]) / rms(&input[24_000..])).log10();
    assert!(change < -20.0, "{change:.1} dB");
}
