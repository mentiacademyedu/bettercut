//! Cache size management (§67).
//!
//! ```text
//! Configurable maximum size: 5 GB   10 GB   20 GB
//! Contents: Proxies, Thumbnails, Waveforms, Preview renders, Analysis results
//! LRU cleanup. Never delete source media.
//! ```
//!
//! # The rule that must never bend
//!
//! **Never delete source media.** §2 makes editing non-destructive and §74 puts
//! "modify source media" on the prohibited list. Everything this module removes
//! lives under the cache root and can be regenerated; nothing outside it is
//! ever touched. `is_inside_root` enforces that on every deletion, because a
//! path-handling slip here destroys the user's footage rather than a file we
//! can rebuild.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bettercut_foundation::MediaId;

use crate::error::CacheError;
use crate::layout::CacheLayout;

/// §67's offered sizes.
pub const CACHE_LIMIT_5_GB: u64 = 5 * 1024 * 1024 * 1024;
pub const CACHE_LIMIT_10_GB: u64 = 10 * 1024 * 1024 * 1024;
pub const CACHE_LIMIT_20_GB: u64 = 20 * 1024 * 1024 * 1024;

/// One asset's cached data, for eviction decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    pub media: MediaId,
    pub dir: PathBuf,
    pub bytes: u64,
    /// Most recent access across the entry's files. Access time is unreliable
    /// on Windows (often disabled), so modification time is used as the
    /// stand-in — a proxy that is being used is one that was recently written.
    pub last_used: SystemTime,
}

pub struct CacheStore {
    layout: CacheLayout,
    limit_bytes: u64,
}

impl CacheStore {
    pub fn new(layout: CacheLayout, limit_bytes: u64) -> Self {
        Self {
            layout,
            limit_bytes,
        }
    }

    pub fn layout(&self) -> &CacheLayout {
        &self.layout
    }

    pub fn limit_bytes(&self) -> u64 {
        self.limit_bytes
    }

    pub fn set_limit_bytes(&mut self, limit: u64) {
        self.limit_bytes = limit;
    }

    /// Total bytes currently cached.
    pub fn total_bytes(&self) -> u64 {
        self.entries().iter().map(|entry| entry.bytes).sum()
    }

    /// One entry per cached asset.
    pub fn entries(&self) -> Vec<CacheEntry> {
        let Ok(dirs) = std::fs::read_dir(self.layout.media_root()) else {
            return Vec::new();
        };

        let mut entries = Vec::new();
        for dir in dirs.flatten() {
            let path = dir.path();
            if !path.is_dir() {
                continue;
            }
            let Some(media) = media_id_from_dir(&path) else {
                // Not one of ours. Leave it alone rather than guess.
                continue;
            };
            let (bytes, last_used) = measure(&path);
            entries.push(CacheEntry {
                media,
                dir: path,
                bytes,
                last_used,
            });
        }
        entries
    }

    /// Would `incoming` bytes fit within the limit?
    ///
    /// §67: *"Warn before generating proxies that would exceed the cache limit
    /// — proxies are large (§13.1)."* All-intra proxies are 3–5× a long-GOP
    /// equivalent, so this is a question worth asking before starting rather
    /// than after filling the disk.
    pub fn would_exceed_limit(&self, incoming: u64) -> bool {
        self.total_bytes().saturating_add(incoming) > self.limit_bytes
    }

    /// Evict least-recently-used assets until `headroom` bytes are free.
    ///
    /// `keep` names assets that must survive — the ones the open project is
    /// using. Evicting a proxy that is on screen would make it regenerate
    /// immediately, which is worse than being over the limit for a moment.
    ///
    /// Returns the bytes reclaimed.
    pub fn evict_to_fit(&self, headroom: u64, keep: &[MediaId]) -> Result<u64, CacheError> {
        let mut entries = self.entries();
        let mut total: u64 = entries.iter().map(|entry| entry.bytes).sum();

        let target = self.limit_bytes.saturating_sub(headroom);
        if total <= target {
            return Ok(0);
        }

        // Oldest first.
        entries.sort_by_key(|entry| entry.last_used);

        let mut reclaimed = 0;
        for entry in entries {
            if total <= target {
                break;
            }
            if keep.contains(&entry.media) {
                continue;
            }
            self.remove_dir(&entry.dir)?;
            total = total.saturating_sub(entry.bytes);
            reclaimed += entry.bytes;
            tracing::debug!(
                media = %entry.media.short(),
                bytes = entry.bytes,
                "evicted cached data"
            );
        }

        Ok(reclaimed)
    }

