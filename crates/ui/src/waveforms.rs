//! Loaded waveforms, ready for the timeline to draw (§12, §53).
//!
//! The same shape as [`crate::thumbnails::ThumbnailStore`], and for the same
//! reason: the timeline redraws at 60 fps while scrolling, so it must not touch
//! the filesystem per clip per frame. Both the answer and the *absence* of one
//! are remembered, and the absence is cleared when analysis reports success.
//!
//! Waveforms stay in RAM as plain peaks rather than becoming textures — the
//! timeline draws them as line segments into its own `Painter`, so there is
//! nothing to upload.

use std::collections::HashMap;
use std::sync::Arc;

use bettercut_cache::{CacheStore, Waveform};
use bettercut_editor_core::foundation::MediaId;

#[derive(Default)]
pub struct WaveformStore {
    cache: Option<Arc<CacheStore>>,
    /// `None` means "looked, not there yet".
    loaded: HashMap<MediaId, Option<Arc<Waveform>>>,
}

impl WaveformStore {
    pub fn attach(&mut self, cache: Arc<CacheStore>) {
        self.cache = Some(cache);
        self.loaded.clear();
    }

    /// Analysis finished, so a remembered absence is now wrong.
    pub fn invalidate(&mut self, media: MediaId) {
        self.loaded.remove(&media);
    }

    /// The peaks for an asset, loading them on first request.
    ///
    /// `None` while analysis has not finished — the caller draws a plain block,
    /// which is what the timeline looked like before waveforms existed.
    pub fn get(&mut self, media: MediaId) -> Option<Arc<Waveform>> {
        if !self.loaded.contains_key(&media) {
            let loaded = self.load(media);
            self.loaded.insert(media, loaded);
        }
        self.loaded.get(&media).and_then(Clone::clone)
    }

    fn load(&self, media: MediaId) -> Option<Arc<Waveform>> {
        let cache = self.cache.as_ref()?;
        let path = cache.layout().waveform_file(media);
        if !path.exists() {
            return None;
        }

        match Waveform::read(&path) {
            Ok(waveform) => Some(Arc::new(waveform)),
            Err(err) => {
                // A corrupt entry is regenerable; drop it and move on.
                tracing::warn!(%err, path = %path.display(), "discarding a bad waveform");
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded.values().filter(|v| v.is_some()).count()
    }
}

impl std::fmt::Debug for WaveformStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WaveformStore")
            .field("known", &self.loaded.len())
            .field("loaded", &self.loaded_count())
            .finish()
    }
}
