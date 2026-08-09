//! The frame cache (§18).
//!
//! LRU, **byte-bounded, not entry-bounded**. A 4K frame is 33 MB and a 360p
//! frame is 0.5 MB; a cache of "200 frames" is either 100 GB or 100 MB
//! depending on the footage, and only one of those fits on the §52.1 reference
//! machine's 8 GB.
//!
//! ```text
//! Default:              256 MB
//! Low-memory mode:      128 MB
//! High-memory systems:  512 MB+
//! ```
//!
//! §18 also states the priority order — current frame, next frames, recent
//! previous frames — which plain LRU already produces during forward playback:
//! the frames just shown are the most recently used, and the ones ahead are
//! inserted as they are decoded.

use std::collections::HashMap;
use std::sync::Arc;

use bettercut_foundation::{MediaId, MediaTime};
use bettercut_media::VideoFrame;

/// Identifies a decoded frame: which media, and where in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameKey {
    pub media: MediaId,
    pub timestamp: MediaTime,
}

struct Entry {
    frame: Arc<VideoFrame>,
    bytes: usize,
    /// Monotonic counter; the smallest value is the least recently used.
    used_at: u64,
}

pub struct FrameCache {
    entries: HashMap<FrameKey, Entry>,
    bytes: usize,
    max_bytes: usize,
    clock: u64,

    hits: u64,
    misses: u64,
}

impl FrameCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            max_bytes,
            clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Change the budget, evicting immediately if it shrank.
    ///
    /// §43 lets the user switch performance mode mid-session, and a cache that
    /// only honoured its new budget on the next insert would keep holding
    /// memory the user just asked it to release.
    pub fn set_max_bytes(&mut self, max_bytes: usize) {
        self.max_bytes = max_bytes;
        self.evict_to_fit(0);
    }

    /// Returns a shared handle. Frames are Arc-wrapped because a 1080p RGBA
    /// frame is 8 MB: handing one back by value would memcpy that per layer per
    /// displayed frame, which at 30 fps is hundreds of megabytes a second of
    /// pure waste (§68, §73).
    pub fn get(&mut self, key: &FrameKey) -> Option<Arc<VideoFrame>> {
        self.clock += 1;
        let clock = self.clock;

        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.used_at = clock;
                self.hits += 1;
                Some(Arc::clone(&entry.frame))
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    pub fn contains(&self, key: &FrameKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Insert a frame, evicting least-recently-used entries to stay in budget.
    ///
    /// A frame larger than the entire budget is **not** cached: storing it
    /// would evict everything else and then itself, which is strictly worse
    /// than not caching at all.
    pub fn insert(&mut self, key: FrameKey, frame: Arc<VideoFrame>) {
        let bytes = frame_bytes(&frame);
        if bytes > self.max_bytes {
            tracing::debug!(
                bytes,
                max = self.max_bytes,
                "frame is larger than the whole cache budget; not caching it"
            );
            return;
        }

        self.clock += 1;
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes -= previous.bytes;
        }

        self.evict_to_fit(bytes);

        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                frame,
                bytes,
                used_at: self.clock,
            },
        );
    }

    /// Drop everything for one media asset.
    ///
    /// Needed when a proxy becomes ready (§13): the frames cached from the
    /// original are still correct, but the whole point is to start using the
    /// proxy, and keeping both wastes the budget.
    pub fn invalidate_media(&mut self, media: MediaId) {
        self.entries.retain(|key, entry| {
            let keep = key.media != media;
            if !keep {
                self.bytes -= entry.bytes;
            }
            keep
        });
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Evict until `incoming` more bytes will fit.
    fn evict_to_fit(&mut self, incoming: usize) {
        while self.bytes + incoming > self.max_bytes && !self.entries.is_empty() {
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used_at)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.bytes -= entry.bytes;
            }
        }
    }
}

