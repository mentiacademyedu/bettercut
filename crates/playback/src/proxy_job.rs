//! Proxy generation as a background job (§13, §15).
//!
//! Generation itself lives in the media crate; this is the wrapper that makes
//! it a scheduled, cancellable, progress-reporting unit of work — and that
//! obeys §15.1's thread cap and §67's cache limit.
//!
//! §69 puts proxy generation at priority 4, below export and well below
//! anything the user is waiting on, because §44 is explicit that proxy work
//! must never starve playback.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bettercut_cache::CacheStore;
use bettercut_foundation::MediaId;
use bettercut_jobs::{JobContext, Priority, Task};
use bettercut_media::{MediaAsset, ProxyResolution};

/// Bridges the scheduler's cancellation to the media crate's token.
///
/// Two traits describing the same idea, kept separate because §86 does not let
/// the media crate depend on the job scheduler.
struct JobCancellation {
    cancelled: Arc<AtomicBool>,
}

impl bettercut_media::CancellationToken for JobCancellation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Encode one proxy.
pub struct ProxyJob {
    asset: MediaAsset,
    resolution: ProxyResolution,
    output: PathBuf,
    temp: PathBuf,
    /// §15.1's per-job FFmpeg thread cap, from `HardwareProfile`.
    threads: u32,
}

impl ProxyJob {
    /// Prepare a job, or `None` when the cache already holds this proxy.
    pub fn new(
        asset: &MediaAsset,
        resolution: ProxyResolution,
        cache: &CacheStore,
        threads: u32,
    ) -> Option<Self> {
        let output = cache.layout().proxy_file(asset.id, resolution.height());
        if output.exists() {
            return None;
        }

        Some(Self {
            asset: asset.clone(),
            resolution,
            temp: cache
                .layout()
                .proxy_temp_file(asset.id, resolution.height()),
            output,
            threads,
        })
    }

    pub fn media(&self) -> MediaId {
        self.asset.id
    }

    pub fn output_path(&self) -> &std::path::Path {
        &self.output
    }
}

impl Task for ProxyJob {
    fn label(&self) -> String {
        format!(
            "Generating {}p proxy for {}",
            self.resolution.height(),
            self.asset.file_name
        )
    }

    fn priority(&self) -> Priority {
        Priority::Proxy
    }

    fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
        if let Some(parent) = self.output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        // Encode to a temporary name and rename on success. A half-written
        // proxy that carried the final name would look complete and play as
        // corrupt footage — the same reasoning as §38.1's atomic project write.
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = JobCancellation {
            cancelled: Arc::clone(&cancelled),
        };

        // The media crate's encoder polls its own token; keep it in step with
        // the scheduler's on every progress callback, which fires per frame.
        let progress = |fraction: f32| {
            if ctx.is_cancelled() {
                cancelled.store(true, Ordering::Release);
            }
            ctx.progress(fraction);
        };

        let result = bettercut_media::generate_proxy(
            &self.asset,
            &self.temp,
            self.resolution.height(),
            self.threads,
            &token,
            &progress,
        );

        match result {
            Ok(()) => {
                std::fs::rename(&self.temp, &self.output).map_err(|e| e.to_string())?;
                tracing::info!(
                    file = %self.asset.file_name,
                    height = self.resolution.height(),
                    "proxy ready"
                );
                Ok(())
            }
            Err(err) => {
                // Never leave a partial file behind: the next run would find it
                // and skip regeneration.
                let _ = std::fs::remove_file(&self.temp);
                if err.is_cancellation() {
                    // §50/§48: a cancellation is expected control flow.
                    return Ok(());
                }
                Err(err.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_cache::{CACHE_LIMIT_5_GB, CacheLayout};
    use bettercut_foundation::MediaTime;
    use bettercut_media::{FfmpegProber, MediaProber};

    fn fixture(name: &str) -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../media/tests/fixtures")
            .join(name)
    }

    fn store(dir: &std::path::Path) -> CacheStore {
        CacheStore::new(CacheLayout::new(dir), CACHE_LIMIT_5_GB)
    }

    #[test]
    fn a_job_describes_itself_usefully() {
        let dir = tempfile::tempdir().expect("tempdir");
        let asset = FfmpegProber
            .probe(&fixture("ntsc-2997.mp4"))
            .expect("probe");
        let job =
            ProxyJob::new(&asset, ProxyResolution::P360, &store(dir.path()), 1).expect("a job");

        assert_eq!(job.priority(), Priority::Proxy);
        assert!(job.label().contains("ntsc-2997.mp4"));
        assert!(job.label().contains("360"));
    }

    /// Regenerating an existing proxy is wasted work on the machine least able
    /// to afford it.
    #[test]
    fn an_existing_proxy_produces_no_job() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = store(dir.path());
        let asset = FfmpegProber
            .probe(&fixture("ntsc-2997.mp4"))
            .expect("probe");

        cache.layout().prepare(asset.id).expect("prepare");
        std::fs::write(
            cache
                .layout()
                .proxy_file(asset.id, ProxyResolution::P360.height()),
            b"already here",
        )
        .expect("write");

        assert!(ProxyJob::new(&asset, ProxyResolution::P360, &cache, 1).is_none());
    }

    /// The whole path, through the real scheduler: submit, encode, land in the
    /// cache under the right name.
    #[test]
    fn running_the_job_puts_a_usable_proxy_in_the_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = store(dir.path());
        let asset = FfmpegProber
            .probe(&fixture("ntsc-2997.mp4"))
            .expect("probe");

        let (scheduler, events) = bettercut_jobs::JobScheduler::new(1);
        let job = ProxyJob::new(&asset, ProxyResolution::P360, &cache, 1).expect("a job");
        let output = job.output_path().to_path_buf();
        scheduler.submit(Box::new(job));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut finished = false;
        while std::time::Instant::now() < deadline && !finished {
            match events.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(bettercut_jobs::JobEvent::Finished { .. }) => finished = true,
                Ok(bettercut_jobs::JobEvent::Failed { message, .. }) => {
                    panic!("proxy job failed: {message}")
                }
                _ => {}
            }
        }

        assert!(finished, "the proxy job did not finish");
        assert!(output.exists(), "no proxy landed in the cache");

        // And it is real, decodable media at the requested height.
        let proxy = FfmpegProber.probe(&output).expect("probe the proxy");
        assert_eq!(proxy.height, 360);
        assert!(proxy.duration > MediaTime::ZERO);

        // No partial file left behind.
        assert!(
            !cache.layout().proxy_temp_file(asset.id, 360).exists(),
            "a .partial file survived a successful encode"
        );
    }

    /// §67: the cache must see what was generated, or eviction has nothing to
    /// act on and the limit is unenforceable.
    #[test]
    fn a_generated_proxy_counts_against_the_cache_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = store(dir.path());
        let asset = FfmpegProber
            .probe(&fixture("ntsc-2997.mp4"))
            .expect("probe");

        assert_eq!(cache.total_bytes(), 0);

        let (scheduler, events) = bettercut_jobs::JobScheduler::new(1);
        let job = ProxyJob::new(&asset, ProxyResolution::P360, &cache, 1).expect("a job");
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

        assert!(
            cache.total_bytes() > 0,
            "the cache did not see the generated proxy"
        );
        assert_eq!(cache.entries().len(), 1);
    }
}
