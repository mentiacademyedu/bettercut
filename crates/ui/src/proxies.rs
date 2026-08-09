//! Proxy lifecycle (§13, §14, §67).
//!
//! Ties together the three pieces that already exist separately: the cache
//! knows what has been generated, the scheduler runs the encoding, and the
//! playback engine reads whichever copy is available.
//!
//! ```text
//! import -> should this have a proxy? (§13) -> queue (§15, priority 4)
//!        -> encode all-intra (§13.1) -> invalidate -> preview uses it (§14)
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use bettercut_cache::CacheStore;
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaId;
use bettercut_editor_core::media::ProxyResolution;
use bettercut_editor_core::project_format::PerformanceMode;
use bettercut_jobs::{JobEvent, JobId, JobScheduler};
use bettercut_playback::{ProxyJob, ProxySource};

/// What happened to proxies this frame, for the caller to act on.
#[derive(Debug, Default)]
pub struct ProxyUpdate {
    /// Proxies that became usable; their decoders must be reopened.
    pub ready: Vec<MediaId>,
    /// Messages worth showing.
    pub messages: Vec<String>,
    pub failures: Vec<String>,
}

impl ProxyUpdate {
    pub fn is_empty(&self) -> bool {
        self.ready.is_empty() && self.messages.is_empty() && self.failures.is_empty()
    }
}

pub struct ProxyManager {
    scheduler: JobScheduler,
    events: std::sync::mpsc::Receiver<JobEvent>,
    cache: Arc<CacheStore>,
    resolution: ProxyResolution,
    /// FFmpeg threads per encode (§15.1).
    threads: u32,

    /// Jobs in flight, so a completion can be traced back to its asset.
    in_flight: HashMap<JobId, MediaId>,
    /// Latest progress per job, for the status bar (§42).
    progress: HashMap<JobId, f32>,
    /// Assets already considered, so re-importing does not requeue.
    considered: std::collections::HashSet<MediaId>,
    /// True once §67's limit has been reported, so the warning appears once
    /// rather than on every import.
    warned_about_space: bool,
}

impl ProxyManager {
    pub fn new(cache: CacheStore, mode: PerformanceMode, max_heavy: usize, threads: u32) -> Self {
        let (scheduler, events) = JobScheduler::new(max_heavy);
        Self {
            scheduler,
            events,
            cache: Arc::new(cache),
            resolution: mode.proxy_resolution(),
            threads,
            in_flight: HashMap::new(),
            progress: HashMap::new(),
            considered: std::collections::HashSet::new(),
            warned_about_space: false,
        }
    }

    /// What the playback engine should read from.
    pub fn source(&self) -> ProxySource {
        ProxySource {
            cache: Arc::clone(&self.cache),
            height: self.resolution.height(),
        }
    }

    pub fn active_jobs(&self) -> usize {
        self.in_flight.len()
    }

    /// Overall progress across queued encodes, for a single status-bar figure.
    pub fn overall_progress(&self) -> Option<f32> {
        if self.in_flight.is_empty() {
            return None;
        }
        let total: f32 = self
            .in_flight
            .keys()
            .map(|id| self.progress.get(id).copied().unwrap_or(0.0))
            .sum();
        Some(total / self.in_flight.len() as f32)
    }

    /// Reconcile with the project's settings, then queue any missing proxies.
    ///
    /// Called every frame. Cheap when nothing changed — a hash lookup per asset
    /// — and it means the manager has one source of truth (the project) rather
    /// than a copy of the settings that can drift from it.
    ///
    /// Returns a new [`ProxySource`] when the resolution changed, which the
    /// caller must hand to the preview: proxies are stored per height, so after
    /// a change the old files are simply not looked at.
    pub fn sync(&mut self, editor: &Editor) -> (Option<ProxySource>, Vec<String>) {
        let wanted = editor
            .project()
            .settings
            .performance_mode
            .proxy_resolution();
        if wanted == self.resolution {
            return (None, self.scan(editor));
        }

        // Changing quality invalidates everything in flight: those encodes are
        // writing the height the user just moved away from.
        self.scheduler.cancel_all();
        self.in_flight.clear();
        self.progress.clear();
        self.considered.clear();
        self.resolution = wanted;

        tracing::info!(height = wanted.height(), "proxy resolution changed");
        let mut messages = vec![format!("Proxy quality changed to {}p", wanted.height())];
        messages.extend(self.scan(editor));
        (Some(self.source()), messages)
    }

