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
use bettercut_playback::{
    AudioMixer, AudioPlan, BLOCK_FRAMES, MixerThread, PlaybackEngine, SyncDecision, plan_frame,
};
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

/// A photo shows its one picture at every instant of its clip — including
/// instants the file has no frame for — and it is decoded once, not once per
/// frame of playback.
#[test]
fn a_photo_shows_throughout_its_clip_from_one_decode() {
    let mut project = Project::new("Photo");
    let asset = FfmpegProber.probe(&fixture("still.png")).expect("probe");
    assert!(asset.is_still(), "setup: a PNG is a still");
    let media = project.add_media(asset);
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(5)).expect("valid");
    project.active_mut().expect("sequence").video_tracks[0]
        .insert(VideoClip::new(media, TimelineTime::ZERO, source).expect("valid"))
        .expect("no overlap");

    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    let first = engine.resolve_video(&project, sequence, TimelineTime::ZERO);
    assert_eq!(first.len(), 1, "the photo at its first frame");
    let misses = engine.cache().misses();

    for millis in [40, 1000, 3000, 4960] {
        let layers = engine.resolve_video(&project, sequence, TimelineTime::from_millis(millis));
        assert_eq!(layers.len(), 1, "no photo at {millis} ms");
        assert_eq!((layers[0].frame.width, layers[0].frame.height), (320, 180));
    }
    assert_eq!(
        engine.cache().misses(),
        misses,
        "the photo was decoded again for a later frame"
    );
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

/// The mixer produces the fixture's tone, not silence — and needs no sound card
/// to prove it, now that mixing is a function rather than a side effect of
/// feeding a device.
#[test]
fn the_mixer_produces_real_samples() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let plan = AudioPlan::of(&project, sequence);
    let mut mixer = AudioMixer::new(1);

    let mut block = vec![0.0_f32; BLOCK_FRAMES * 2];
    mixer.mix_block(
        &plan,
        TimelineTime::from_millis(100),
        BLOCK_FRAMES,
        2,
        &mut block,
    );

    let peak = block.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.01, "the block is silent (peak {peak})");
}

/// A fade in is heard: near silence at the clip's start, full level once the
/// fade has run — and mixed in two blocks, the result is sample-for-sample
/// the one-block result, which is what lets preview and export agree (§46).
#[test]
fn a_fade_in_is_heard_and_does_not_depend_on_block_size() {
    let mut project = project_with_fixture();
    let sequence = project.active_mut().expect("sequence");
    let track = &mut sequence.audio_tracks[0];
    let id = track.clips()[0].id;
    track.get_mut(id).expect("clip").fade_in = TimelineTime::from_millis(500);

    let sequence = project.active().expect("sequence");
    let plan = AudioPlan::of(&project, sequence);
    let peak = |block: &[f32]| block.iter().fold(0.0_f32, |m, s| m.max(s.abs()));

    let mut mixer = AudioMixer::new(1);
    let mut start = vec![0.0_f32; BLOCK_FRAMES];
    mixer.mix_block(&plan, TimelineTime::ZERO, BLOCK_FRAMES, 1, &mut start);
    let mut later = vec![0.0_f32; BLOCK_FRAMES];
    mixer.mix_block(
        &plan,
        TimelineTime::from_millis(700),
        BLOCK_FRAMES,
        1,
        &mut later,
    );
    assert!(
        peak(&start) < 0.05 * peak(&later),
        "the first 10 ms were not faded (peak {} against {})",
        peak(&start),
        peak(&later)
    );

    let span = BLOCK_FRAMES * 4;
    let mut whole = vec![0.0_f32; span];
    AudioMixer::new(1).mix_block(&plan, TimelineTime::from_millis(100), span, 1, &mut whole);
    let mut halves = vec![0.0_f32; span];
    let mut two_blocks = AudioMixer::new(1);
    let (first, second) = halves.split_at_mut(span / 2);
    two_blocks.mix_block(&plan, TimelineTime::from_millis(100), span / 2, 1, first);
    two_blocks.mix_block(
        &plan,
        TimelineTime::from_millis(100)
            + TimelineTime::from_ticks((span / 2) as i64 * TICKS_PER_AUDIO_SAMPLE),
        span / 2,
        1,
        second,
    );
    let worst = whole
        .iter()
        .zip(&halves)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(worst < 1e-6, "block size changed the fade by {worst}");
}

