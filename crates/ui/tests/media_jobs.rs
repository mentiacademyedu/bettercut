//! The proxy pipeline, end to end (§13, §14, §67, §74).
//!
//! `crates/playback/tests` proves a `ProxyJob` produces a decodable file, and
//! `crates/cache` proves eviction never touches source media. What is left —
//! and what actually broke in every earlier attempt — is the wiring: does
//! importing queue anything, does a finished encode come back as a `MediaId`
//! the preview can act on, and does a failure surface rather than vanish?

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bettercut_cache::{CACHE_LIMIT_5_GB, CacheLayout, CacheStore};
use bettercut_editor_core::Editor;
use bettercut_editor_core::media::{MediaAsset, MediaKind, MediaProber};
use bettercut_editor_core::project_format::PerformanceMode;
use bettercut_ui::{MediaJobs, MediaUpdate};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

fn store(dir: &tempfile::TempDir) -> CacheStore {
    CacheStore::new(CacheLayout::new(dir.path()), CACHE_LIMIT_5_GB)
}

/// Spin `poll()` until nothing is in flight, gathering everything it reported.
///
/// Encoding runs on a real job thread, so the test has to wait on it the same
/// way the UI does — by polling, not by blocking on a handle.
fn drain(manager: &mut MediaJobs, timeout: Duration) -> MediaUpdate {
    let deadline = Instant::now() + timeout;
    let mut all = MediaUpdate::default();

    while Instant::now() < deadline {
        let update = manager.poll();
        all.ready.extend(update.ready);
        all.messages.extend(update.messages);
        all.failures.extend(update.failures);

        if manager.active_jobs() == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    all
}

/// The fixture is 640x360 h264 8-bit, which §13's rules say does *not* need a
/// proxy. Claiming it does would mean every phone clip gets re-encoded.
#[test]
fn ordinary_footage_is_not_proxied() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Proxies");
    editor
        .import_file(&fixture("ntsc-2997.mp4"))
        .expect("probe");

    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 1, 1);
    manager.scan(&editor);

    assert_eq!(
        manager.active_jobs(),
        0,
        "640x360 h264 should not have been queued"
    );
}

/// The success path: import something §13 considers heavy, and the proxy that
/// comes back is reported against the right `MediaId`.
///
/// The asset is the real fixture with its declared resolution raised, so the
/// encode is genuine while the test stays fast. What is under test is the
/// manager, not the prober.
#[test]
fn a_heavy_import_produces_a_proxy_and_reports_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Proxies");

    let mut asset = bettercut_media::FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    asset.height = 2160;
    asset.width = 3840;
    assert!(
        asset.should_generate_proxy(),
        "the fixture-under-test must qualify"
    );
    let media = editor.import_media(asset);

    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 1, 1);
    let queued = manager.scan(&editor);

    assert_eq!(manager.active_jobs(), 1, "nothing was queued on import");
    assert!(
        queued.iter().any(|m| m.contains("proxy")),
        "the user was told nothing about the work starting: {queued:?}"
    );

    let update = drain(&mut manager, Duration::from_secs(120));
    assert!(
        update.failures.is_empty(),
        "the encode failed: {:?}",
        update.failures
    );
    assert_eq!(
        update.ready,
        vec![media],
        "the finished proxy was not reported against its asset"
    );

    // §14: the file the preview will read has to actually be there.
    let source = manager.source();
    let path = source.cache.layout().proxy_file(media, source.height);
    assert!(path.exists(), "no proxy landed at {}", path.display());
    assert!(path.metadata().unwrap().len() > 0, "the proxy is empty");
}

/// Scanning twice must not queue the work twice, because `draw` calls it after
/// every import and the library still holds everything imported before.
#[test]
fn rescanning_does_not_requeue() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Proxies");

    let mut asset = bettercut_media::FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    asset.height = 2160;
    editor.import_media(asset);

    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 1, 1);
    manager.scan(&editor);
    let after_first = manager.active_jobs();
    manager.scan(&editor);

    assert_eq!(
        manager.active_jobs(),
        after_first,
        "a second scan queued the same asset again"
    );
    manager.cancel_all();
}

/// §13: "Allow the user to disable automatic proxies." Off means off, even for
/// media that would otherwise qualify.
#[test]
fn disabling_automatic_proxies_queues_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Proxies");

    let mut asset = bettercut_media::FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    asset.height = 2160;
    editor.import_media(asset);
    editor
        .dispatch(bettercut_editor_core::Command::ChangeSetting {
            change: bettercut_editor_core::SettingChange::AutoGenerateProxies(false),
        })
        .expect("settings change");

    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 1, 1);
    let messages = manager.scan(&editor);

    assert_eq!(manager.active_jobs(), 0);
    assert!(messages.is_empty(), "it announced work it did not do");
}

