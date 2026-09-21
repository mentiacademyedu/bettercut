//! The timecode and file name burnt into the picture — the same plan for the
//! preview and the export (§46).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::engine::{LayerSource, burn_in_clip};
use bettercut_project_format::Project;
use bettercut_timeline::{BurnIn, SourceRange, VideoClip};

/// A project with one named shot on the timeline from 0–10 s.
fn project() -> Project {
    let mut project = Project::new("Notes");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/C0042.MP4",
        MediaTime::from_seconds(20),
    );
    asset.label = Some("Interview, wide".to_owned());
    let media = project.add_media(asset);
    let clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    project.active_mut().unwrap().video_tracks[0]
        .insert(clip)
        .unwrap();
    project
}

fn set_burn(project: &mut Project, burn: BurnIn) {
    project.active_mut().unwrap().master.burn_in = burn;
}

#[test]
fn nothing_is_burnt_in_until_it_is_asked_for() {
    let project = project();
    let sequence = project.active().unwrap();
    let at = TimelineTime::from_seconds(3);
    assert!(burn_in_clip(&project, sequence, at).is_none());
    assert!(
        !bettercut_playback::layer_requests(&project, sequence, at)
            .iter()
            .any(|layer| matches!(layer.source, LayerSource::BurnIn))
    );
}

#[test]
fn the_timecode_and_the_file_name_are_drawn_over_the_picture() {
    let mut project = project();
    set_burn(
        &mut project,
        BurnIn {
            timecode: true,
            file_name: true,
            ..BurnIn::default()
        },
    );
    let sequence = project.active().unwrap();
    let at = TimelineTime::from_seconds(3);

    let clip = burn_in_clip(&project, sequence, at).expect("a burn-in");
    assert!(clip.text.contains(&at.format_timecode()), "{}", clip.text);
    // The name the project calls the file, not the path.
    assert!(clip.text.contains("Interview, wide"), "{}", clip.text);
    // Readable over anything: a box behind it, and no title outline.
    assert!(clip.style.background.is_some());
    assert!(clip.style.stroke.is_none());

    // And it is in the plan, over everything, on the lane of none.
    let requests = bettercut_playback::layer_requests(&project, sequence, at);
    let burn = requests
        .iter()
        .find(|layer| matches!(layer.source, LayerSource::BurnIn))
        .expect("the burn-in is not in the plan");
    assert_eq!(burn.source_time.ticks(), at.ticks());
    assert_eq!(burn.track, bettercut_foundation::TrackId::from_u128(0));
    assert_eq!(
        requests.last().map(|layer| layer.source),
        Some(LayerSource::BurnIn),
        "the burn-in should be drawn last"
    );
}

#[test]
fn the_timecode_alone_says_nothing_about_the_file() {
    let mut project = project();
    set_burn(
        &mut project,
        BurnIn {
            timecode: true,
            ..BurnIn::default()
        },
    );
    let sequence = project.active().unwrap();
    let clip = burn_in_clip(&project, sequence, TimelineTime::from_seconds(1)).unwrap();
    assert!(!clip.text.contains("Interview"), "{}", clip.text);
}

/// Where there is no picture there is no file name — and nothing is drawn if
/// that is all that was asked for.
#[test]
fn a_gap_has_no_file_name_to_burn_in() {
    let mut project = project();
    set_burn(
        &mut project,
        BurnIn {
            file_name: true,
            ..BurnIn::default()
        },
    );
    let sequence = project.active().unwrap();
    assert!(burn_in_clip(&project, sequence, TimelineTime::from_seconds(15)).is_none());
    assert!(burn_in_clip(&project, sequence, TimelineTime::from_seconds(5)).is_some());
}

/// The text grows with the frame, so a burn-in on a 4K export is the same size
/// on screen as one on a 1080p export.
#[test]
fn the_size_follows_the_frame() {
    let mut project = project();
    set_burn(
        &mut project,
        BurnIn {
            timecode: true,
            size: 0.05,
            ..BurnIn::default()
        },
    );
    let sequence = project.active().unwrap();
    let small = burn_in_clip(&project, sequence, TimelineTime::ZERO).unwrap();
    assert!(
        (small.style.size - 0.05 * sequence.resolution.height as f32).abs() < 1.0,
        "{}",
        small.style.size
    );
}
