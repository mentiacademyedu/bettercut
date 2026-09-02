//! Rasterized text, cached (§26.1).
//!
//! One of these serves both configurations of the render graph: the preview
//! asks it for the picture on screen and the export asks it for the picture in
//! the file, and because it is the same rasterizer producing the same bitmap
//! from the same parameters, §46 holds for text without either side having to
//! try.
//!
//! The result is handed over as a [`VideoFrame`], which is not a fudge: a video
//! frame is a picture in memory with a timestamp and a known colour space, and
//! that is exactly what shaping some text produces. Saying so lets a title
//! travel through the compositor, the effect chain and the export writer on the
//! path that already exists, rather than a second one built beside it.

use std::collections::HashMap;
use std::sync::Arc;

use bettercut_foundation::MediaTime;
use bettercut_media::{ColorMetadata, FrameStorage, VideoFrame};
use bettercut_text::{TextError, TextRenderer};
use bettercut_timeline::TextClip;

/// How many rasterized titles to keep.
///
/// Small on purpose. Each entry is a full-size RGBA bitmap, and the working set
/// is "the titles visible around the playhead" — a handful, not a project's
/// worth. §17's memory rules apply here as much as to decoded frames.
const CAPACITY: usize = 16;

/// Shapes and caches text overlays.
pub struct TextFrames {
    renderer: TextRenderer,
    /// Keyed by the text and the style, not by the clip: two titles that say
    /// the same thing in the same way *are* the same picture, and re-typing a
    /// character and undoing it should not cost a re-shape.
    cache: HashMap<u64, Arc<VideoFrame>>,
    /// Insertion order, for eviction. A plain queue rather than a real LRU:
    /// with sixteen entries the difference is unmeasurable and the bookkeeping
    /// is not.
    order: Vec<u64>,
    /// Keys that produced nothing, so a title of blank space or missing glyphs
    /// is not re-shaped every frame.
    barren: std::collections::HashSet<u64>,
}

impl Default for TextFrames {
    fn default() -> Self {
        Self::new()
    }
}

impl TextFrames {
    pub fn new() -> Self {
        Self {
            renderer: TextRenderer::new(),
            cache: HashMap::new(),
            order: Vec::new(),
            barren: std::collections::HashSet::new(),
        }
    }

    /// The picture for this clip, shaping it if it is not already cached.
    ///
    /// `None` when there is nothing to draw — empty text, or a string with no
    /// glyphs in any available font. That is not an error the user needs
    /// telling about; the layer is simply absent.
    pub fn frame_for(&mut self, clip: &TextClip) -> Option<Arc<VideoFrame>> {
        let key = clip.style.key(&clip.text);
        if let Some(frame) = self.cache.get(&key) {
            return Some(Arc::clone(frame));
        }
        if self.barren.contains(&key) {
            return None;
        }

        match self.renderer.rasterize(&clip.text, &clip.style) {
            Ok(bitmap) => {
                let frame = Arc::new(VideoFrame {
                    timestamp: MediaTime::ZERO,
                    width: bitmap.width,
                    height: bitmap.height,
                    // Already in the working space (§21a): the rasterizer
                    // produced sRGB-encoded, full-range RGBA, so there is
                    // nothing to convert at upload.
                    color: ColorMetadata::srgb(),
                    storage: FrameStorage::System {
                        stride: bitmap.width * 4,
                        data: bitmap.pixels,
                    },
                });
                self.insert(key, Arc::clone(&frame));
                Some(frame)
            }
            Err(TextError::Empty) => {
                self.barren.insert(key);
                None
            }
            Err(err) => {
                tracing::warn!(%err, "could not rasterize a text overlay");
                self.barren.insert(key);
                None
            }
        }
    }

    fn insert(&mut self, key: u64, frame: Arc<VideoFrame>) {
        if self.order.len() >= CAPACITY
            && let Some(oldest) = self.order.first().copied()
        {
            self.order.remove(0);
            self.cache.remove(&oldest);
        }
        self.order.push(key);
        self.cache.insert(key, frame);
    }

    /// The font families installed on this machine (§26).
    ///
    /// Asked of the rasterizer that is already here rather than of a second
    /// one built for the purpose: a `FontSystem` reads every font directory on
    /// the machine, and doing that twice costs the startup time twice and the
    /// memory twice for an identical answer.
    pub fn families(&self) -> Vec<String> {
        self.renderer.families()
    }

    /// How many pictures are held, for diagnostics and tests.
    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.order.clear();
        self.barren.clear();
    }
}
