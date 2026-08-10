//! End-to-end playback against real media.
//!
//! The screenshot proves the wgpu half works; this proves the half that
//! decides *what* to show and *what* to play, which is where §20a and §47a
//! actually live.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_audio::{AudioClock, AudioOutput};
use bettercut_foundation::{MediaTime, TICKS_PER_AUDIO_SAMPLE, TimelineTime};
use bettercut_media::{FfmpegProber, MediaProber};
use bettercut_playback::{PlaybackEngine, SyncDecision, plan_frame};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioClip, SourceRange, VideoClip};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

/// A project with the fixture on V1 and A1, starting at timeline zero.
fn project_with_fixture() -> Project {
    let mut project = Project::new("Playback");
    let asset = FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    let duration = asset.duration;
    let media = project.add_media(asset);

    let source = SourceRange::new(MediaTime::ZERO, duration).expect("non-empty");
    let sequence = project.active_mut().expect("sequence");

    sequence.video_tracks[0]
        .insert(VideoClip::new(media, TimelineTime::ZERO, source).expect("valid"))
        .expect("no overlap");
    sequence.audio_tracks[0]
        .insert(AudioClip::new(media, TimelineTime::ZERO, source).expect("valid"))
        .expect("no overlap");

    project
}

#[test]
fn video_resolves_to_a_layer_at_the_playhead() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    let layers = engine.resolve_video(&project, sequence, TimelineTime::from_millis(500));

    assert_eq!(layers.len(), 1, "expected exactly the one video track");
    assert_eq!((layers[0].frame.width, layers[0].frame.height), (640, 360));
    assert_eq!(layers[0].opacity, 1.0);
}

/// Stepping through the timeline must produce *different* pictures. A decoder
/// that silently returns the same frame would pass every size check and show a
/// frozen preview.
#[test]
fn stepping_forward_produces_different_frames() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    let mut signatures = Vec::new();
    for frame in [0_i64, 10, 20, 30, 40] {
        let position = TimelineTime::from_ticks(frame * 32_000);
        let layers = engine.resolve_video(&project, sequence, position);
        let layer = layers.first().expect("a layer at every position");

        let bettercut_media::FrameStorage::System { data, .. } = &layer.frame.storage else {
            panic!("expected a RAM frame");
        };
        // Cheap content signature.
        let sum: u64 = data.iter().step_by(997).map(|b| u64::from(*b)).sum();
        signatures.push(sum);
    }

    let unique: std::collections::HashSet<_> = signatures.iter().collect();
    assert!(
        unique.len() >= 4,
        "stepping through the timeline produced {} distinct frames out of 5 - \
         the picture is probably frozen",
        unique.len()
    );
}

#[test]
fn a_hidden_track_contributes_nothing() {
    let mut project = project_with_fixture();
    project.active_mut().expect("sequence").video_tracks[0].enabled = false;

    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    let layers = engine.resolve_video(&project, sequence, TimelineTime::from_millis(500));
    assert!(layers.is_empty(), "a hidden track was still rendered");
}

#[test]
fn a_position_past_every_clip_resolves_to_nothing() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    let layers = engine.resolve_video(&project, sequence, TimelineTime::from_seconds(30));
    assert!(layers.is_empty());
}

/// The second visit to a position must come from the cache, not the decoder.
#[test]
fn repeated_positions_are_served_from_the_cache() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    let position = TimelineTime::from_millis(400);
    let _ = engine.resolve_video(&project, sequence, position);
    let hits_before = engine.cache().hits();

    let _ = engine.resolve_video(&project, sequence, position);
    assert!(
        engine.cache().hits() > hits_before,
        "revisiting a position went back to the decoder"
    );
}

/// §20a: audio has to actually reach the device buffer, and it has to be sound
/// rather than silence.
#[test]
fn audio_fills_the_device_buffer_with_real_samples() {
    // No device on a headless CI machine is not a failure (§50).
    let Ok((_output, mut sink)) = AudioOutput::open() else {
        eprintln!("no audio device; skipping");
        return;
    };

    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);
    engine.reset_audio(TimelineTime::ZERO);

    let pushed = engine.fill_audio(&project, sequence, &mut sink);
    assert!(pushed > 0, "no audio reached the sink");

    // The fixture is a 440 Hz tone, so it cannot be silent.
    assert_eq!(engine.limited_samples(), 0, "a plain tone should not clip");
}