/// Changing the quality setting has to reach the running manager, or the
/// control would look like it did something and change nothing until restart.
#[test]
fn changing_quality_repoints_the_source_and_requeues() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Proxies");

    let mut asset = bettercut_media::FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    asset.height = 2160;
    editor.import_media(asset);

    // Set the mode explicitly: `new_project` picks one from the host's
    // hardware, so hard-coding a starting point here would make the test's
    // result depend on the machine running it.
    editor
        .dispatch(bettercut_editor_core::Command::ChangeSetting {
            change: bettercut_editor_core::SettingChange::PerformanceMode(
                PerformanceMode::Performance,
            ),
        })
        .expect("settings change");

    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Performance, 1, 1);
    let (source, _) = manager.sync(&editor);
    assert!(source.is_none(), "nothing changed on the first sync");
    let before = manager.source().height;

    editor
        .dispatch(bettercut_editor_core::Command::ChangeSetting {
            change: bettercut_editor_core::SettingChange::PerformanceMode(PerformanceMode::Quality),
        })
        .expect("settings change");

    let (source, messages) = manager.sync(&editor);
    let source = source.expect("the preview was not given a new source");
    assert!(
        source.height > before,
        "expected a taller proxy after switching to Sharpest, got {}",
        source.height
    );
    assert_eq!(manager.active_jobs(), 1, "the asset was not requeued");
    assert!(
        messages.iter().any(|m| m.contains("quality")),
        "the change was not reported: {messages:?}"
    );

    manager.cancel_all();
}

/// `sync` runs every frame, so it must not narrate every frame.
#[test]
fn a_steady_state_sync_says_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Proxies");

    let mut asset = bettercut_media::FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    asset.height = 2160;
    editor.import_media(asset);

    // Matched to the project so the first sync is a plain scan, not a mode change.
    let mode = editor.project().settings.performance_mode;
    let mut manager = MediaJobs::new(store(&dir), mode, 1, 1);
    let (_, first) = manager.sync(&editor);
    assert!(!first.is_empty(), "the first sync should announce the work");

    for _ in 0..5 {
        let (source, messages) = manager.sync(&editor);
        assert!(source.is_none());
        assert!(messages.is_empty(), "repeated status spam: {messages:?}");
    }

    manager.cancel_all();
}

/// §74: "Never silently ignore FFmpeg failures." A file that cannot be opened
/// has to come back as something the status bar can show.
#[test]
fn a_failed_encode_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Proxies");

    // Points nowhere, but is marked present so the manager does not skip it as
    // §66 missing media — the failure must come from FFmpeg, not from a guard.
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        dir.path().join("not-a-video.mp4"),
        bettercut_editor_core::foundation::MediaTime::from_seconds(5),
    )
    .with_video(
        3840,
        2160,
        bettercut_editor_core::foundation::FrameRate::FPS_30,
    );
    asset.missing = false;
    editor.import_media(asset);

    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 1, 1);
    manager.scan(&editor);
    assert_eq!(manager.active_jobs(), 1);

    let update = drain(&mut manager, Duration::from_secs(30));
    assert!(
        !update.failures.is_empty(),
        "a missing source file failed silently"
    );
    assert!(update.ready.is_empty(), "a failed encode reported success");
}

/// A frame that cannot be saved says so as a failure, rather than vanishing —
/// the user pressed a button and is waiting for a file.
///
/// A sequence that is not in the project fails before any GPU or decoder is
/// touched, so this runs anywhere.
#[test]
fn a_still_that_fails_is_reported() {
    use bettercut_editor_core::foundation::{SequenceId, TimelineTime};
    use bettercut_export::StillJob;

    let dir = tempfile::tempdir().unwrap();
    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 1, 1);
    let (editor, _events) = Editor::new_project("Still");
    manager.submit_still(StillJob::new(
        editor.project().clone(),
        SequenceId::new(),
        TimelineTime::ZERO,
        dir.path().join("frame.png"),
    ));

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut failures = Vec::new();
    while Instant::now() < deadline && failures.is_empty() {
        failures.extend(manager.poll().failures);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].starts_with("Could not save the frame"),
        "{failures:?}"
    );
}