    /// Queue proxies for anything in the project that wants one (§13).
    ///
    /// Idempotent: assets already considered are skipped, and `ProxyJob::new`
    /// declines when the cache already holds the file.
    pub fn scan(&mut self, editor: &Editor) -> Vec<String> {
        let mut messages = Vec::new();
        let mut queued = 0;

        if !editor.project().settings.auto_generate_proxies {
            // §13: "Allow the user to disable automatic proxies."
            return messages;
        }

        for asset in &editor.project().media {
            if !self.considered.insert(asset.id) {
                continue;
            }
            if asset.missing || !asset.should_generate_proxy() {
                continue;
            }

            // §67: warn before a proxy run would blow the cache limit, because
            // all-intra files are 3-5x the size of a long-GOP equivalent.
            let estimate = estimate_proxy_bytes(asset, self.resolution);
            if self.cache.would_exceed_limit(estimate) && !self.warned_about_space {
                self.warned_about_space = true;
                messages.push(format!(
                    "Proxy cache is near its {} GB limit; older proxies will be removed",
                    self.cache.limit_bytes() / (1024 * 1024 * 1024)
                ));
                // Make room rather than refusing: the media in this project is
                // what the user is working on, so it wins over older entries.
                let keep: Vec<MediaId> = editor.project().media.iter().map(|m| m.id).collect();
                if let Err(err) = self.cache.evict_to_fit(estimate, &keep) {
                    tracing::warn!(%err, "could not reclaim cache space");
                }
            }

            let Some(job) = ProxyJob::new(asset, self.resolution, &self.cache, self.threads) else {
                continue; // already cached
            };

            let media = job.media();
            let id = self.scheduler.submit(Box::new(job));
            self.in_flight.insert(id, media);
            queued += 1;
            tracing::info!(
                file = %asset.file_name,
                height = self.resolution.height(),
                "queued a proxy"
            );
        }

        // Only for what *this* call started. `sync` runs every frame, so
        // reporting the whole in-flight set would repeat the message forever;
        // the status bar's progress indicator is what shows ongoing work.
        if queued > 0 {
            messages.push(format!("Generating {queued} proxy file(s)"));
        }
        messages
    }

    /// Drain job events. Called once per UI frame.
    pub fn poll(&mut self) -> ProxyUpdate {
        let mut update = ProxyUpdate::default();

        while let Ok(event) = self.events.try_recv() {
            match event {
                JobEvent::Progress { id, fraction } => {
                    self.progress.insert(id, fraction);
                }
                JobEvent::Finished { id } => {
                    self.progress.remove(&id);
                    if let Some(media) = self.in_flight.remove(&id) {
                        update.ready.push(media);
                    }
                    if self.in_flight.is_empty() {
                        update.messages.push("Proxies ready".to_owned());
                    }
                }
                JobEvent::Failed { id, message } => {
                    self.progress.remove(&id);
                    self.in_flight.remove(&id);
                    // §74 forbids silently ignoring an FFmpeg failure. Playback
                    // still works from the original, so this is a warning
                    // rather than an error the user must act on.
                    update
                        .failures
                        .push(format!("Proxy generation failed: {message}"));
                }
                JobEvent::Cancelled { id } => {
                    self.progress.remove(&id);
                    self.in_flight.remove(&id);
                }
                JobEvent::Started { .. } => {}
            }
        }

        update
    }

    /// Stop everything in flight (§48). Called when closing.
    pub fn cancel_all(&self) {
        self.scheduler.cancel_all();
    }
}

/// Rough size of a proxy, for §67's "will this fit?" question.
///
/// All-intra H.264 at the bitrate `encode.rs` requests, which is
/// `width * height * 4` bits per second.
fn estimate_proxy_bytes(
    asset: &bettercut_editor_core::media::MediaAsset,
    resolution: ProxyResolution,
) -> u64 {
    let height = u64::from(resolution.height());
    let aspect = if asset.height > 0 {
        f64::from(asset.width) / f64::from(asset.height)
    } else {
        16.0 / 9.0
    };
    let width = (height as f64 * aspect) as u64;
    let bits_per_second = width * height * 4;
    let seconds = (asset.duration.as_seconds_f64().max(0.0)) as u64;

    // Plus AAC audio, plus container overhead.
    (bits_per_second / 8) * seconds + 16_000 * seconds
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_editor_core::foundation::MediaTime;
    use bettercut_editor_core::media::{MediaAsset, MediaKind};

    fn asset(seconds: i64, width: u32, height: u32) -> MediaAsset {
        MediaAsset::new(
            MediaKind::Video,
            "C:/media/clip.mp4",
            MediaTime::from_seconds(seconds),
        )
        .with_video(
            width,
            height,
            bettercut_editor_core::foundation::FrameRate::FPS_30,
        )
    }

    #[test]
    fn the_estimate_grows_with_duration_and_resolution() {
        let ten = estimate_proxy_bytes(&asset(10, 1920, 1080), ProxyResolution::P720);
        let sixty = estimate_proxy_bytes(&asset(60, 1920, 1080), ProxyResolution::P720);
        assert!(
            sixty > ten * 5,
            "a 6x longer clip should estimate far larger"
        );

        let small = estimate_proxy_bytes(&asset(60, 1920, 1080), ProxyResolution::P360);
        assert!(small < sixty, "a smaller proxy should estimate smaller");
    }

    #[test]
    fn a_zero_length_asset_estimates_nothing() {
        assert_eq!(
            estimate_proxy_bytes(&asset(0, 1920, 1080), ProxyResolution::P720),
            0
        );
    }

    /// The estimate is used to decide whether to evict, so being wildly wrong
    /// in either direction matters. A minute of 720p all-intra is tens of MB.
    #[test]
    fn the_estimate_is_in_a_plausible_range() {
        let bytes = estimate_proxy_bytes(&asset(60, 1920, 1080), ProxyResolution::P720);
        let megabytes = bytes / (1024 * 1024);
        assert!(
            (10..=500).contains(&megabytes),
            "a minute of 720p proxy estimated at {megabytes} MB"
        );
    }
}
