//! Background work derived from imported media (§13, §14, §19, §67).
//!
//! Ties together the pieces that exist separately: the cache knows what has
//! been generated, the scheduler runs the work, and the playback engine reads
//! whichever copy is available.
//!
//! ```text
//! import ─┬─ should this have a proxy? (§13) → queue (§15, priority 4)
//!         │     → encode all-intra (§13.1) → invalidate → preview uses it
//!         └─ poster thumbnail (§12, §19) → queue (background priority)
//!               → decode one frame → downscale → cache
//! ```
//!
//! **One scheduler, deliberately.** Proxies and thumbnails share a single job
//! pool because §15's concurrency limit is a budget for the whole machine; two
//! pools each honouring the limit would oversubscribe exactly the hardware the
//! limit exists to protect. Thumbnails are not `is_heavy()`, so they run
//! alongside the one heavy proxy encode rather than queueing behind it.

use std::collections::HashMap;
use std::sync::Arc;

use bettercut_cache::CacheStore;
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaId;
use bettercut_editor_core::media::ProxyResolution;
use bettercut_editor_core::project_format::PerformanceMode;
use bettercut_export::{ExportJob, Outcome as ExportOutcome};
use bettercut_jobs::{JobEvent, JobId, JobScheduler};
use bettercut_playback::{FilmstripJob, ProxyJob, ProxySource, ThumbnailJob, WaveformJob};

/// What finished this frame, for the caller to act on.
#[derive(Debug, Default)]
pub struct MediaUpdate {
    /// Proxies that became usable; their decoders must be reopened.
    pub ready: Vec<MediaId>,
    /// Thumbnails that became readable; any cached "not there" must be dropped.
    pub thumbnails: Vec<MediaId>,
    /// Waveforms that became readable.
    pub waveforms: Vec<MediaId>,
    /// Messages worth showing.
    pub messages: Vec<String>,
    pub failures: Vec<String>,
}

impl MediaUpdate {
    pub fn is_empty(&self) -> bool {
        self.ready.is_empty()
            && self.thumbnails.is_empty()
            && self.waveforms.is_empty()
            && self.messages.is_empty()
            && self.failures.is_empty()
    }
}

/// Width of the poster thumbnails the media browser shows.
///
/// Fixed rather than derived from the panel: the cache is keyed by width, so a
/// resizable value would regenerate every thumbnail whenever the user dragged a
/// splitter.
pub const THUMBNAIL_WIDTH: u32 = 160;

pub struct MediaJobs {
    scheduler: JobScheduler,
    events: std::sync::mpsc::Receiver<JobEvent>,
    cache: Arc<CacheStore>,
    resolution: ProxyResolution,
    /// FFmpeg threads per encode (§15.1).
    threads: u32,

    /// Proxy jobs in flight, so a completion can be traced back to its asset.
    in_flight: HashMap<JobId, MediaId>,
    /// Thumbnail jobs, kept separate so progress reporting counts only the
    /// slow work — a thumbnail finishes in milliseconds and would make the
    /// proxy progress bar jump around.
    thumbnails: HashMap<JobId, MediaId>,
    /// Waveform analysis, tracked separately for the same reason.
    waveforms: HashMap<JobId, MediaId>,
    /// Latest progress per job, for the status bar (§42).
    progress: HashMap<JobId, f32>,
    /// Exports in flight, with the slot their outcome lands in. Tracked apart
    /// from proxies because an export is something the user asked for and is
    /// waiting on: its progress and its result both need reporting, where a
    /// proxy's are background noise.
    exports: HashMap<JobId, Arc<std::sync::Mutex<Option<ExportOutcome>>>>,
    /// Assets already considered, so re-importing does not requeue.
    considered: std::collections::HashSet<MediaId>,
    /// True once §67's limit has been reported, so the warning appears once
    /// rather than on every import.
    warned_about_space: bool,
}

