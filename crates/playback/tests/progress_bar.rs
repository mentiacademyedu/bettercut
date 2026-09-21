//! The progress bar (`MasterLook::progress_bar`): a solid strip over everything,
//! as wide as the video is played.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{FrameRate, MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::{
    LayerSource, graded_beneath, layer_requests, layer_transform, solid_frame,
};
use bettercut_project_format::Project;
use bettercut_timeline::{ProgressBar, SourceRange, VideoClip, fit_scale};

/// Ten seconds of footage with a progress bar along `top` or the bottom.
fn project(bar: ProgressBar) -> Project {
    let mut project = Project::new("Progress");
    let media = project.add_media(
        MediaAsset::new(
            MediaKind::Video,
            "C:/media/talk.mp4",
            MediaTime::from_seconds(10),
        )
        .with_video(1920, 1080, FrameRate::new(30, 1).unwrap()),
    );
    let clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    let sequence = project.active_mut().unwrap();
    sequence.video_tracks[0].insert(clip).unwrap();
    sequence.master.progress_bar = bar;
    project
}

const BAR: ProgressBar = ProgressBar {
    height: 0.02,
    colour: [255, 0, 0],
    top: false,
};

#[test]
fn the_bar_is_drawn_last_as_wide_as_the_video_is_played() {
    let project = project(BAR);
    let sequence = project.active().unwrap();
    let requests = layer_requests(&project, sequence, TimelineTime::from_millis(2500));
    assert_eq!(requests.len(), 2);
    let bar = requests.last().unwrap();
    assert_eq!(bar.source, LayerSource::Solid { rgb: [255, 0, 0] });

    // Placed as the compositor will draw it: a quarter of the width, a fiftieth
    // of the height, from the bottom-left corner.
    let frame = solid_frame([255, 0, 0]);
    let transform = layer_transform(bar, &frame, sequence.resolution);
    let (fit_x, fit_y) = fit_scale(1.0, 1920.0 / 1080.0);
    assert!((transform.scale.x * fit_x - 0.25).abs() < 1e-4);
    assert!((transform.scale.y * fit_y - 0.02).abs() < 1e-4);
    assert_eq!((transform.anchor.x, transform.anchor.y), (0.0, 1.0));
    assert_eq!((transform.position.x, transform.position.y), (-0.5, 0.5));

    // No adjustment grades it.
    assert_eq!(
        graded_beneath(sequence, requests.iter().map(|r| r.track)),
        1
    );
}

#[test]
fn along_the_top_and_nothing_at_the_start_or_when_off() {
    let top = project(ProgressBar { top: true, ..BAR });
    let sequence = top.active().unwrap();
    let requests = layer_requests(&top, sequence, TimelineTime::from_seconds(5));
    let bar = requests.last().unwrap();
    assert_eq!(bar.look.transform.anchor.y, 0.0);
    assert_eq!(bar.look.transform.position.y, -0.5);
    assert_eq!(bar.look.transform.scale.x, 0.5);

    // Nothing played yet: nothing to draw.
    assert_eq!(layer_requests(&top, sequence, TimelineTime::ZERO).len(), 1);

    let off = project(ProgressBar::NONE);
    let sequence = off.active().unwrap();
    assert_eq!(
        layer_requests(&off, sequence, TimelineTime::from_seconds(5)).len(),
        1
    );
}