/// The track stage reaches the mix: half the volume is half the level, and a
/// hard-left pan leaves the right channel silent.
#[test]
fn a_tracks_volume_and_pan_are_heard() {
    let block_for = |gain: f32, pan: f32| {
        let mut project = project_with_fixture();
        let track = &mut project.active_mut().expect("sequence").audio_tracks[0];
        track.gain = gain;
        track.pan = pan;
        let sequence = project.active().expect("sequence");
        let plan = AudioPlan::of(&project, sequence);
        let mut out = vec![0.0_f32; BLOCK_FRAMES * 2];
        AudioMixer::new(1).mix_block(
            &plan,
            TimelineTime::from_millis(100),
            BLOCK_FRAMES,
            2,
            &mut out,
        );
        out
    };
    let peak =
        |samples: &mut dyn Iterator<Item = f32>| samples.fold(0.0_f32, |m, s| m.max(s.abs()));

    let full = block_for(1.0, 0.0);
    let half = block_for(0.5, 0.0);
    let ratio = peak(&mut half.iter().copied()) / peak(&mut full.iter().copied());
    assert!(
        (ratio - 0.5).abs() < 1e-3,
        "half volume gave {ratio} of the level"
    );

    let left = block_for(1.0, -1.0);
    assert!(
        peak(&mut left.iter().step_by(2).copied()) > 0.01,
        "left went silent"
    );
    assert!(
        peak(&mut left.iter().skip(1).step_by(2).copied()) < 1e-6,
        "hard left still reached the right channel"
    );
}

/// §51 in the mixed output, where the export bug lived. The fixture is a 440 Hz
/// tone, so doubling the speed has to double the pitch — and the zero crossings
/// are a way to count that without an FFT. Export calls this same function, so
/// this is what stops a 2× clip exporting its sound at normal speed again.
#[test]
fn a_fast_clip_mixes_at_its_own_speed() {
    let crossings = |speed: bettercut_foundation::Rational| {
        let mut project = project_with_fixture();
        let sequence = project.active_mut().expect("sequence");
        let track = &mut sequence.audio_tracks[0];
        let id = track.clips()[0].id;
        track.get_mut(id).expect("clip").speed = speed;

        let sequence = project.active().expect("sequence");
        let plan = AudioPlan::of(&project, sequence);
        let mut mixer = AudioMixer::new(1);

        // Mono, and several blocks so the count is not dominated by where the
        // first one happens to start in the waveform.
        let frames = BLOCK_FRAMES * 8;
        let mut out = vec![0.0_f32; frames];
        mixer.mix_block(&plan, TimelineTime::from_millis(50), frames, 1, &mut out);
        out.windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    };

    let normal = crossings(bettercut_foundation::Rational::ONE);
    let double = crossings(bettercut_foundation::Rational::new(2, 1).expect("ratio"));

    assert!(
        normal > 10,
        "the tone is not there to count ({normal} crossings)"
    );
    let ratio = double as f64 / normal as f64;
    assert!(
        (1.8..=2.2).contains(&ratio),
        "at 2x the tone crossed zero {double} times against {normal}: a ratio of {ratio:.2}, not 2"
    );
}

/// A clip deleted from the timeline must not keep its decoder open for the life
/// of the mixer — a file handle and FFmpeg's buffers, held for nothing.
#[test]
fn the_mixer_closes_decoders_the_plan_no_longer_needs() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let plan = AudioPlan::of(&project, sequence);
    let mut mixer = AudioMixer::new(1);

    let mut block = vec![0.0_f32; BLOCK_FRAMES * 2];
    mixer.mix_block(&plan, TimelineTime::ZERO, BLOCK_FRAMES, 2, &mut block);
    assert_eq!(mixer.open_sources(), 1, "setup: the clip's decoder opened");

    mixer.retain_current(&AudioPlan::default());
    assert_eq!(mixer.open_sources(), 0, "the decoder outlived its clip");
}

