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

/// Decoded stills kept per `FrameSource`: two, so a crossfade between photos
/// holds both sides without decoding either twice.
///
/// Small on purpose. A still is up to `MAX_STILL_EDGE`² × 4 bytes — tens of
/// megabytes — and there are three `FrameSource`s (preview, decode-ahead,
/// export). A montage plays its photos in order, so the two most recent are
/// the two that matter.
const STILLS_KEPT: usize = 2;

/// A pool of open decoders, one per (asset, copy).
pub struct FrameSource {
    decoders: HashMap<(MediaId, Source), FfmpegDecoder>,
    /// Decoded stills, most recently used last.
    stills: std::collections::VecDeque<(MediaId, Arc<VideoFrame>)>,
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
            stills: std::collections::VecDeque::new(),
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
        self.stills.retain(|(cached, _)| *cached != media);
    }

    pub fn clear(&mut self) {
        self.decoders.clear();
        self.stills.clear();
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
        if asset.is_still() {
            return self.still(asset, cancel);
        }

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

        // §47a.2's `Playback`: **never seeks; reads sequentially.**
        //
        // This is the difference between playback working and playback
        // crawling. Asking for a frame used to mean seeking to it, and a
        // container seek only reaches the preceding keyframe — §47a.1 puts
        // that at up to 250 decodes on long-GOP media, *for every frame
        // displayed*, plus a codec flush that discards the pipeline. All-intra
        // proxies (§13.1) hide it, because there the preceding keyframe is the
        // frame itself; the moment playback runs on the original, or before a
        // proxy has finished building, the cost is the whole GOP.
        //
        // During ordinary playback the next frame wanted is the next frame in
        // the file, and reading it costs one decode.
        let frame = if is_next_frame(decoder, source_time) {
            decoder.decode_frame_at(source_time, cancel)?
        } else {
            decoder.seek(source_time, mode)?;
            decoder.decode_frame(cancel)?
        };
        let frame = frame.ok_or(PlaybackError::NoFrame { at: source_time })?;

        self.last_seek_error =
            MediaTime::from_ticks((frame.timestamp.ticks() - source_time.ticks()).abs());

        Ok(Arc::new(frame))
    }

    /// A still's one picture, decoded once.
    ///
    /// The decoder is opened, read and dropped rather than pooled: it owns an
    /// RGBA buffer as large as the picture, and keeping it open would hold
    /// every still twice. Always the original — stills have no proxies.
    fn still(
        &mut self,
        asset: &MediaAsset,
        cancel: &dyn CancellationToken,
    ) -> Result<Arc<VideoFrame>, PlaybackError> {
        self.last_seek_error = MediaTime::ZERO;
        if let Some(index) = self.stills.iter().position(|(id, _)| *id == asset.id)
            && let Some(entry) = self.stills.remove(index)
        {
            let frame = Arc::clone(&entry.1);
            self.stills.push_back(entry);
            return Ok(frame);
        }

        let mut decoder = FfmpegDecoder::new(self.threads)?;
        decoder.open(asset)?;
        let frame = decoder
            .decode_frame(cancel)?
            .ok_or(PlaybackError::NoFrame {
                at: MediaTime::ZERO,
            })?;
        let frame = Arc::new(frame);

        self.stills.push_back((asset.id, Arc::clone(&frame)));
        while self.stills.len() > STILLS_KEPT {
            self.stills.pop_front();
        }
        Ok(frame)
    }

    /// How many decoders are open, for diagnostics.
    pub fn open_decoders(&self) -> usize {
        self.decoders.len()
    }
}

/// How far ahead of the decoder a request may be and still be read forward.
///
/// One frame is the playback case. A few more covers frames dropped under load
/// (§47a.4) and arrow-key stepping, which would otherwise pay for a seek to
/// reach the very next frame.
///
/// It has to stay small. Walking forward is only a win while it is cheaper than
/// a seek, and on an all-intra proxy a seek costs exactly one decode — so a
/// generous window would make proxies slower to serve the very case they exist
/// for. Past this, seeking is the better bet.
const MAX_SEQUENTIAL_FRAMES: i64 = 4;

