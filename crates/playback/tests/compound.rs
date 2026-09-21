//! A compound clip plays the sequence folded inside it — the same plan for the
//! preview and the export (§46), with the compound's own framing and fade
//! carried onto every layer it holds.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{Generated, MediaAsset, MediaKind};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioClip, SourceRange, Vec2, VideoClip};

fn secs(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

/// A project whose main sequence holds one compound clip, and whose inner
/// sequence holds two shots of two seconds each with sound under them.
fn project_with_a_compound() -> (Project, bettercut_foundation::ClipId) {
    let mut project = Project::new("Compound");
    let footage = project.add_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/inside.mp4",
        MediaTime::from_seconds(10),
    ));

    let mut inner = bettercut_timeline::Sequence::new(
        "Inside",
        bettercut_timeline::Resolution::HD_1080,
        bettercut_foundation::FrameRate::FPS_30,
    )
    .unwrap();
    inner
        .video_tracks
        .push(bettercut_timeline::VideoTrack::new("V1"));
    inner
        .audio_tracks
        .push(bettercut_timeline::AudioTrack::new("A1"));
    for step in 0..2 {
        let at = TimelineTime::from_seconds(step * 2);
        let source = SourceRange::new(
            MediaTime::from_seconds(step * 2),
            MediaTime::from_seconds(step * 2 + 2),
        )
        .unwrap();
        let mut clip = VideoClip::new(footage, at, source).unwrap();
        // Something to recognise the composition by: half size, off centre.
        clip.transform.scale = Vec2::new(0.5, 0.5);
        clip.transform.position = Vec2::new(0.2, 0.0);
        clip.opacity = 0.5;
        inner.video_tracks[0].insert(clip).unwrap();
        inner.audio_tracks[0]
            .insert(AudioClip::new(footage, at, source).unwrap())
            .unwrap();
    }
    let inner_id = inner.id;
    project.sequences.push(inner);

    let media = project.add_media(MediaAsset::generated(
        Generated::Compound { sequence: inner_id },
        1920,
        1080,
    ));
    let mut compound = VideoClip::new(
        media,
        secs(1),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    compound.transform.scale = Vec2::new(0.5, 0.5);
    compound.opacity = 0.5;
    let id = compound.id;
    project.active_mut().unwrap().video_tracks[0]
        .insert(compound)
        .unwrap();
    (project, id)
}

#[test]
fn a_compound_draws_what_is_inside_it_with_its_own_look_on_top() {
    let (project, _) = project_with_a_compound();
    let sequence = project.active().unwrap();

    // A second in: the compound is a second old, so its first inner shot is
    // playing.
    let requests = bettercut_playback::layer_requests(&project, sequence, secs(2));
    assert_eq!(requests.len(), 1, "one inner layer should be drawn");
    let layer = &requests[0];
    assert!(
        layer.source.media().is_some(),
        "the layer should read the footage inside, not the compound"
    );
    // Half of a half, and both opacities.
    assert!((layer.look.transform.scale.x - 0.25).abs() < 1e-5);
    assert!((layer.look.opacity - 0.25).abs() < 1e-5);
    // The inner offset is scaled by the compound it sits in.
    assert!((layer.look.transform.position.x - 0.1).abs() < 1e-5);

    // Three seconds in the compound is playing its second inner shot.
    let later = bettercut_playback::layer_requests(&project, sequence, secs(4));
    assert_eq!(later.len(), 1);
    assert_eq!(
        later[0].source_time,
        MediaTime::from_seconds(3),
        "the second shot should be read a second into itself"
    );
}

/// Before and after the compound there is nothing to draw: what is inside it
/// only plays where it plays.
#[test]
fn nothing_inside_plays_outside_the_compound() {
    let (project, _) = project_with_a_compound();
    let sequence = project.active().unwrap();
    assert!(bettercut_playback::layer_requests(&project, sequence, secs(0)).is_empty());
    assert!(bettercut_playback::layer_requests(&project, sequence, secs(6)).is_empty());
}

