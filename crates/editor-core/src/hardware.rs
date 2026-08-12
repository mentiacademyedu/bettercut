//! Hardware detection, so the editor configures itself for the machine it is
//! actually on (§44).
//!
//! §44 requires the editor to *automatically* limit background workers, cap
//! FFmpeg threads, and shrink caches on low-end devices. A setting the user has
//! to find and change does not satisfy that — the machines that need it most
//! belong to the users least likely to go looking.
//!
//! The reference machine (§52.1) is a 4-core / 8-thread laptop, and getting its
//! tier right is the whole point of this module.

use bettercut_project_format::PerformanceMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareProfile {
    /// Logical processors, as the OS reports them.
    pub logical_processors: usize,
}

impl HardwareProfile {
    /// Detect once per process and reuse the answer.
    ///
    /// The hardware does not change while the editor is running, and both
    /// `new_project` and `from_project` ask for it — which logged the profile
    /// twice at startup and made the log read as though something had been
    /// re-detected. Memoizing also means callers can treat this as free.
    pub fn detect() -> Self {
        static PROFILE: std::sync::OnceLock<HardwareProfile> = std::sync::OnceLock::new();

        *PROFILE.get_or_init(|| {
            let logical_processors = std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                // A machine that cannot report its own parallelism is assumed
                // to be small. Guessing high here would oversubscribe it.
                .unwrap_or(2);

            let profile = Self { logical_processors };

            let mode = profile.recommended_mode();
            tracing::info!(
                logical = profile.logical_processors,
                estimated_cores = profile.estimated_physical_cores(),
                recommends = ?mode,
                heavy_jobs = profile.max_heavy_jobs(mode),
                ffmpeg_threads = profile.ffmpeg_threads_per_job(mode),
                "detected hardware profile"
            );

            profile
        })
    }

    /// Estimated *physical* cores.
    ///
    /// §15's thresholds ("cores <= 4", "cores >= 8") are about real throughput,
    /// not logical processors. The reference machine is an i5-8250U: 4 cores,
    /// 8 threads. Feeding its 8 logical processors straight into §15 would put
    /// it in the "2–4 heavy jobs" tier and let proxy generation starve playback
    /// on exactly the machine the rule exists to protect (§69).
    ///
    /// Rust's standard library cannot report physical cores, so this assumes
    /// SMT on any machine reporting 8 or more logical processors. That
    /// under-counts a true 8-core CPU without SMT, which is the safe direction:
    /// §80 puts playback above background work, and §88 says the editor must
    /// feel fast before it feels powerful.
    pub fn estimated_physical_cores(&self) -> usize {
        if self.logical_processors >= 8 {
            self.logical_processors / 2
        } else {
            self.logical_processors
        }
        .max(1)
    }

    /// The §43 mode this machine should start in.
    ///
    /// A 4-core machine gets Performance mode by default — 540p proxies,
    /// smaller cache, aggressive throttling (§44).
    pub fn recommended_mode(&self) -> PerformanceMode {
        match self.estimated_physical_cores() {
            0..=4 => PerformanceMode::Performance,
            5..=7 => PerformanceMode::Balanced,
            _ => PerformanceMode::Quality,
        }
    }

    /// Heavy background jobs allowed at once (§15).
    ///
    /// Takes the mode rather than reading `recommended_mode()`, because these
    /// three limits are what §43's mode *is*. They used to derive from the
    /// recommendation, which meant changing the mode moved the proxy
    /// resolution and nothing else — and the diagnostics panel cheerfully
    /// showed `mode Quality` beside numbers computed from Balanced.
    ///
    /// Requiring the argument makes that mistake impossible to repeat: there
    /// is no longer a version of this that silently ignores the user.
    pub fn max_heavy_jobs(&self, mode: PerformanceMode) -> usize {
        mode.max_heavy_jobs(self.estimated_physical_cores())
    }

    /// FFmpeg's internal thread cap **per background job** (§15.1).
    pub fn ffmpeg_threads_per_job(&self, mode: PerformanceMode) -> u32 {
        mode.ffmpeg_threads_per_job(self.estimated_physical_cores())
    }

    /// Frame cache budget in bytes (§18).
    pub fn frame_cache_bytes(&self, mode: PerformanceMode) -> usize {
        mode.frame_cache_bytes()
    }

