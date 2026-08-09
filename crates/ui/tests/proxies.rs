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
use bettercut_ui::{ProxyManager, ProxyUpdate};

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
fn drain(manager: &mut ProxyManager, timeout: Duration) -> ProxyUpdate {
    let deadline = Instant::now() + timeout;
    let mut all = ProxyUpdate::default();

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

    let mut manager = ProxyManager::new(store(&dir), PerformanceMode::Balanced, 1, 1);
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

    let mut manager = ProxyManager::new(store(&dir), PerformanceMode::Balanced, 1, 1);
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

    let mut manager = ProxyManager::new(store(&dir), PerformanceMode::Balanced, 1, 1);
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

    let mut manager = ProxyManager::new(store(&dir), PerformanceMode::Balanced, 1, 1);
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

    let mut manager = ProxyManager::new(store(&dir), PerformanceMode::Performance, 1, 1);
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
    let mut manager = ProxyManager::new(store(&dir), mode, 1, 1);
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

    let mut manager = ProxyManager::new(store(&dir), PerformanceMode::Balanced, 1, 1);
    manager.scan(&editor);
    assert_eq!(manager.active_jobs(), 1);

    let update = drain(&mut manager, Duration::from_secs(30));
    assert!(
        !update.failures.is_empty(),
        "a missing source file failed silently"
    );
    assert!(update.ready.is_empty(), "a failed encode reported success");
}