/// And its sound plays where the compound does: the mixer is handed ordinary
/// tracks, with the inner clips moved onto the parent's timeline.
#[test]
fn a_compounds_sound_is_mixed_where_the_compound_plays() {
    let (project, _) = project_with_a_compound();
    let sequence = project.active().unwrap();
    let plan = bettercut_playback::mixer::AudioPlan::of(&project, sequence);

    let clips: Vec<(i64, i64)> = plan
        .tracks
        .iter()
        .flat_map(|track| track.clips())
        .map(|clip| {
            (
                clip.timeline.start.ticks() / bettercut_foundation::TICKS_PER_SECOND,
                clip.timeline.end.ticks() / bettercut_foundation::TICKS_PER_SECOND,
            )
        })
        .collect();
    assert_eq!(clips, vec![(1, 3), (3, 5)], "{clips:?}");
}

/// A compound trimmed at the front shows — and plays — from further in.
#[test]
fn trimming_the_compound_moves_the_window_into_it() {
    let (mut project, compound) = project_with_a_compound();
    {
        let sequence = project.active_mut().unwrap();
        let clip = sequence.video_tracks[0].get_mut(compound).unwrap();
        clip.source =
            SourceRange::new(MediaTime::from_seconds(2), MediaTime::from_seconds(4)).unwrap();
        clip.timeline = bettercut_timeline::TimelineRange {
            start: secs(1),
            end: secs(3),
        };
    }
    let sequence = project.active().unwrap();

    // At the very start of the compound it is two seconds into the inner
    // sequence, which is the second shot.
    let requests = bettercut_playback::layer_requests(&project, sequence, secs(1));
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].source_time, MediaTime::from_seconds(2));

    // And only the part of the inner sound the window shows is mixed.
    let plan = bettercut_playback::mixer::AudioPlan::of(&project, sequence);
    let clips: Vec<(i64, i64)> = plan
        .tracks
        .iter()
        .flat_map(|track| track.clips())
        .map(|clip| {
            (
                clip.timeline.start.ticks() / bettercut_foundation::TICKS_PER_SECOND,
                clip.timeline.end.ticks() / bettercut_foundation::TICKS_PER_SECOND,
            )
        })
        .collect();
    assert_eq!(clips, vec![(1, 3)], "{clips:?}");
}

/// A multicam clip shows one camera: the lane its angle names, and no other.
#[test]
fn a_multicam_shows_only_the_angle_it_names() {
    let (mut project, compound) = project_with_a_compound();
    // A second camera on a second lane inside, over the whole span.
    let inner_id = project
        .media
        .iter()
        .find_map(|asset| asset.generated.and_then(|g| g.compound()))
        .expect("a compound");
    let second = project.add_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/other.mp4",
        MediaTime::from_seconds(10),
    ));
    {
        let inner = project
            .sequences
            .iter_mut()
            .find(|sequence| sequence.id == inner_id)
            .expect("the inner sequence");
        inner
            .video_tracks
            .push(bettercut_timeline::VideoTrack::new("V2"));
        let clip = VideoClip::new(
            second,
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
        )
        .unwrap();
        inner.video_tracks[1].insert(clip).unwrap();
    }

    let sequence = project.active().unwrap();
    // With no angle set, both cameras draw — an ordinary compound.
    assert_eq!(
        bettercut_playback::layer_requests(&project, sequence, secs(2)).len(),
        2
    );

    // Angle two: only the second camera.
    project.active_mut().unwrap().video_tracks[0]
        .get_mut(compound)
        .unwrap()
        .angle = Some(1);
    let sequence = project.active().unwrap();
    let requests = bettercut_playback::layer_requests(&project, sequence, secs(2));
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].source.media(), Some(second));

    // And angle one is the first camera again.
    project.active_mut().unwrap().video_tracks[0]
        .get_mut(compound)
        .unwrap()
        .angle = Some(0);
    let sequence = project.active().unwrap();
    let requests = bettercut_playback::layer_requests(&project, sequence, secs(2));
    assert_eq!(requests.len(), 1);
    assert_ne!(requests[0].source.media(), Some(second));
}
