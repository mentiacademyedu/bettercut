//! A caption lighting each word draws a different picture as each word comes
//! (`TextFrames::frame_at`), which the preview and the export both call.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::TimelineTime;
use bettercut_media::FrameStorage;
use bettercut_playback::TextFrames;
use bettercut_text::Rgba;
use bettercut_timeline::TextClip;

fn pixels(frame: &bettercut_media::VideoFrame) -> Vec<u8> {
    match &frame.storage {
        FrameStorage::System { data, .. } => data.clone(),
        _ => panic!("a title is drawn in system memory"),
    }
}

#[test]
fn each_word_is_its_own_picture_and_plain_text_is_unchanged() {
    let mut frames = TextFrames::new();
    let mut clip =
        TextClip::with_duration("one two", TimelineTime::ZERO, TimelineTime::from_seconds(4))
            .unwrap();
    let at = |ms: i64| TimelineTime::from_millis(ms);
    let plain = pixels(&frames.frame_at(&clip, None, at(500)).unwrap());

    clip.highlight = Some(Rgba::opaque(255, 214, 10));
    // "one" and "two" weigh the same: the first half and the second.
    let first = pixels(&frames.frame_at(&clip, None, at(500)).unwrap());
    let again = pixels(&frames.frame_at(&clip, None, at(1_900)).unwrap());
    let second = pixels(&frames.frame_at(&clip, None, at(2_100)).unwrap());
    assert_ne!(first, plain, "the highlight drew nothing");
    assert_eq!(first, again, "the same word lit twice drew differently");
    assert_ne!(first, second, "the highlight did not move on");
    assert_eq!(
        first.len(),
        second.len(),
        "the layout changed between words"
    );

    // At the end nothing is lit: the ordinary picture.
    let after = pixels(&frames.frame_at(&clip, None, at(4_000)).unwrap());
    assert_eq!(after, plain);
}
