//! Job priorities (§69).
//!
//! ```text
//! Priority 0   Audio callback (real-time, never preempted)
//! Priority 1   Playback / decode / render
//! Priority 2   User-triggered operations
//! Priority 3   Export
//! Priority 4   Proxy generation
//! Priority 5   Thumbnail / background analysis
//! ```
//!
//! Levels 0 and 1 are **not** scheduled here. The audio callback belongs to the
//! device and the playback path runs on its own threads; putting either behind
//! a work queue would let a proxy job delay them, which is exactly what §69's
//! ordering and §15.1's starvation rules exist to prevent.
//!
//! So this queue starts at level 2, and every thread it owns runs *below* normal
//! OS priority.

/// What a queued job is for. Lower is more urgent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// The user asked for this and is waiting (§69 level 2).
    UserRequested = 2,
    /// §69 level 3. May be configurable later, as §69 allows.
    Export = 3,
    /// §69 level 4. The heaviest recurring background work (§13.1).
    Proxy = 4,
    /// §69 level 5: thumbnails, waveforms, scene analysis.
    Background = 5,
}

impl Priority {
    /// Is this heavy enough to count against the §15 concurrency cap?
    ///
    /// §15 limits *heavy* jobs — the ones that saturate a core. Thumbnail and
    /// waveform work is cheap and bursty; counting it against the same budget
    /// would leave a 4-core machine unable to draw its own timeline while a
    /// proxy runs.
    pub fn is_heavy(self) -> bool {
        matches!(self, Self::Export | Self::Proxy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §69's ordering, asserted. Reordering these silently changes which work
    /// starves which.
    #[test]
    fn priorities_follow_section_69() {
        assert!(Priority::UserRequested < Priority::Export);
        assert!(Priority::Export < Priority::Proxy);
        assert!(Priority::Proxy < Priority::Background);
    }

    #[test]
    fn only_export_and_proxy_count_as_heavy() {
        assert!(Priority::Export.is_heavy());
        assert!(Priority::Proxy.is_heavy());
        assert!(!Priority::UserRequested.is_heavy());
        assert!(!Priority::Background.is_heavy());
    }
}
