//! The EQ presets (`ClipEq::PRESETS`) are real settings: already in range,
//! each different, none flat.

use bettercut_timeline::ClipEq;

#[test]
fn every_preset_is_in_range_distinct_and_does_something() {
    for (name, _, eq) in ClipEq::PRESETS {
        assert_eq!(eq, eq.clamped(), "{name} is out of range");
        assert!(!eq.is_flat(), "{name} changes nothing");
    }
    for (index, (a, _, first)) in ClipEq::PRESETS.iter().enumerate() {
        for (b, _, second) in &ClipEq::PRESETS[index + 1..] {
            assert_ne!(first, second, "{a} and {b} are the same");
        }
    }
}
