//! Heartbeat (`heartbeat_zoom`): two thumps a beat, the second softer, then
//! a rest — without markers.

use bettercut_foundation::TimelineTime;
use bettercut_playback::engine::{HEARTBEAT_HZ, MAX_HEARTBEAT, heartbeat_zoom};

fn at_phase(beat: u32, phase: f64) -> TimelineTime {
    let s = (f64::from(beat) + phase) / HEARTBEAT_HZ;
    TimelineTime::from_ticks((s * bettercut_foundation::TICKS_PER_SECOND as f64) as i64)
}

#[test]
fn off_does_nothing() {
    assert_eq!(heartbeat_zoom(0.0, at_phase(0, 0.0)), 1.0);
    assert_eq!(heartbeat_zoom(f32::NAN, at_phase(0, 0.0)), 1.0);
}

/// Lub, then a softer dub, then nothing until the next beat; never past the
/// most, and the same every beat.
#[test]
fn it_thumps_twice_then_rests() {
    for beat in 0..5 {
        let lub = heartbeat_zoom(100.0, at_phase(beat, 0.001));
        let dub = heartbeat_zoom(100.0, at_phase(beat, 0.221));
        let rest = heartbeat_zoom(100.0, at_phase(beat, 0.7));
        assert!((lub - (1.0 + MAX_HEARTBEAT)).abs() < 2e-3, "{lub}");
        assert!(1.0 < dub && dub < lub, "{dub}");
        assert_eq!(rest, 1.0);
    }
    for i in 0..200 {
        let zoom = heartbeat_zoom(100.0, at_phase(0, f64::from(i) / 200.0));
        assert!((1.0..=1.0 + MAX_HEARTBEAT + 1e-6).contains(&zoom));
    }
    assert!(heartbeat_zoom(30.0, at_phase(1, 0.001)) < heartbeat_zoom(100.0, at_phase(1, 0.001)));
}
