//! The watermark: a logo drawn last, in its corner, sized to the frame's width.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{FrameRate, MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::{LayerSource, layer_requests};
use bettercut_project_format::Project;
use bettercut_timeline::watermark::{Watermark, WatermarkCorner};
use bettercut_timeline::{SourceRange, VideoClip, fit_scale};

#[test]
fn the_logo_is_drawn_last_in_its_corner_at_its_share_of_the_width() {
    let mut project = Project::new("Logo");
    let shot = project.add_media(
        MediaAsset::new(
            MediaKind::Video,
            "C:/media/shot.mp4",
            MediaTime::from_seconds(5),
        )
        .with_video(1920, 1080, FrameRate::new(30, 1).unwrap()),
    );
    let mut logo_asset = MediaAsset::new(MediaKind::Image, "C:/media/logo.png", MediaTime::ZERO);
    logo_asset.width = 400;
    logo_asset.height = 400;
    let logo = project.add_media(logo_asset);
    let sequence = project.active_mut().unwrap();
    sequence.video_tracks[0]
        .insert(
            VideoClip::new(
                shot,
                TimelineTime::ZERO,
                SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(5)).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    sequence.watermark = Some(Watermark {
        corner: WatermarkCorner::TopLeft,
        size: 0.2,
        opacity: 0.5,
        ..Watermark::new(logo)
    });
    let sequence = project.active().unwrap();

    let requests = layer_requests(&project, sequence, TimelineTime::from_seconds(1));
    assert_eq!(requests.len(), 2);
    let mark = requests.last().unwrap();
    assert_eq!(mark.source, LayerSource::Media(logo));
    assert_eq!(mark.look.opacity, 0.5);
    let (fit_x, _) = fit_scale(1.0, 1920.0 / 1080.0);
    assert!((mark.look.transform.scale.x * fit_x - 0.2).abs() < 1e-4);
    assert_eq!(
        (mark.look.transform.anchor.x, mark.look.transform.anchor.y),
        (0.0, 0.0)
    );
    assert!(mark.look.transform.position.x < -0.4 && mark.look.transform.position.y < -0.4);

    // It shows even where there is no clip.
    let empty = layer_requests(&project, sequence, TimelineTime::from_seconds(30));
    assert_eq!(empty.len(), 1);
}