/// Whether `target` is close enough ahead of the decoder to read forward to.
///
/// Deliberately strict about the lower bound: `target` must be past the end of
/// the frame the decoder last produced. A request landing *inside* that frame
/// wants the frame already decoded, and reading forward would return the next
/// one — a silent off-by-one that a seek gets right.
fn is_next_frame(decoder: &impl MediaDecoder, target: MediaTime) -> bool {
    let (Some(position), duration) = (decoder.position(), decoder.frame_duration()) else {
        return false;
    };
    if duration.ticks() <= 0 {
        // No frame duration means no way to judge the distance; a stream that
        // does not report one is unusual enough to be worth a real seek.
        return false;
    }

    let ahead = target.ticks() - position.ticks();
    ahead >= duration.ticks() && ahead <= duration.ticks() * MAX_SEQUENTIAL_FRAMES
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

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_media::{AudioBuffer, CancellationToken, MediaError};

    /// Reports a position and a frame duration; decodes nothing.
    struct Stub {
        position: Option<MediaTime>,
        duration: MediaTime,
    }

    impl MediaDecoder for Stub {
        fn open(&mut self, _asset: &MediaAsset) -> Result<(), MediaError> {
            Ok(())
        }
        fn seek(&mut self, _timestamp: MediaTime, _mode: SeekMode) -> Result<(), MediaError> {
            Ok(())
        }
        fn decode_frame(
            &mut self,
            _cancel: &dyn CancellationToken,
        ) -> Result<Option<VideoFrame>, MediaError> {
            Ok(None)
        }
        fn decode_frame_at(
            &mut self,
            _target: MediaTime,
            _cancel: &dyn CancellationToken,
        ) -> Result<Option<VideoFrame>, MediaError> {
            Ok(None)
        }
        fn decode_audio(
            &mut self,
            _cancel: &dyn CancellationToken,
        ) -> Result<Option<AudioBuffer>, MediaError> {
            Ok(None)
        }
        fn position(&self) -> Option<MediaTime> {
            self.position
        }
        fn frame_duration(&self) -> MediaTime {
            self.duration
        }
        fn duration(&self) -> MediaTime {
            MediaTime::ZERO
        }
    }

    /// 29.97 fps.
    const FRAME: i64 = 32_032;

    fn at(position_frames: i64) -> Stub {
        Stub {
            position: Some(MediaTime::from_ticks(position_frames * FRAME)),
            duration: MediaTime::from_ticks(FRAME),
        }
    }

    fn wants(decoder: &Stub, frame: i64) -> bool {
        is_next_frame(decoder, MediaTime::from_ticks(frame * FRAME))
    }

    /// The case this exists for: playing frame 11 after frame 10.
    #[test]
    fn the_very_next_frame_is_read_forward() {
        assert!(wants(&at(10), 11));
    }

    /// A few frames dropped under load (§47a.4) still beats a seek.
    #[test]
    fn a_short_gap_is_still_read_forward() {
        assert!(wants(&at(10), 12));
        assert!(wants(&at(10), 14));
    }

    /// Past the window, seeking wins — and on an all-intra proxy a seek is one
    /// decode, so the window must not be generous.
    #[test]
    fn a_long_jump_seeks() {
        assert!(!wants(&at(10), 15));
        assert!(!wants(&at(10), 600));
    }

    /// Backwards is never sequential; there is no rewinding without a seek.
    #[test]
    fn going_backwards_seeks() {
        assert!(!wants(&at(10), 9));
        assert!(!wants(&at(10), 0));
    }

    /// A request landing *inside* the frame already decoded wants that frame,
    /// not the next one. Reading forward would quietly return the wrong one,
    /// so it has to seek.
    #[test]
    fn a_request_inside_the_current_frame_seeks() {
        assert!(!wants(&at(10), 10));
        let decoder = at(10);
        assert!(
            !is_next_frame(&decoder, MediaTime::from_ticks(10 * FRAME + FRAME / 2)),
            "half a frame ahead is still inside the frame already decoded"
        );
    }

    /// Nothing decoded yet means nothing to read forward from.
    #[test]
    fn a_fresh_decoder_seeks() {
        let fresh = Stub {
            position: None,
            duration: MediaTime::from_ticks(FRAME),
        };
        assert!(!is_next_frame(&fresh, MediaTime::from_ticks(FRAME)));
    }

    /// Without a frame duration there is no way to judge the distance, and
    /// guessing would risk handing back the wrong frame.
    #[test]
    fn a_stream_with_no_declared_rate_seeks() {
        let unknown = Stub {
            position: Some(MediaTime::ZERO),
            duration: MediaTime::ZERO,
        };
        assert!(!is_next_frame(&unknown, MediaTime::from_ticks(FRAME)));
    }
}
