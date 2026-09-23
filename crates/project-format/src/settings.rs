//! Project settings — the §43 performance modes and the knobs they set.

use bettercut_media::ProxyResolution;
use serde::{Deserialize, Serialize};

/// §43. The mode a user picks; the individual limits derive from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceMode {
    /// 540p proxies, quarter preview, small cache, aggressive throttling.
    Performance,
    #[default]
    Balanced,
    /// Higher preview resolution, larger cache, less aggressive proxying.
    Quality,
}

impl PerformanceMode {
    pub fn proxy_resolution(self) -> ProxyResolution {
        match self {
            Self::Performance => ProxyResolution::P540,
            Self::Balanced | Self::Quality => ProxyResolution::P720,
        }
    }

    /// Frame cache budget in bytes (§18).
    ///
    /// ```text
    /// Default             256 MB
    /// Low-memory mode     128 MB
    /// High-memory systems 512 MB+
    /// ```
    pub fn frame_cache_bytes(self) -> usize {
        match self {
            Self::Performance => 128 * 1024 * 1024,
            Self::Balanced => 256 * 1024 * 1024,
            Self::Quality => 512 * 1024 * 1024,
        }
    }

    /// Heavy background jobs allowed at once (§15).
    ///
    /// ```text
    /// cores <= 4  → 1 heavy job
    /// cores >= 8  → 2-4 heavy jobs
    /// ```
    pub fn max_heavy_jobs(self, cpu_cores: usize) -> usize {
        let by_cores = if cpu_cores <= 4 {
            1
        } else {
            (cpu_cores / 4).min(4)
        };
        match self {
            Self::Performance => 1,
            Self::Balanced => by_cores,
            Self::Quality => by_cores.max(1),
        }
    }

    /// FFmpeg's internal thread cap **per background job** (§15.1).
    ///
    /// FFmpeg defaults to using every core. One uncapped proxy job saturates the
    /// machine no matter how few jobs the scheduler runs.
    pub fn ffmpeg_threads_per_job(self, cpu_cores: usize) -> u32 {
        if cpu_cores <= 4 { 1 } else { 2 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    #[serde(default)]
    pub performance_mode: PerformanceMode,

    /// §13: "Allow the user to disable automatic proxies."
    #[serde(default = "default_true")]
    pub auto_generate_proxies: bool,

    /// §67: configurable cache ceiling, 5/10/20 GB.
    #[serde(default = "default_cache_limit")]
    pub cache_limit_bytes: u64,

    /// §38: debounce for full snapshots. Not the journal interval.
    #[serde(default = "default_autosave_secs")]
    pub autosave_interval_secs: u32,

    /// §10: snapping on the timeline.
    #[serde(default = "default_true")]
    pub snapping_enabled: bool,

    /// How long a photo — or a colour clip — runs when it is put on the
    /// timeline. Defaulted, so older projects keep five seconds.
    #[serde(default = "default_photo_length")]
    pub photo_length: bettercut_foundation::MediaTime,

    /// A magnetic main track: clips on the first picture lane stay packed
    /// from the start, so a delete or a move closes up behind it. Off unless
    /// chosen.
    #[serde(default)]
    pub magnetic_timeline: bool,

    /// How long a transition is when one is added: the project's own
    /// house style, half a second by default. Defaulted for older projects.
    #[serde(default = "default_transition_length")]
    pub transition_length: bettercut_foundation::TimelineTime,
}

/// The shortest and longest a default transition may be set to.
pub const MIN_TRANSITION_LENGTH: bettercut_foundation::TimelineTime =
    bettercut_foundation::TimelineTime::from_millis(100);
pub const MAX_TRANSITION_LENGTH: bettercut_foundation::TimelineTime =
    bettercut_foundation::TimelineTime::from_seconds(5);

fn default_transition_length() -> bettercut_foundation::TimelineTime {
    bettercut_foundation::TimelineTime::from_millis(500)
}

/// The shortest and longest a photo may be set to run when placed: shorter
/// than half a second is a flash, longer than a minute is a still that should
/// be a video.
pub const MIN_PHOTO_LENGTH: bettercut_foundation::MediaTime =
    bettercut_foundation::MediaTime::from_millis(500);
pub const MAX_PHOTO_LENGTH: bettercut_foundation::MediaTime =
    bettercut_foundation::MediaTime::from_seconds(60);

fn default_photo_length() -> bettercut_foundation::MediaTime {
    bettercut_media::STILL_DURATION
}

fn default_true() -> bool {
    true
}

fn default_cache_limit() -> u64 {
    10 * 1024 * 1024 * 1024
}

fn default_autosave_secs() -> u32 {
    60
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            performance_mode: PerformanceMode::default(),
            auto_generate_proxies: true,
            cache_limit_bytes: default_cache_limit(),
            autosave_interval_secs: default_autosave_secs(),
            snapping_enabled: true,
            photo_length: default_photo_length(),
            magnetic_timeline: false,

            transition_length: default_transition_length(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_mode_uses_540p_proxies() {
        assert_eq!(
            PerformanceMode::Performance.proxy_resolution(),
            ProxyResolution::P540
        );
        assert_eq!(
            PerformanceMode::Balanced.proxy_resolution(),
            ProxyResolution::P720
        );
    }

    /// §15's rule, asserted: a 4-core machine runs one heavy job, never more.
    #[test]
    fn four_core_machines_run_one_heavy_job() {
        for mode in [
            PerformanceMode::Performance,
            PerformanceMode::Balanced,
            PerformanceMode::Quality,
        ] {
            assert_eq!(mode.max_heavy_jobs(4), 1, "{mode:?} oversubscribes 4 cores");
            assert_eq!(mode.max_heavy_jobs(2), 1);
        }
        assert!(PerformanceMode::Balanced.max_heavy_jobs(8) >= 2);
        assert!(PerformanceMode::Balanced.max_heavy_jobs(32) <= 4);
    }

    /// §15.1: FFmpeg must never be allowed all cores in a background job (§74).
    #[test]
    fn ffmpeg_threads_are_always_capped() {
        for cores in [1usize, 2, 4, 8, 16, 64] {
            let threads = PerformanceMode::Balanced.ffmpeg_threads_per_job(cores);
            assert!(threads <= 2, "{cores} cores gave {threads} threads");
            assert!(threads >= 1);
        }
        assert_eq!(PerformanceMode::Balanced.ffmpeg_threads_per_job(4), 1);
        assert_eq!(PerformanceMode::Balanced.ffmpeg_threads_per_job(8), 2);
    }

    #[test]
    fn defaults_match_the_guide() {
        let s = ProjectSettings::default();
        assert_eq!(s.performance_mode, PerformanceMode::Balanced);
        assert_eq!(
            PerformanceMode::Balanced.frame_cache_bytes(),
            256 * 1024 * 1024
        );
        assert_eq!(s.cache_limit_bytes, 10 * 1024 * 1024 * 1024);
        assert!(s.auto_generate_proxies);
    }

    /// Old project files predate these fields; loading one must not fail.
    #[test]
    fn settings_deserialize_from_an_empty_object() {
        let s: ProjectSettings = serde_json::from_str("{}").expect("all fields default");
        assert_eq!(s, ProjectSettings::default());
    }
}
