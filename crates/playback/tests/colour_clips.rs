//! Colour clips decode without a file: the frame is made from the colours,
//! the same way for the preview and the export (§46).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{FrameStorage, Generated, MediaAsset, NeverCancelled, SeekMode};
use bettercut_playback::FrameSource;
use bettercut_project_format::Project;
use bettercut_timeline::{SourceRange, VideoClip};

#[test]
fn a_generated_frame_needs_no_file() {
    let asset = MediaAsset::generated(
        Generated::Colour {
            top: [255, 255, 255],
            bottom: [0, 0, 0],
        },
        1920,
        1080,
    );
    assert!(asset.exists());
    let mut frames = FrameSource::new(1);
    let frame = frames
        .decode(
            &asset,
            MediaTime::from_seconds(3),
            SeekMode::Playback,
            &NeverCancelled,
        )
        .unwrap();
    assert_eq!(frame.width * 1080, frame.height * 1920, "wrong shape");
    let FrameStorage::System { data, .. } = &frame.storage else {
        panic!("system frame");
    };
    assert_eq!(&data[..4], &[255, 255, 255, 255]);
    assert_eq!(&data[data.len() - 4..], &[0, 0, 0, 255]);
    assert_eq!(frames.open_decoders(), 0, "opened a decoder");
}

#[test]
fn the_plan_draws_it_like_any_picture() {
    let mut project = Project::new("Colour");
    let media = project.add_media(MediaAsset::generated(
        Generated::solid([9, 9, 9]),
        1920,
        1080,
    ));
    let clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(5)).unwrap(),
    )
    .unwrap();
    project.active_mut().unwrap().video_tracks[0]
        .insert(clip)
        .unwrap();
    let sequence = project.active().unwrap();
    let requests =
        bettercut_playback::layer_requests(&project, sequence, TimelineTime::from_seconds(2));
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].source.media(), Some(media));
}
