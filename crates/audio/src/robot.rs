//! The robot voice: a ring modulator.
//!
//! The voice is multiplied by a low sine, which splits every harmonic into a
//! pair either side of where it was — no longer in a harmonic series, so the
//! voice keeps its words and loses its warmth. `amount` blends it with the
//! voice as recorded.
//!
//! No state: the sine's phase comes from the sample's place on the timeline,
//! so playback that jumps, and an export that renders in other-sized blocks,
//! both hear exactly the same wobble at the same moment.

use bettercut_foundation::AUDIO_SAMPLE_RATE;

/// The carrier, in hertz. Low enough to sound like a machine rather than a
/// bell; the classic science-fiction robot sits around here.
pub const ROBOT_HZ: f64 = 50.0;

/// Ring-modulate `planes`, whose first frame is sample `first` of the
/// timeline, by `amount` (0–100; zero leaves them untouched).
pub fn robot(planes: &mut [Vec<f32>], amount: f32, first: i64) {
    if !(amount.is_finite() && amount > 0.0) {
        return;
    }
    let wet = (amount / 100.0).min(1.0);
    let step = std::f64::consts::TAU * ROBOT_HZ / AUDIO_SAMPLE_RATE as f64;
    // Whole carrier cycles dropped from every index, not just the block's
    // first: the same moment then always takes the sine of the same number,
    // bit for bit, whatever block it falls in.
    let period = AUDIO_SAMPLE_RATE / ROBOT_HZ as i64;
    for plane in planes.iter_mut() {
        for (i, sample) in plane.iter_mut().enumerate() {
            let phase = (first + i as i64).rem_euclid(period);
            let carrier = (phase as f64 * step).sin() as f32;
            // Times 1.4 so the wet signal is about as loud as the dry: the
            // sine's average power is half.
            *sample *= 1.0 - wet + wet * carrier * 1.4;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_leaves_the_sound_alone() {
        let mut planes = vec![vec![0.5_f32; 64]];
        robot(&mut planes, 0.0, 0);
        assert!(planes[0].iter().all(|&s| s == 0.5));
    }

    #[test]
    fn full_follows_the_carrier_and_is_the_same_in_any_block_size() {
        let mut whole = vec![vec![0.5_f32; 2000]];
        robot(&mut whole, 100.0, 12_345);
        let mut split_a = vec![vec![0.5_f32; 700]];
        let mut split_b = vec![vec![0.5_f32; 1300]];
        robot(&mut split_a, 100.0, 12_345);
        robot(&mut split_b, 100.0, 12_345 + 700);
        let joined: Vec<f32> = split_a[0].iter().chain(&split_b[0]).copied().collect();
        for (a, b) in whole[0].iter().zip(&joined) {
            assert!((a - b).abs() < 1e-5);
        }
        // The steady input now swings both ways with the carrier.
        assert!(whole[0].iter().any(|&s| s > 0.3));
        assert!(whole[0].iter().any(|&s| s < -0.3));
    }
}