impl MediaJobs {
    pub fn new(cache: CacheStore, mode: PerformanceMode, max_heavy: usize, threads: u32) -> Self {
        let (scheduler, events) = JobScheduler::new(max_heavy);
        Self {
            scheduler,
            events,
            cache: Arc::new(cache),
            resolution: mode.proxy_resolution(),
            threads,
            in_flight: HashMap::new(),
            thumbnails: HashMap::new(),
            waveforms: HashMap::new(),
            progress: HashMap::new(),
            exports: HashMap::new(),
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

    /// The running export, if any, and how far along it is (§42).
    ///
    /// Reported apart from proxy progress because it is the one piece of
    /// background work the user is actually waiting on — it gets its own bar
    /// and its own stop button rather than being averaged into a count.
    pub fn export_progress(&self) -> Option<(JobId, f32)> {
        let id = *self.exports.keys().next()?;
        Some((id, self.progress.get(&id).copied().unwrap_or(0.0)))
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

        // §13: "Allow the user to disable automatic proxies." Checked per asset
        // rather than as an early return, because thumbnails are a separate
        // decision and turning proxies off should not also blank the browser.
        let proxies_enabled = editor.project().settings.auto_generate_proxies;

        for asset in &editor.project().media {
            if !self.considered.insert(asset.id) {
                continue;
            }

            if let Some(job) = ThumbnailJob::new(asset, THUMBNAIL_WIDTH, &self.cache, 1) {
                let media = job.media();
                let id = self.scheduler.submit(Box::new(job));
                self.thumbnails.insert(id, media);
            }

            if let Some(job) = WaveformJob::new(asset, &self.cache, 1) {
                let media = job.media();
                let id = self.scheduler.submit(Box::new(job));
                self.waveforms.insert(id, media);
            }

            if let Some(job) = FilmstripJob::new(asset, &self.cache, 1) {
                let media = job.media();
                let id = self.scheduler.submit(Box::new(job));
                // Filmstrips share the thumbnail bookkeeping: both are pictures
                // the browser and timeline reload the same way.
                self.thumbnails.insert(id, media);
            }

            if !proxies_enabled || asset.missing || !asset.should_generate_proxy() {
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

    /// Queue an export (§15, §74 — never block the UI while FFmpeg runs).
    ///
    /// Returns the job id so a cancel button has something to name.
    pub fn submit_export(&mut self, job: ExportJob) -> JobId {
        let outcome = job.outcome();
        let id = self.scheduler.submit(Box::new(job));
        self.exports.insert(id, outcome);
        id
    }

    /// Whether an export is running, so the interface can offer to stop it
    /// rather than start a second one.
    pub fn export_in_flight(&self) -> Option<JobId> {
        self.exports.keys().copied().next()
    }

    pub fn cancel(&self, id: JobId) {
        self.scheduler.cancel(id);
    }

    /// Drain job events. Called once per UI frame.
    pub fn poll(&mut self) -> MediaUpdate {
        let mut update = MediaUpdate::default();

        while let Ok(event) = self.events.try_recv() {
            match event {
                JobEvent::Progress { id, fraction } => {
                    // Only proxy progress feeds the status bar; a thumbnail
                    // finishes too fast to be worth showing.
                    if self.in_flight.contains_key(&id) || self.exports.contains_key(&id) {
                        self.progress.insert(id, fraction);
                    }
                }
                JobEvent::Finished { id } => {
                    self.progress.remove(&id);
                    if let Some(slot) = self.exports.remove(&id) {
                        update.messages.push(describe_export(&slot));
                        continue;
                    }
                    if let Some(media) = self.thumbnails.remove(&id) {
                        update.thumbnails.push(media);
                        continue;
                    }
                    if let Some(media) = self.waveforms.remove(&id) {
                        update.waveforms.push(media);
                        continue;
                    }
                    if let Some(media) = self.in_flight.remove(&id) {
                        update.ready.push(media);
                    }
                    if self.in_flight.is_empty() {
                        update.messages.push("Proxies ready".to_owned());
                    }
                }
                JobEvent::Failed { id, message } => {
                    self.progress.remove(&id);
                    if self.exports.remove(&id).is_some() {
                        // Unlike a proxy, an export failing means the user did
                        // not get the thing they asked for. It is an error.
                        update.failures.push(format!("Export failed: {message}"));
                        continue;
                    }
                    if self.waveforms.remove(&id).is_some() {
                        tracing::warn!(%message, "waveform analysis failed");
                        continue;
                    }
                    if self.thumbnails.remove(&id).is_some() {
                        // A missing thumbnail costs the user a picture in the
                        // browser, not the ability to edit. §74's "never
                        // silently ignore" is satisfied by the log; putting it
                        // in the status bar would bury real failures.
                        tracing::warn!(%message, "thumbnail generation failed");
                        continue;
                    }
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
                    if self.exports.remove(&id).is_some() {
                        update.messages.push("Export cancelled".to_owned());
                    }
                    self.in_flight.remove(&id);
                    self.thumbnails.remove(&id);
                    self.waveforms.remove(&id);
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

/// Turn a finished export's outcome into something to show the user.
///
/// Names the encoder, because "why was that fast" and "why was that slow" have
/// the same answer and §41 says the interface should explain itself.
fn describe_export(slot: &Arc<std::sync::Mutex<Option<ExportOutcome>>>) -> String {
    match slot.lock().ok().and_then(|guard| guard.clone()) {
        Some(ExportOutcome::Finished {
            path,
            frames,
            encoder,
            hardware,
        }) => {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let how = if hardware { "hardware" } else { "software" };
            format!("Exported {name} — {frames} frames, {encoder} ({how})")
        }
        Some(ExportOutcome::Cancelled) => "Export cancelled".to_owned(),
        Some(ExportOutcome::Failed(message)) => format!("Export failed: {message}"),
        // The job reported success without writing an outcome, which would be a
        // bug here rather than something the user did.
        None => "Export finished".to_owned(),
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
