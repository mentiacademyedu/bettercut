//! The audio visualizer: loudness measured from a clip's waveform, and bars in
//! the frame plan.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_cache::{Peak, Waveform};
use bettercut_foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_playback::{LayerSource, layer_requests, visualizer_levels};
use bettercut_project_format::Project;
use bettercut_timeline::visualizer::{LEVELS_PER_SECOND, Visualizer};
use bettercut_timeline::{AudioClip, SourceRange};

/// Four seconds at 200 peaks a second: quiet for two, loud for two.
fn waveform() -> Waveform {
    let peaks = (0..800)
        .map(|i| {
            let loud: i8 = if i < 400 { 12 } else { 120 };
            Peak {
                min: -loud,
                max: loud,
            }
        })
        .collect();
    Waveform::new(200, peaks).unwrap()
}

#[test]
fn levels_follow_the_clips_loudness_on_the_timeline() {
    let clip = AudioClip::new(
        MediaId::new(),
        TimelineTime::from_seconds(3),
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap(),
    )
    .unwrap();
    let (start, levels) = visualizer_levels(&clip, &waveform()).unwrap();
    assert_eq!(start, TimelineTime::from_seconds(3));
    assert_eq!(levels.len(), 4 * LEVELS_PER_SECOND as usize);
    // The loudest is 1; the quiet half is about a tenth of it.
    assert!((levels[100] - 1.0).abs() < 1e-3);
    assert!(levels[10] < 0.15, "{}", levels[10]);
}

#[test]
fn the_plan_draws_a_bar_a_layer_while_the_sound_is_loud() {
    let mut project = Project::new("Visualizer");
    let mut levels = vec![0.0; 30];
    levels.extend(vec![1.0; 30]);
    let sequence = project.active_mut().unwrap();
    sequence.visualizer = Some(Visualizer {
        bars: 10,
        ..Visualizer::new(levels, TimelineTime::ZERO)
    });
    let sequence = project.active().unwrap();

    let quiet = layer_requests(&project, sequence, TimelineTime::from_millis(500));
    assert!(quiet.is_empty(), "bars drawn over silence");

    let loud = layer_requests(&project, sequence, TimelineTime::from_millis(1500));
    assert_eq!(loud.len(), 10);
    for (i, bar) in loud.iter().enumerate() {
        assert_eq!(
            bar.source,
            LayerSource::Solid {
                rgb: [255, 255, 255]
            }
        );
        // Standing on the bottom edge, spread evenly across.
        assert_eq!(bar.look.transform.anchor.y, 1.0);
        assert_eq!(bar.look.transform.position.y, 0.5);
        let expected_x = -0.5 + (i as f32 + 0.5) / 10.0;
        assert!((bar.look.transform.position.x - expected_x).abs() < 1e-5);
        assert!(bar.look.transform.scale.y > 0.0 && bar.look.transform.scale.y <= 0.2);
    }
}
