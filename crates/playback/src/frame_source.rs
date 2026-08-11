//! Opening decoders and pulling one frame out of them (§14, §47a.2).
//!
//! Lifted out of [`crate::engine`] so there can be **two** of these: the one
//! the UI thread uses to answer "what is on screen right now", and the one the
//! decode-ahead thread uses to work in front of the playhead (§47a.3).
//!
//! Two is not an accident of design — an `FfmpegDecoder` owns a demuxer with a
//! single read position, so sharing one between a thread seeking to the
//! playhead and a thread reading forward would make them fight over that
//! position. The symptom would be stuttering, which is the exact thing
//! decode-ahead exists to remove.

use std::collections::HashMap;
use std::sync::Arc;

use bettercut_foundation::{MediaId, MediaTime};
use bettercut_media::{
    CancellationToken, FfmpegDecoder, MediaAsset, MediaDecoder, SeekMode, VideoFrame,
};

use crate::engine::ProxySource;
use crate::error::PlaybackError;

/// Which copy of a media file a decoder was opened against.
///
/// Part of the decoder key: switching to a proxy mid-session must open a new
/// decoder rather than reuse the one pointed at the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Original,
    Proxy,
}

/// A pool of open decoders, one per (asset, copy).
pub struct FrameSource {
    decoders: HashMap<(MediaId, Source), FfmpegDecoder>,
    proxy: Option<ProxySource>,
    /// FFmpeg threads per decoder (§15.1), from `HardwareProfile`.
    threads: u32,
    /// How far the last decode landed from what was asked for.
    last_seek_error: MediaTime,
}

impl FrameSource {
    pub fn new(threads: u32) -> Self {
        Self {
            decoders: HashMap::new(),
            proxy: None,
            threads: threads.max(1),
            last_seek_error: MediaTime::ZERO,
        }
    }

    pub fn set_proxy_source(&mut self, proxy: Option<ProxySource>) {
        self.proxy = proxy;
        // Every open decoder points at whichever copy was current when it was
        // opened.
        self.decoders.clear();
    }

    /// Drop the decoders for one asset, so the next request reopens it.
    pub fn invalidate(&mut self, media: MediaId) {
        self.decoders.retain(|(cached, _), _| *cached != media);
    }

    pub fn clear(&mut self) {
        self.decoders.clear();
    }

    pub fn last_seek_error(&self) -> MediaTime {
        self.last_seek_error
    }

    /// Decode the frame covering `source_time`.
    ///
    /// `cancel` is checked inside the decode loop, not merely on entry (§48),
    /// which is what lets a seek abandon decode-ahead work immediately.
    pub fn decode(
        &mut self,
        asset: &MediaAsset,
        source_time: MediaTime,
        mode: SeekMode,
        cancel: &dyn CancellationToken,
    ) -> Result<Arc<VideoFrame>, PlaybackError> {
        // §14: preview reads the proxy when there is one. §13.1's all-intra
        // encoding is what makes the seek below cost one decode instead of a
        // walk forward from the previous keyframe.
        let (source, opened) = match self.proxy.as_ref().and_then(|p| p.path_for(asset.id)) {
            Some(path) => {
                let mut proxy_asset = asset.clone();
                proxy_asset.path = path;
                (Source::Proxy, proxy_asset)
            }
            None => (Source::Original, asset.clone()),
        };

        let threads = self.threads;
        let decoder = match self.decoders.entry((asset.id, source)) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut decoder = FfmpegDecoder::new(threads)?;
                decoder.open(&opened)?;
                entry.insert(decoder)
            }
        };

        decoder.seek(source_time, mode)?;
        let frame = decoder
            .decode_frame(cancel)?
            .ok_or(PlaybackError::NoFrame { at: source_time })?;

        self.last_seek_error =
            MediaTime::from_ticks((frame.timestamp.ticks() - source_time.ticks()).abs());

        Ok(Arc::new(frame))
    }

    /// How many decoders are open, for diagnostics.
    pub fn open_decoders(&self) -> usize {
        self.decoders.len()
    }
}

// `FrameSource` is `Send` automatically: `FfmpegDecoder` already declares it
// (with the justification, in the one crate permitted `unsafe`), and everything
// else here is a plain map, an `Arc` and a number. Moving one onto the decode
// thread therefore needs no unsafe code in this crate, which the workspace
// forbids outright.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<FrameSource>();
};
