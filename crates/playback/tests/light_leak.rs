//! A light leak (`engine::light_leak_at`): a warm glow screened over a clip,
//! drifting on its own.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_playback::engine::light_leak_at;

#[test]
fn no_amount_is_no_leak() {
    assert!(light_leak_at(0.0, 1.0).is_none());
    assert!(light_leak_at(f32::NAN, 1.0).is_none());
}

/// Stronger with the amount, never past full, and on the frame.
#[test]
fn the_glow_is_held_to_the_amount_and_the_frame() {
    for step in 0..200 {
        let seconds = f64::from(step) * 0.37;
        let (centre, size, opacity) = light_leak_at(100.0, seconds).unwrap();
        assert!(opacity > 0.0 && opacity <= 1.0, "{opacity}");
        assert!((0.0..=1.0).contains(&centre[0]) && (0.0..=1.0).contains(&centre[1]));
        assert!(size[0] > 0.2 && size[1] > 0.2);
        let (_, _, half) = light_leak_at(50.0, seconds).unwrap();
        assert!(
            (half * 2.0 - opacity).abs() < 1e-5,
            "the amount does not scale it"
        );
    }
    assert_eq!(light_leak_at(500.0, 2.0), light_leak_at(100.0, 2.0));
}

/// It moves: the glow is somewhere else a few seconds on, and at the same
/// instant it is always in the same place.
#[test]
fn the_glow_drifts_and_is_repeatable() {
    let (a, _, _) = light_leak_at(60.0, 0.5).unwrap();
    let (b, _, _) = light_leak_at(60.0, 4.0).unwrap();
    assert!(
        (a[0] - b[0]).abs() + (a[1] - b[1]).abs() > 0.05,
        "it did not move"
    );
    assert_eq!(light_leak_at(60.0, 0.5), light_leak_at(60.0, 0.5));
}

/// With a leak set, the clip is drawn and then the glow over it: a warm solid,
/// screened, cut to a soft oval. Without, the clip alone.
#[test]
fn the_glow_is_a_screened_layer_over_the_clip() {
    use bettercut_foundation::{MediaTime, TimelineTime};
    use bettercut_media::{MediaAsset, MediaKind};
    use bettercut_playback::engine::{LIGHT_LEAK_RGB, LayerSource};
    use bettercut_project_format::Project;
    use bettercut_timeline::{BlendMode, SourceRange, VideoClip};

    let mut project = Project::new("Leak");
    let media = project.add_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(5),
    ));
    let clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(5)).unwrap(),
    )
    .unwrap();
    let id = clip.id;
    project.active_mut().unwrap().video_tracks[0]
        .insert(clip)
        .unwrap();

    let at = TimelineTime::from_seconds(2);
    let plain = bettercut_playback::layer_requests(&project, project.active().unwrap(), at);
    assert_eq!(plain.len(), 1);

    project.active_mut().unwrap().video_tracks[0]
        .get_mut(id)
        .unwrap()
        .light_leak = 70.0;
    let leaked = bettercut_playback::layer_requests(&project, project.active().unwrap(), at);
    assert_eq!(leaked.len(), 2, "no glow layer");
    let glow = &leaked[1];
    assert!(matches!(glow.source, LayerSource::Solid { rgb } if rgb == LIGHT_LEAK_RGB));
    assert_eq!(glow.look.blend, BlendMode::Screen);
    assert!(glow.look.mask.is_some());
    assert!(glow.look.opacity > 0.0 && glow.look.opacity <= 0.7);
}

/// The beat pulse punches in on a marker and eases back to nothing.
#[test]
fn a_beat_pulse_peaks_on_the_marker_and_settles() {
    use bettercut_foundation::TimelineTime;
    use bettercut_playback::engine::{BEAT_PULSE_SECONDS, MAX_BEAT_PULSE, beat_pulse_at};
    use bettercut_timeline::Marker;

    let markers = vec![Marker::at(TimelineTime::from_seconds(2))];
    let at = |ms: i64| beat_pulse_at(100.0, &markers, TimelineTime::from_millis(ms));
    assert_eq!(at(1_990), 1.0, "it pulsed before the beat");
    assert!((at(2_000) - (1.0 + MAX_BEAT_PULSE)).abs() < 1e-5);
    assert!(
        at(2_100) < at(2_000) && at(2_100) > 1.0,
        "it did not ease back"
    );
    let after = 2_000 + (BEAT_PULSE_SECONDS * 1000.0) as i64;
    assert_eq!(at(after), 1.0, "it never settled");
    assert_eq!(
        beat_pulse_at(0.0, &markers, TimelineTime::from_seconds(2)),
        1.0
    );
    assert_eq!(beat_pulse_at(50.0, &[], TimelineTime::from_seconds(2)), 1.0);
}