    /// Drop everything cached for one asset.
    pub fn purge(&self, media: MediaId) -> Result<(), CacheError> {
        let dir = self.layout.media_dir(media);
        if dir.exists() {
            self.remove_dir(&dir)?;
        }
        Ok(())
    }

    /// Delete a directory, refusing anything outside the cache root.
    ///
    /// The check is not paranoia about our own code so much as about future
    /// code: this is the one function in the project that deletes recursively,
    /// and the cost of it being handed the wrong path once is the user's
    /// footage.
    fn remove_dir(&self, dir: &Path) -> Result<(), CacheError> {
        if !self.is_inside_root(dir) {
            return Err(CacheError::OutsideCacheRoot {
                path: dir.to_path_buf(),
            });
        }
        std::fs::remove_dir_all(dir).map_err(|source| CacheError::Io {
            path: dir.to_path_buf(),
            source,
        })
    }

    /// Is `path` genuinely inside the cache root?
    ///
    /// Compared after canonicalising both, so `..` segments and symlinks cannot
    /// be used to step outside.
    fn is_inside_root(&self, path: &Path) -> bool {
        let root = self.layout.media_root();
        let (Ok(root), Ok(path)) = (root.canonicalize(), path.canonicalize()) else {
            return false;
        };
        path.starts_with(&root) && path != root
    }
}

/// Recover a `MediaId` from a cache directory name.
fn media_id_from_dir(path: &Path) -> Option<MediaId> {
    let name = path.file_name()?.to_str()?;
    uuid::Uuid::parse_str(name).ok().map(MediaId::from_uuid)
}

