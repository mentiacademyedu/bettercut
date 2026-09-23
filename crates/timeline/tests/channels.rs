//! The channel modes as applied to a decoded pair
//! (`bettercut_timeline::ChannelMode`): the swap in particular, which turns
//! a backwards-wired recording round without touching anything else.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_timeline::ChannelMode;

fn pair() -> Vec<Vec<f32>> {
    vec![vec![0.8, 0.2, -0.4], vec![0.1, 0.6, 0.3]]
}

#[test]
fn swapped_is_left_and_right_the_other_way_round() {
    let mut planes = pair();
    ChannelMode::Swapped.apply(&mut planes);
    assert_eq!(planes[0], pair()[1]);
    assert_eq!(planes[1], pair()[0]);

    // Twice is as recorded.
    ChannelMode::Swapped.apply(&mut planes);
    assert_eq!(planes, pair());
}

/// A mono source has no sides to swap, and is left alone.
#[test]
fn swapping_a_single_channel_changes_nothing() {
    let mut planes = vec![vec![0.5, 0.25]];
    ChannelMode::Swapped.apply(&mut planes);
    assert_eq!(planes, vec![vec![0.5, 0.25]]);
}

/// Every mode is offered, each with its own words, and only one of them is
/// how the sound was recorded.
#[test]
fn every_mode_is_offered_and_named() {
    assert!(ChannelMode::ALL.contains(&ChannelMode::Swapped));
    let labels: std::collections::HashSet<&str> =
        ChannelMode::ALL.iter().map(|mode| mode.label()).collect();
    assert_eq!(
        labels.len(),
        ChannelMode::ALL.len(),
        "two modes share a name"
    );
    assert_eq!(ChannelMode::default(), ChannelMode::Stereo);
    for mode in ChannelMode::ALL {
        assert!(!mode.description().is_empty(), "{mode:?} says nothing");
    }
}