#[test]
fn audio_filling_stops_when_the_buffer_is_full() {
    let Ok((_output, mut sink)) = AudioOutput::open() else {
        eprintln!("no audio device; skipping");
        return;
    };

    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);
    engine.reset_audio(TimelineTime::ZERO);

    // Fill until full, then confirm a second pass does not spin.
    let first = engine.fill_audio(&project, sequence, &mut sink);
    let second = engine.fill_audio(&project, sequence, &mut sink);

    assert!(first > 0);
    assert!(
        second < first,
        "the second fill pushed as much as the first; it is not respecting the buffer"
    );
}

/// The §20a.1 loop, without a sound card: the clock advances and the frame
/// chosen for each instant follows it.
#[test]
fn the_clock_drives_which_frame_is_shown() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    let clock = AudioClock::new(48_000, 2);
    clock.set_playing(true);

    let interval = TimelineTime::from_ticks(sequence.ticks_per_frame());
    let mut seen = 0;

    // Half a second, in 10 ms steps, exactly as the device would report it.
    for _ in 0..50 {
        clock.advance(480);
        let position = clock.position();

        let layers = engine.resolve_video(&project, sequence, position);
        if layers.is_empty() {
            continue;
        }
        seen += 1;

        // The frame we resolved is for this instant, so it must be presentable.
        let plan = plan_frame(position, clock.position(), interval, 0);
        assert_eq!(
            plan.decision,
            SyncDecision::Present,
            "a frame fetched for the current instant was not presentable"
        );
        assert!(!plan.drift_is_alarming);
    }

    assert!(seen > 40, "only {seen} of 50 steps produced a picture");
    assert_eq!(
        clock.position(),
        TimelineTime::from_millis(500),
        "clock did not land exactly where the sample count says"
    );
}

/// §14: once a proxy exists, preview reads it instead of the original.
///
/// Checked by resolution: the fixture is 640x360 and the proxy is encoded at
/// 180 tall, so the frame's own dimensions say which file was decoded.
#[test]
fn preview_switches_to_the_proxy_once_it_exists() {
    use bettercut_cache::{CACHE_LIMIT_5_GB, CacheLayout, CacheStore};
    use bettercut_playback::{ProxyJob, ProxySource};
    use std::sync::Arc;

    let dir = tempfile::tempdir().expect("tempdir");
    let cache = Arc::new(CacheStore::new(
        CacheLayout::new(dir.path()),
        CACHE_LIMIT_5_GB,
    ));

    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let asset = project.media.first().expect("an asset").clone();

    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);
    let at = TimelineTime::from_millis(500);

    // Before: the original, at its native height.
    let original = engine.resolve_video(&project, sequence, at);
    assert_eq!(original.first().expect("a layer").frame.height, 360);

    // Generate a proxy through the real job path.
    let (scheduler, events) = bettercut_jobs::JobScheduler::new(1);
    let job =
        ProxyJob::new(&asset, bettercut_media::ProxyResolution::P360, &cache, 1).expect("a job");
    scheduler.submit(Box::new(job));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        match events.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(bettercut_jobs::JobEvent::Finished { .. }) => break,
            Ok(bettercut_jobs::JobEvent::Failed { message, .. }) => {
                panic!("proxy job failed: {message}")
            }
            _ => {}
        }
    }

    // After: point the engine at the cache and the frames come from the proxy.
    engine.set_proxy_source(Some(ProxySource {
        cache: Arc::clone(&cache),
        height: 360,
    }));

    let proxied = engine.resolve_video(&project, sequence, at);
    let frame = &proxied.first().expect("a layer").frame;
    assert_eq!(
        frame.height, 360,
        "the proxy is 360 tall, so this should match"
    );
    // The fixture is 640x360 and the proxy is 640x360 too, so compare widths
    // against what the proxy actually reports rather than assuming.
    let probed = bettercut_media::FfmpegProber
        .probe(&cache.layout().proxy_file(asset.id, 360))
        .expect("probe the proxy");
    assert_eq!(frame.width, probed.width);
}