    /// One-line summary for diagnostics and the settings UI.
    ///
    /// Describes the machine and what it *would* choose, not what the project
    /// is set to — the caller knows that and the profile does not.
    pub fn summary(&self) -> String {
        let mode = self.recommended_mode();
        format!(
            "{} logical processors (~{} cores) · recommends {:?} mode · {} heavy job(s) · {} FFmpeg thread(s)/job",
            self.logical_processors,
            self.estimated_physical_cores(),
            mode,
            self.max_heavy_jobs(mode),
            self.ffmpeg_threads_per_job(mode),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(logical: usize) -> HardwareProfile {
        HardwareProfile {
            logical_processors: logical,
        }
    }

    /// The case this module exists for. i5-8250U: 4 cores, 8 threads.
    /// If this ever reports more than one heavy job, proxy generation can
    /// starve playback on the reference machine (§15.1, §69).
    #[test]
    fn the_reference_machine_lands_in_the_single_job_tier() {
        let reference = profile(8); // 4C/8T
        assert_eq!(reference.estimated_physical_cores(), 4);
        let mode = reference.recommended_mode();
        assert_eq!(mode, PerformanceMode::Performance);
        assert_eq!(reference.max_heavy_jobs(mode), 1);
        assert_eq!(reference.ffmpeg_threads_per_job(mode), 1);
    }

    /// Every mode, not just the recommended one.
    ///
    /// The user can select any of them, so an invariant that only held for the
    /// machine's own suggestion would not be an invariant at all.
    const ALL_MODES: [PerformanceMode; 3] = [
        PerformanceMode::Performance,
        PerformanceMode::Balanced,
        PerformanceMode::Quality,
    ];

    /// §74: "Let FFmpeg use all CPU cores in a background job" is prohibited.
    #[test]
    fn ffmpeg_is_never_given_more_than_two_threads() {
        for logical in 1..=128 {
            for mode in ALL_MODES {
                let threads = profile(logical).ffmpeg_threads_per_job(mode);
                assert!(
                    (1..=2).contains(&threads),
                    "{logical} logical processors in {mode:?} gave {threads} FFmpeg threads"
                );
            }
        }
    }

    #[test]
    fn heavy_jobs_never_exceed_four_and_never_reach_zero() {
        for logical in 1..=128 {
            for mode in ALL_MODES {
                let jobs = profile(logical).max_heavy_jobs(mode);
                assert!(
                    (1..=4).contains(&jobs),
                    "{logical} logical processors in {mode:?} gave {jobs} heavy jobs"
                );
            }
        }
    }

    /// The point of the fix: the limits follow the *chosen* mode. If they did
    /// not, changing the mode would move the proxy resolution and nothing else.
    #[test]
    fn the_limits_follow_the_mode_not_the_recommendation() {
        // A large machine, whose recommendation is Quality.
        let big = profile(32);
        assert_eq!(big.recommended_mode(), PerformanceMode::Quality);

        assert!(
            big.frame_cache_bytes(PerformanceMode::Performance)
                < big.frame_cache_bytes(PerformanceMode::Quality),
            "asking for Performance did not shrink the frame cache"
        );
        assert!(
            big.max_heavy_jobs(PerformanceMode::Performance)
                <= big.max_heavy_jobs(PerformanceMode::Quality),
            "asking for Performance did not reduce background work"
        );
    }

    #[test]
    fn small_machines_get_performance_mode() {
        for logical in [1, 2, 4, 8] {
            assert_eq!(
                profile(logical).recommended_mode(),
                PerformanceMode::Performance,
                "{logical} logical processors"
            );
        }
    }

    #[test]
    fn large_machines_get_quality_mode() {
        assert_eq!(profile(16).recommended_mode(), PerformanceMode::Quality);
        assert_eq!(profile(32).recommended_mode(), PerformanceMode::Quality);
    }

    /// A dual-core with no SMT must not be halved to one.
    #[test]
    fn small_core_counts_are_not_halved() {
        assert_eq!(profile(2).estimated_physical_cores(), 2);
        assert_eq!(profile(4).estimated_physical_cores(), 4);
        assert_eq!(profile(1).estimated_physical_cores(), 1);
    }

    #[test]
    fn detection_returns_something_usable_on_this_machine() {
        let detected = HardwareProfile::detect();
        assert!(detected.logical_processors >= 1);
        assert!(detected.max_heavy_jobs(detected.recommended_mode()) >= 1);
        assert!(!detected.summary().is_empty());
    }
}