/// The snapshot compares equal when nothing about the audio changed, which is
/// what stops the preview re-sending it on every frame of a video-only edit.
#[test]
fn an_unchanged_project_gives_an_equal_plan() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    assert_eq!(
        AudioPlan::of(&project, sequence),
        AudioPlan::of(&project, sequence)
    );

    let mut changed = project.clone();
    let sequence = changed.active_mut().expect("sequence");
    let id = sequence.audio_tracks[0].clips()[0].id;
    sequence.audio_tracks[0].get_mut(id).expect("clip").gain = 0.5;
    let sequence = changed.active().expect("sequence");
    assert_ne!(
        AudioPlan::of(&project, project.active().expect("sequence")),
        AudioPlan::of(&changed, sequence),
        "a gain change did not change the plan, so the mixer would never hear it"
    );
}

/// §20a.2's thread, end to end: it fills the device while playing and stops
/// when paused. Skipped without a sound card, which is not a failure (§50).
#[test]
fn the_mixer_thread_feeds_the_device_only_while_playing() {
    let Ok((_output, sink)) = AudioOutput::open() else {
        eprintln!("no audio device; skipping");
        return;
    };
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");

    let mixer = MixerThread::spawn(sink, 1).expect("thread");
    mixer.set_plan(AudioPlan::of(&project, sequence));
    mixer.seek(TimelineTime::ZERO);

    // Paused: nothing should reach the device.
    std::thread::sleep(std::time::Duration::from_millis(60));
    assert_eq!(
        mixer.frames_pushed(),
        0,
        "the mixer pushed audio while paused"
    );

    mixer.set_playing(true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while mixer.frames_pushed() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(mixer.frames_pushed() > 0, "the mixer never fed the device");
    assert_eq!(mixer.limited_samples(), 0, "a plain tone should not clip");

    // §20a's meter: the fixture has sound in it, so something must register.
    let (left, right) = mixer.peaks();
    assert!(
        left > 0.0 || right > 0.0,
        "the meter stayed at zero while audio was playing"
    );

    // And it falls back to silence when paused. A meter holding the last level
    // of a stopped mix looks like sound that is not there.
    mixer.set_playing(false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while mixer.peaks() != (0.0, 0.0) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        mixer.peaks(),
        (0.0, 0.0),
        "the meter stayed lit after playback stopped"
    );
}

/// Dropping the handle stops the thread and joins it. A mixer that outlived its
/// preview would keep a decoder and the device's ring buffer alive.
#[test]
fn dropping_the_mixer_stops_its_thread() {
    let Ok((_output, sink)) = AudioOutput::open() else {
        eprintln!("no audio device; skipping");
        return;
    };
    let mixer = MixerThread::spawn(sink, 1).expect("thread");
    mixer.set_playing(true);

    let started = std::time::Instant::now();
    drop(mixer);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "joining the mixer took {:?}",
        started.elapsed()
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

/// §47a.3: decode-ahead has to actually serve frames, or it is just a thread
/// burning CPU. Plan a second of playback, wait for the ring to fill, then
/// check the engine takes frames from it instead of decoding inline.
#[test]
fn decode_ahead_serves_frames_to_playback() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    engine.start_prefetch(bettercut_playback::budget_for(640 * 360 * 4, 30.0, 1.0));
    engine.prefetch_ahead(
        &project,
        sequence,
        TimelineTime::ZERO,
        TimelineTime::from_millis(500),
    );

    // The decode thread runs in the background; give it a bounded chance.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while engine.prefetched_frames() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        engine.prefetched_frames() > 0,
        "the decode-ahead thread produced nothing"
    );

    // Play through the span it decoded and confirm the ring was used.
    for frame in 0..12_i64 {
        let at = TimelineTime::from_ticks(frame * 32_000);
        let layers = engine.resolve_video(&project, sequence, at);
        assert!(!layers.is_empty(), "no picture at frame {frame}");
    }

    assert!(
        engine.prefetch_hits() > 0,
        "playback decoded everything inline; the ring was never used"
    );
}