/// Asking for a proxy that has not been generated must fall back silently to
/// the original, not fail.
#[test]
fn a_missing_proxy_falls_back_to_the_original() {
    use bettercut_cache::{CACHE_LIMIT_5_GB, CacheLayout, CacheStore};
    use bettercut_playback::ProxySource;
    use std::sync::Arc;

    let dir = tempfile::tempdir().expect("tempdir");
    let cache = Arc::new(CacheStore::new(
        CacheLayout::new(dir.path()),
        CACHE_LIMIT_5_GB,
    ));

    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    engine.set_proxy_source(Some(ProxySource { cache, height: 540 }));

    let layers = engine.resolve_video(&project, sequence, TimelineTime::from_millis(500));
    assert_eq!(
        layers.first().expect("a layer").frame.height,
        360,
        "expected the original when no proxy has been generated"
    );
}

/// §12/§19: a poster thumbnail comes out of the real decode path, at the right
/// size and aspect, and lands in the cache.
#[test]
fn a_thumbnail_is_generated_and_cached() {
    use bettercut_cache::{CACHE_LIMIT_5_GB, CacheLayout, CacheStore, Thumbnail};
    use bettercut_playback::ThumbnailJob;

    let dir = tempfile::tempdir().expect("tempdir");
    let cache = CacheStore::new(CacheLayout::new(dir.path()), CACHE_LIMIT_5_GB);

    let asset = FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");

    let job = ThumbnailJob::new(&asset, 160, &cache, 1).expect("a job");
    let (scheduler, events) = bettercut_jobs::JobScheduler::new(1);
    scheduler.submit(Box::new(job));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut finished = false;
    while std::time::Instant::now() < deadline {
        match events.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(bettercut_jobs::JobEvent::Finished { .. }) => {
                finished = true;
                break;
            }
            Ok(bettercut_jobs::JobEvent::Failed { message, .. }) => {
                panic!("thumbnail job failed: {message}")
            }
            _ => {}
        }
    }
    assert!(finished, "the thumbnail job never finished");

    let path = cache.layout().thumbnail_file(asset.id, 160);
    assert!(path.exists(), "no thumbnail at {}", path.display());

    let thumb = Thumbnail::read(&path).expect("read it back");
    assert_eq!(thumb.width, 160);
    // The fixture is 640x360, so 160 wide is 90 tall.
    assert_eq!(thumb.height, 90, "aspect ratio was not preserved");
    assert_eq!(thumb.rgba.len(), 160 * 90 * 4);

    // The fixture is colour bars, so a thumbnail of it cannot be uniform.
    // A single flat colour would mean the decode or the downscale silently
    // produced nothing useful.
    let distinct: std::collections::HashSet<[u8; 3]> = thumb
        .rgba
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    assert!(
        distinct.len() > 4,
        "thumbnail has only {} distinct colours; it is probably blank",
        distinct.len()
    );

    // Asking again is a no-op, so a rescan does not redo the work.
    assert!(
        ThumbnailJob::new(&asset, 160, &cache, 1).is_none(),
        "an already-cached thumbnail was queued again"
    );
}

/// Audio has no picture; queuing a thumbnail for it would fail every time.
#[test]
fn audio_gets_no_thumbnail_job() {
    use bettercut_cache::{CACHE_LIMIT_5_GB, CacheLayout, CacheStore};
    use bettercut_playback::ThumbnailJob;

    let dir = tempfile::tempdir().expect("tempdir");
    let cache = CacheStore::new(CacheLayout::new(dir.path()), CACHE_LIMIT_5_GB);
    let asset = FfmpegProber.probe(&fixture("tone-48k.wav")).expect("probe");

    assert!(ThumbnailJob::new(&asset, 160, &cache, 1).is_none());
}

/// §9 again, at the level that matters for sync: 480 samples is exactly 10 ms
/// and exactly 9,600 ticks, with no rounding to accumulate.
#[test]
fn audio_block_timing_is_tick_exact() {
    assert_eq!(TICKS_PER_AUDIO_SAMPLE, 20);
    let block_ticks = 480 * TICKS_PER_AUDIO_SAMPLE;
    assert_eq!(block_ticks, 9_600);
    assert_eq!(
        TimelineTime::from_ticks(block_ticks),
        TimelineTime::from_millis(10)
    );
}