/// Exports asked for together — one edit in several shapes — run one after
/// another, and a failure does not strand the ones behind it: a vertical cut
/// that fails says nothing about the square one.
///
/// Empty sequences, so each export fails at once without touching a GPU or an
/// encoder; what is under test is the queue, not the export.
#[test]
fn exports_asked_for_together_run_one_after_another() {
    use bettercut_export::{ExportJob, ExportSettings};

    let dir = tempfile::tempdir().unwrap();
    // Room for several heavy jobs, so one-at-a-time is the queue's doing and
    // not the scheduler's limit.
    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 4, 1);
    let (editor, _events) = Editor::new_project("Queue");
    let sequence = editor.active_sequence().unwrap();

    for name in ["main", "vertical", "square"] {
        let settings =
            ExportSettings::for_sequence(dir.path().join(format!("{name}.mp4")), sequence);
        manager.submit_export(ExportJob::new(editor.project(), sequence.id, settings));
    }
    assert!(
        manager.export_in_flight().is_some(),
        "the first did not start"
    );
    assert_eq!(manager.exports_waiting(), 2, "the others did not wait");

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut failures = Vec::new();
    let mut waiting_seen = vec![manager.exports_waiting()];
    while Instant::now() < deadline {
        let update = manager.poll();
        failures.extend(update.failures);
        let waiting = manager.exports_waiting();
        if waiting_seen.last() != Some(&waiting) {
            waiting_seen.push(waiting);
        }
        if manager.export_in_flight().is_none() && waiting == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        failures.len(),
        3,
        "every export should have run and reported: {failures:?}"
    );
    // Never more waiting than before: nothing re-queued or skipped ahead. (Two
    // may leave in one poll — an empty export fails in well under a frame.)
    assert!(
        waiting_seen.windows(2).all(|pair| pair[1] < pair[0]) && waiting_seen.last() == Some(&0),
        "the queue did not drain in order: {waiting_seen:?}"
    );
}

/// A waiting export moved up runs before the ones it passed; moving off the
/// queue's ends is refused.
#[test]
fn a_waiting_export_can_be_moved_up_the_queue() {
    use bettercut_export::{ExportJob, ExportSettings};

    let dir = tempfile::tempdir().unwrap();
    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 4, 1);
    let (editor, _events) = Editor::new_project("Queue");
    let sequence = editor.active_sequence().unwrap();
    for name in ["main", "second", "third", "fourth"] {
        let settings =
            ExportSettings::for_sequence(dir.path().join(format!("{name}.mp4")), sequence);
        manager.submit_export(ExportJob::new(editor.project(), sequence.id, settings));
    }
    let before = manager.waiting_export_labels();
    assert_eq!(before.len(), 3);

    assert!(manager.move_waiting_export(2, 0));
    let after = manager.waiting_export_labels();
    assert_eq!(
        after[0], before[2],
        "the moved export is not first: {after:?}"
    );
    assert_eq!(
        &after[1..],
        &before[..2],
        "the others did not close up: {after:?}"
    );

    assert!(
        !manager.move_waiting_export(3, 0),
        "moved from past the end"
    );
    assert!(!manager.move_waiting_export(0, 3), "moved to past the end");
    assert_eq!(manager.waiting_export_labels(), after);

    manager.clear_waiting_exports();
}

/// A frame grab for a sequence that is not there fails, and says it was the
/// copy that failed.
#[test]
fn a_frame_grab_that_fails_is_reported() {
    use bettercut_editor_core::foundation::{SequenceId, TimelineTime};
    use bettercut_export::FrameGrabJob;

    let dir = tempfile::tempdir().unwrap();
    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 1, 1);
    let (editor, _events) = Editor::new_project("Grab");
    manager.submit_frame_grab(FrameGrabJob::new(
        editor.project().clone(),
        SequenceId::new(),
        TimelineTime::ZERO,
    ));

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut failures = Vec::new();
    while Instant::now() < deadline && failures.is_empty() {
        failures.extend(manager.poll().failures);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].starts_with("Could not copy the frame"),
        "{failures:?}"
    );
}

/// A frame grab comes back as the sequence's full-size picture — here a title
/// on black, so something is drawn — ready for the clipboard. Skips without a
/// GPU.
#[test]
fn a_frame_grab_hands_back_the_full_size_picture() {
    use bettercut_editor_core::foundation::TimelineTime;
    use bettercut_export::FrameGrabJob;

    let gpu = pollster::block_on(
        bettercut_renderer::wgpu::Instance::new(
            bettercut_renderer::wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
        )
        .request_adapter(&bettercut_renderer::wgpu::RequestAdapterOptions::default()),
    )
    .is_ok();
    if !gpu {
        eprintln!("no GPU adapter; skipping");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let mut manager = MediaJobs::new(store(&dir), PerformanceMode::Balanced, 1, 1);
    let (mut editor, _events) = Editor::new_project("Grab");
    editor.add_text("COPY ME").unwrap();
    let sequence = editor.active_sequence().unwrap();
    let (id, size) = (sequence.id, sequence.resolution);
    manager.submit_frame_grab(FrameGrabJob::new(
        editor.project().clone(),
        id,
        TimelineTime::from_seconds(1),
    ));

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut copied = None;
    while Instant::now() < deadline && copied.is_none() {
        let update = manager.poll();
        assert!(update.failures.is_empty(), "{:?}", update.failures);
        copied = update.copied_frame;
        std::thread::sleep(Duration::from_millis(20));
    }
    let (width, height, rgba) = copied.expect("no frame came back");
    assert_eq!((width, height), (size.width, size.height));
    assert_eq!(rgba.len(), (width * height * 4) as usize);
    assert!(
        rgba.chunks(4).any(|p| p[0] > 128),
        "the title was not drawn"
    );
}