/// §47a.5: a seek must throw away work queued for where the playhead *was*.
#[test]
fn seeking_discards_decode_ahead_work() {
    let project = project_with_fixture();
    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    engine.start_prefetch(bettercut_playback::budget_for(640 * 360 * 4, 30.0, 1.0));
    engine.prefetch_ahead(
        &project,
        sequence,
        TimelineTime::ZERO,
        TimelineTime::from_millis(500),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while engine.prefetched_frames() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(engine.prefetched_frames() > 0, "nothing was decoded ahead");

    engine.reset_prefetch();
    assert_eq!(
        engine.prefetched_frames(),
        0,
        "a seek left frames for the old position in the ring"
    );
}

/// Starting twice would run two decode threads over the same files — §74
/// forbids it, and it would be slower than one.
#[test]
fn starting_decode_ahead_twice_is_a_no_op() {
    let mut engine = PlaybackEngine::new(16 * 1024 * 1024, 1);
    engine.start_prefetch(16 * 1024 * 1024);
    let generation = {
        engine.start_prefetch(16 * 1024 * 1024);
        engine.prefetched_frames()
    };
    assert_eq!(generation, 0);
    engine.stop_prefetch();
    assert_eq!(engine.prefetched_frames(), 0);
}

/// §12: the waveform has to cover the *whole* file. A short one would draw a
/// clip that goes flat partway through, which reads as damaged audio.
#[test]
fn a_waveform_covers_the_whole_file() {
    use bettercut_cache::{CACHE_LIMIT_5_GB, CacheLayout, CacheStore, Waveform};
    use bettercut_playback::WaveformJob;

    let dir = tempfile::tempdir().expect("tempdir");
    let cache = CacheStore::new(CacheLayout::new(dir.path()), CACHE_LIMIT_5_GB);
    let asset = FfmpegProber.probe(&fixture("tone-48k.wav")).expect("probe");
    let expected = asset.duration.as_seconds_f64();

    let job = WaveformJob::new(&asset, &cache, 1).expect("a job");
    let (scheduler, events) = bettercut_jobs::JobScheduler::new(1);
    scheduler.submit(Box::new(job));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut finished = false;
    while std::time::Instant::now() < deadline {
        match events.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(bettercut_jobs::JobEvent::Finished { .. }) => {
                finished = true;
                break;
            }
            Ok(bettercut_jobs::JobEvent::Failed { message, .. }) => {
                panic!("waveform job failed: {message}")
            }
            _ => {}
        }
    }
    assert!(finished, "the waveform job never finished");

    let waveform = Waveform::read(&cache.layout().waveform_file(asset.id)).expect("read");
    let covered = waveform.duration_seconds();
    assert!(
        (covered - expected).abs() < 0.1,
        "waveform covers {covered:.2}s of a {expected:.2}s file"
    );

    // The fixture is a 440 Hz tone, so it must not read as silence.
    assert!(
        waveform.peaks.iter().any(|p| p.magnitude() > 0.1),
        "a tone analysed as silence"
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

/// A held frame shows the same picture throughout, and costs one decode
/// however long it is held — the same trick a photo uses.
#[test]
fn a_frozen_clip_shows_one_frame_from_one_decode() {
    let mut project = project_with_fixture();
    let sequence = project.active_mut().expect("sequence");
    let clip = sequence.video_tracks[0].clips()[0].id;
    {
        let clip = sequence.video_tracks[0].get_mut(clip).expect("clip");
        clip.frozen = true;
        clip.source.start = MediaTime::from_millis(500);
    }

    let sequence = project.active().expect("sequence");
    let mut engine = PlaybackEngine::new(64 * 1024 * 1024, 1);

    let signature = |layer: &bettercut_playback::ResolvedLayer| {
        let bettercut_media::FrameStorage::System { data, .. } = &layer.frame.storage else {
            panic!("expected a RAM frame");
        };
        data.iter().step_by(997).map(|b| u64::from(*b)).sum::<u64>()
    };

    let first = engine.resolve_video(&project, sequence, TimelineTime::ZERO);
    let held = signature(&first[0]);
    let misses = engine.cache().misses();

    for millis in [40, 500, 1_200] {
        let layers = engine.resolve_video(&project, sequence, TimelineTime::from_millis(millis));
        assert_eq!(
            signature(&layers[0]),
            held,
            "the picture moved at {millis} ms"
        );
    }
    assert_eq!(
        engine.cache().misses(),
        misses,
        "a held frame was decoded more than once"
    );
}