/// How much memory a frame occupies.
fn frame_bytes(frame: &VideoFrame) -> usize {
    match &frame.storage {
        bettercut_media::FrameStorage::System { data, .. } => data.len(),
        // A GPU frame costs no system RAM, but it does cost VRAM, which is
        // scarcer on the integrated graphics §52.1 targets. Charge it the same
        // so the budget still bounds something real.
        bettercut_media::FrameStorage::Gpu { .. } => {
            (frame.width as usize) * (frame.height as usize) * 4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_media::{ColorMetadata, FrameStorage};

    fn frame(width: u32, height: u32) -> VideoFrame {
        VideoFrame {
            timestamp: MediaTime::ZERO,
            width,
            height,
            color: ColorMetadata::default(),
            storage: FrameStorage::System {
                data: vec![0; (width * height * 4) as usize],
                stride: width * 4,
            },
        }
    }

    fn key(media: MediaId, ticks: i64) -> FrameKey {
        FrameKey {
            media,
            timestamp: MediaTime::from_ticks(ticks),
        }
    }

    #[test]
    fn a_stored_frame_can_be_read_back() {
        let mut cache = FrameCache::new(1024 * 1024);
        let media = MediaId::new();

        cache.insert(key(media, 0), Arc::new(frame(16, 16)));
        assert!(cache.get(&key(media, 0)).is_some());
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 0);

        assert!(cache.get(&key(media, 100)).is_none());
        assert_eq!(cache.misses(), 1);
    }

    /// The property that matters: the budget is in **bytes**, so big frames
    /// crowd out more entries than small ones.
    #[test]
    fn the_cache_is_bounded_by_bytes_not_entries() {
        // Room for exactly four 16x16 frames (1024 bytes each).
        let mut cache = FrameCache::new(4 * 16 * 16 * 4);
        let media = MediaId::new();

        for i in 0..4 {
            cache.insert(key(media, i), Arc::new(frame(16, 16)));
        }
        assert_eq!(cache.len(), 4);
        assert!(cache.bytes() <= cache.max_bytes());

        // A fifth pushes the first out.
        cache.insert(key(media, 4), Arc::new(frame(16, 16)));
        assert_eq!(cache.len(), 4);
        assert!(cache.bytes() <= cache.max_bytes());

        // One frame four times the size clears the rest out on its own.
        cache.insert(key(media, 99), Arc::new(frame(32, 32)));
        assert!(cache.bytes() <= cache.max_bytes());
        assert!(cache.contains(&key(media, 99)));
    }

    #[test]
    fn the_least_recently_used_frame_is_evicted_first() {
        let mut cache = FrameCache::new(2 * 16 * 16 * 4);
        let media = MediaId::new();

        cache.insert(key(media, 0), Arc::new(frame(16, 16)));
        cache.insert(key(media, 1), Arc::new(frame(16, 16)));

        // Touch the older one so the newer becomes the eviction candidate.
        assert!(cache.get(&key(media, 0)).is_some());

        cache.insert(key(media, 2), Arc::new(frame(16, 16)));

        assert!(
            cache.contains(&key(media, 0)),
            "recently used frame evicted"
        );
        assert!(!cache.contains(&key(media, 1)), "LRU frame survived");
    }

    /// Caching a frame bigger than the whole budget would evict everything and
    /// then not fit anyway.
    #[test]
    fn an_oversized_frame_is_refused_without_clearing_the_cache() {
        let mut cache = FrameCache::new(1024);
        let media = MediaId::new();
        cache.insert(key(media, 0), Arc::new(frame(16, 16))); // exactly 1024 bytes

        cache.insert(key(media, 1), Arc::new(frame(64, 64))); // 16 KB

        assert!(
            cache.contains(&key(media, 0)),
            "existing frame was thrown away"
        );
        assert!(!cache.contains(&key(media, 1)));
        assert!(cache.bytes() <= cache.max_bytes());
    }

    #[test]
    fn shrinking_the_budget_evicts_immediately() {
        let mut cache = FrameCache::new(8 * 1024);
        let media = MediaId::new();
        for i in 0..8 {
            cache.insert(key(media, i), Arc::new(frame(16, 16)));
        }
        assert_eq!(cache.len(), 8);

        cache.set_max_bytes(2 * 1024);
        assert!(
            cache.bytes() <= 2 * 1024,
            "budget not honoured after shrinking"
        );
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn reinserting_a_key_does_not_double_count_its_bytes() {
        let mut cache = FrameCache::new(1024 * 1024);
        let media = MediaId::new();

        cache.insert(key(media, 0), Arc::new(frame(16, 16)));
        let after_first = cache.bytes();
        cache.insert(key(media, 0), Arc::new(frame(16, 16)));

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.bytes(), after_first, "bytes were counted twice");
    }

    #[test]
    fn invalidating_one_media_leaves_the_others_alone() {
        let mut cache = FrameCache::new(1024 * 1024);
        let (a, b) = (MediaId::new(), MediaId::new());

        cache.insert(key(a, 0), Arc::new(frame(16, 16)));
        cache.insert(key(a, 1), Arc::new(frame(16, 16)));
        cache.insert(key(b, 0), Arc::new(frame(16, 16)));

        cache.invalidate_media(a);

        assert!(!cache.contains(&key(a, 0)));
        assert!(!cache.contains(&key(a, 1)));
        assert!(cache.contains(&key(b, 0)));
        assert_eq!(
            cache.bytes(),
            16 * 16 * 4,
            "byte count drifted after eviction"
        );
    }

    #[test]
    fn frames_from_different_media_at_the_same_time_are_distinct() {
        let mut cache = FrameCache::new(1024 * 1024);
        let (a, b) = (MediaId::new(), MediaId::new());

        cache.insert(key(a, 500), Arc::new(frame(16, 16)));
        assert!(
            cache.get(&key(b, 500)).is_none(),
            "keys collided across media"
        );
    }
}