/// Total size and most recent modification time under `dir`.
fn measure(dir: &Path) -> (u64, SystemTime) {
    let mut bytes = 0;
    let mut newest = SystemTime::UNIX_EPOCH;

    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };

            if meta.is_dir() {
                stack.push(path);
            } else {
                bytes += meta.len();
                if let Ok(modified) = meta.modified()
                    && modified > newest
                {
                    newest = modified;
                }
            }
        }
    }

    (bytes, newest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(dir: &Path, limit: u64) -> CacheStore {
        CacheStore::new(CacheLayout::new(dir), limit)
    }

    /// Write `bytes` of proxy data for an asset, with a given age.
    fn write_proxy(store: &CacheStore, media: MediaId, bytes: usize) {
        store.layout().prepare(media).expect("prepare");
        std::fs::write(store.layout().proxy_file(media, 720), vec![0_u8; bytes])
            .expect("write proxy");
    }

    #[test]
    fn total_size_counts_every_cached_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), CACHE_LIMIT_5_GB);

        let (a, b) = (MediaId::new(), MediaId::new());
        write_proxy(&store, a, 1000);
        write_proxy(&store, b, 2000);

        assert_eq!(store.total_bytes(), 3000);
        assert_eq!(store.entries().len(), 2);
    }

    #[test]
    fn an_empty_cache_measures_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), CACHE_LIMIT_5_GB);
        assert_eq!(store.total_bytes(), 0);
        assert!(store.entries().is_empty());
    }

    /// §67: warn before a proxy run would blow the limit.
    #[test]
    fn oversized_additions_are_predicted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), 10_000);
        write_proxy(&store, MediaId::new(), 8_000);

        assert!(!store.would_exceed_limit(1_000));
        assert!(store.would_exceed_limit(5_000));
    }

    #[test]
    fn eviction_removes_the_least_recently_used_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), 3_000);

        let old = MediaId::new();
        write_proxy(&store, old, 2_000);
        // Ensure a measurable difference in modification time.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let recent = MediaId::new();
        write_proxy(&store, recent, 2_000);

        let reclaimed = store.evict_to_fit(0, &[]).expect("evict");

        assert!(reclaimed >= 2_000);
        assert!(
            !store.layout().media_dir(old).exists(),
            "the oldest entry survived"
        );
        assert!(
            store.layout().media_dir(recent).exists(),
            "the newest entry was evicted"
        );
    }

    /// Evicting media the open project is using would make it regenerate
    /// immediately — strictly worse than being briefly over the limit.
    #[test]
    fn media_in_use_is_never_evicted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), 1_000);

        let in_use = MediaId::new();
        write_proxy(&store, in_use, 5_000);

        store.evict_to_fit(0, &[in_use]).expect("evict");
        assert!(
            store.layout().media_dir(in_use).exists(),
            "an in-use proxy was evicted"
        );
    }

    #[test]
    fn eviction_stops_once_the_cache_fits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), 5_000);

        for _ in 0..4 {
            write_proxy(&store, MediaId::new(), 2_000);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(store.total_bytes(), 8_000);

        store.evict_to_fit(0, &[]).expect("evict");

        assert!(store.total_bytes() <= 5_000);
        assert!(
            !store.entries().is_empty(),
            "eviction cleared more than it needed to"
        );
    }

    #[test]
    fn nothing_is_evicted_when_the_cache_already_fits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), 10_000);
        write_proxy(&store, MediaId::new(), 1_000);

        assert_eq!(store.evict_to_fit(0, &[]).expect("evict"), 0);
        assert_eq!(store.total_bytes(), 1_000);
    }

    #[test]
    fn purging_removes_one_asset_and_leaves_the_rest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), CACHE_LIMIT_5_GB);

        let (a, b) = (MediaId::new(), MediaId::new());
        write_proxy(&store, a, 1_000);
        write_proxy(&store, b, 1_000);

        store.purge(a).expect("purge");
        assert!(!store.layout().media_dir(a).exists());
        assert!(store.layout().media_dir(b).exists());
    }

    #[test]
    fn purging_something_that_is_not_cached_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), CACHE_LIMIT_5_GB);
        assert!(store.purge(MediaId::new()).is_ok());
    }

    /// **The rule that must never bend** (§2, §74). This is the one recursive
    /// delete in the project; it must refuse anything outside the cache.
    #[test]
    fn deleting_outside_the_cache_root_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir.path().join("cache"), CACHE_LIMIT_5_GB);

        // Stand in for the user's footage, right next to the cache.
        let footage = dir.path().join("footage");
        std::fs::create_dir_all(&footage).expect("mkdir");
        std::fs::write(footage.join("interview.mp4"), b"precious").expect("write");

        let result = store.remove_dir(&footage);

        assert!(matches!(result, Err(CacheError::OutsideCacheRoot { .. })));
        assert!(
            footage.join("interview.mp4").exists(),
            "source media was deleted"
        );
    }

    /// A `..` escape must not get past the check either.
    #[test]
    fn traversal_out_of_the_cache_root_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = dir.path().join("cache");
        let store = store_in(&cache, CACHE_LIMIT_5_GB);
        std::fs::create_dir_all(store.layout().media_root()).expect("mkdir");

        let footage = dir.path().join("footage");
        std::fs::create_dir_all(&footage).expect("mkdir");

        let escaped = store
            .layout()
            .media_root()
            .join("..")
            .join("..")
            .join("footage");
        assert!(matches!(
            store.remove_dir(&escaped),
            Err(CacheError::OutsideCacheRoot { .. })
        ));
        assert!(
            footage.exists(),
            "traversal deleted a directory outside the cache"
        );
    }

    /// Deleting the media root itself would wipe every asset's cache at once.
    #[test]
    fn the_media_root_itself_cannot_be_deleted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), CACHE_LIMIT_5_GB);
        std::fs::create_dir_all(store.layout().media_root()).expect("mkdir");

        assert!(matches!(
            store.remove_dir(&store.layout().media_root()),
            Err(CacheError::OutsideCacheRoot { .. })
        ));
    }

    /// Foreign directories are left alone rather than guessed at.
    #[test]
    fn unrecognised_directories_are_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path(), CACHE_LIMIT_5_GB);
        std::fs::create_dir_all(store.layout().media_root().join("not-a-uuid")).expect("mkdir");

        assert!(store.entries().is_empty());
        assert_eq!(store.total_bytes(), 0);
    }
}
