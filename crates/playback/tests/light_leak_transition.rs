//! The light leak transition (`leak_burst`): a warm glow crossing the cut,
//! strongest at the cut and gone at either end.

use bettercut_playback::engine::leak_burst;

#[test]
fn nothing_at_either_end_and_most_at_the_cut() {
    assert_eq!(leak_burst(0.0).2, 0.0);
    assert_eq!(leak_burst(1.0).2, 0.0);
    let at_cut = leak_burst(0.5);
    assert!(at_cut.2 > 0.9, "{at_cut:?}");
    for i in 1..10 {
        let p = i as f32 / 10.0;
        assert!(leak_burst(p).2 <= at_cut.2 + 1e-6);
    }
}

/// It crosses from left to right, and at the cut it covers more than the
/// frame so the middle of the cut is all light.
#[test]
fn it_crosses_left_to_right_and_swallows_the_frame_at_the_cut() {
    let xs: Vec<f32> = (0..=10).map(|i| leak_burst(i as f32 / 10.0).0[0]).collect();
    assert!(xs.windows(2).all(|w| w[1] > w[0]), "{xs:?}");
    let (_, size, _) = leak_burst(0.5);
    assert!(size[0] > 1.0, "the glow at the cut is smaller than the frame: {size:?}");
}
