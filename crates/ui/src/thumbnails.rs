//! Turning cached thumbnails into GPU textures (§12, §19).
//!
//! The cache holds raw RGBA; egui wants a `TextureHandle`. This does the
//! conversion once per asset and remembers the answer, including the negative
//! one — a browser with 500 assets must not stat 500 files every frame at
//! 60 fps, which is precisely the kind of idle cost §81 budgets against.
//!
//! A negative answer is cleared by [`ThumbnailStore::invalidate`] when the job
//! that generates one reports success.

use std::collections::HashMap;
use std::sync::Arc;

use bettercut_cache::{CacheStore, Thumbnail};
use bettercut_editor_core::foundation::MediaId;

/// What we know about one asset's thumbnail.
enum Slot {
    /// Uploaded and ready.
    Ready(egui::TextureHandle),
    /// Looked, found nothing. Not retried until invalidated.
    Absent,
}

#[derive(Default)]
pub struct ThumbnailStore {
    cache: Option<Arc<CacheStore>>,
    width: u32,
    slots: HashMap<MediaId, Slot>,
    /// Filmstrip sheets, kept in their own map because a clip wants the strip
    /// and the browser wants the poster, and neither substitutes for the other.
    strips: HashMap<MediaId, Slot>,
}

impl ThumbnailStore {
    /// Point the store at the cache. Called once, at startup.
    pub fn attach(&mut self, cache: Arc<CacheStore>, width: u32) {
        self.cache = Some(cache);
        self.width = width;
        self.slots.clear();
        self.strips.clear();
    }

    /// A thumbnail became available, so the remembered "absent" is now wrong.
    pub fn invalidate(&mut self, media: MediaId) {
        self.slots.remove(&media);
        self.strips.remove(&media);
    }

    /// The filmstrip sheet for a clip, loading it on first request (§53).
    ///
    /// Returns the texture and how many tiles it holds, so the caller can map a
    /// position along the clip onto a tile.
    pub fn filmstrip(
        &mut self,
        ctx: &egui::Context,
        media: MediaId,
    ) -> Option<(egui::TextureHandle, u32)> {
        if !self.strips.contains_key(&media) {
            let slot = self
                .load_strip(ctx, media)
                .map_or(Slot::Absent, Slot::Ready);
            self.strips.insert(media, slot);
        }
        match self.strips.get(&media) {
            Some(Slot::Ready(handle)) => Some((handle.clone(), bettercut_playback::TILES)),
            _ => None,
        }
    }

    fn load_strip(&self, ctx: &egui::Context, media: MediaId) -> Option<egui::TextureHandle> {
        let cache = self.cache.as_ref()?;
        let path = cache.layout().filmstrip_file(
            media,
            bettercut_playback::TILES,
            bettercut_playback::TILE_WIDTH,
        );
        if !path.exists() {
            return None;
        }

        let sheet = match Thumbnail::read(&path) {
            Ok(sheet) => sheet,
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "discarding a bad filmstrip");
                let _ = std::fs::remove_file(&path);
                return None;
            }
        };

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [sheet.width as usize, sheet.height as usize],
            &sheet.rgba,
        );
        Some(ctx.load_texture(
            format!("strip-{media:?}"),
            image,
            egui::TextureOptions::LINEAR,
        ))
    }

    /// The texture for an asset, loading it on first request.
    ///
    /// Returns `None` while the thumbnail does not exist yet — the caller draws
    /// a placeholder rather than waiting, because generation is a background
    /// job and §2 does not allow the UI to block on it.
    pub fn texture(&mut self, ctx: &egui::Context, media: MediaId) -> Option<&egui::TextureHandle> {
        if !self.slots.contains_key(&media) {
            let slot = self.load(ctx, media).map_or(Slot::Absent, Slot::Ready);
            self.slots.insert(media, slot);
        }
        match self.slots.get(&media) {
            Some(Slot::Ready(handle)) => Some(handle),
            _ => None,
        }
    }

    fn load(&self, ctx: &egui::Context, media: MediaId) -> Option<egui::TextureHandle> {
        let cache = self.cache.as_ref()?;
        let path = cache.layout().thumbnail_file(media, self.width);
        if !path.exists() {
            return None;
        }

        let thumbnail = match Thumbnail::read(&path) {
            Ok(thumbnail) => thumbnail,
            Err(err) => {
                // A corrupt cache entry is not the user's problem: drop it so
                // the next scan regenerates one.
                tracing::warn!(%err, path = %path.display(), "discarding a bad thumbnail");
                let _ = std::fs::remove_file(&path);
                return None;
            }
        };

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [thumbnail.width as usize, thumbnail.height as usize],
            &thumbnail.rgba,
        );
        Some(ctx.load_texture(
            format!("thumb-{media:?}"),
            image,
            // Linear: these are only ever drawn smaller than or equal to their
            // stored size, where nearest would shimmer as the panel resizes.
            egui::TextureOptions::LINEAR,
        ))
    }

    /// How many textures are currently held, for diagnostics.
    pub fn loaded(&self) -> usize {
        self.slots
            .values()
            .filter(|s| matches!(s, Slot::Ready(_)))
            .count()
    }
}

impl std::fmt::Debug for ThumbnailStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThumbnailStore")
            .field("width", &self.width)
            .field("slots", &self.slots.len())
            .field("loaded", &self.loaded())
            .finish()
    }
}
