//! Waveforms are drawn in decibels, so sound at an ordinary level fills most
//! of a clip, as in CapCut, rather than a sliver of it.

use bettercut_ui::timeline::{WAVEFORM_FLOOR_DB, waveform_height};

fn amplitude(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[test]
fn an_ordinary_level_fills_most_of_the_clip() {
    assert!(
        (waveform_height(1.0) - 1.0).abs() < 1e-6,
        "full scale is full"
    );
    // A voice or a song mixed for the web peaks around −18 dB.
    let ordinary = waveform_height(amplitude(-18.0));
    assert!(
        (0.55..0.7).contains(&ordinary),
        "−18 dB fills {ordinary} of the half-height"
    );
    // Even, in decibels: −24 dB is half way down a 48 dB range.
    assert!((waveform_height(amplitude(-24.0)) - 0.5).abs() < 1e-4);
}

#[test]
fn silence_and_the_floor_draw_nothing_and_louder_is_taller() {
    assert_eq!(waveform_height(0.0), 0.0);
    assert_eq!(waveform_height(amplitude(WAVEFORM_FLOOR_DB - 6.0)), 0.0);
    assert_eq!(waveform_height(f32::NAN), 0.0);
    assert_eq!(
        waveform_height(4.0),
        1.0,
        "over full scale is held to the clip"
    );
    let mut last = 0.0;
    for db in (-48..=0).step_by(6) {
        let height = waveform_height(amplitude(db as f32));
        assert!(height >= last, "{db} dB drew lower than quieter sound");
        last = height;
    }
}
