//! The media abstraction from §3.
//!
//! > Create an internal Rust abstraction around FFmpeg. Do not allow the rest of
//! > the application to depend directly on a specific FFmpeg Rust wrapper.
//!
//! FFmpeg becomes one implementation of these traits in Milestone 2. Nothing
//! outside this crate names an `rsmpeg` type (§86).

use bettercut_foundation::MediaTime;

use crate::asset::MediaAsset;
use crate::color::ColorMetadata;
use crate::error::MediaError;

/// Where a decoded frame's pixels live.
///
/// §5 forbids a RAM round trip per frame in the hot path, but requires a
/// software fallback so unsupported hardware degrades rather than fails. This
/// enum is how a caller tells the two apart without caring which it got.
#[derive(Debug)]
pub enum FrameStorage {
    /// Already on the GPU — the §5 target path. Carries an opaque handle the
    /// renderer crate resolves; the media crate never names a wgpu type.
    Gpu { handle: u64 },
    /// System RAM, tightly packed. The §5 fallback path.
    System { data: Vec<u8>, stride: u32 },
}

impl FrameStorage {
    /// True when using this frame costs an upload (§5's fallback path).
    pub fn needs_upload(&self) -> bool {
        matches!(self, Self::System { .. })
    }
}

/// One decoded video frame.
#[derive(Debug)]
pub struct VideoFrame {
    /// Presentation time, already converted out of the source timebase (§9).
    pub timestamp: MediaTime,
    pub width: u32,
    pub height: u32,
    /// Source colour, for the §21a.2 conversion at upload. Nothing downstream
    /// of that boundary reads it.
    pub color: ColorMetadata,
    pub storage: FrameStorage,
}

/// Decoded audio, normalized to the §20a.3 internal format.
///
/// 48 kHz, `f32`, planar. Conversion happens in the decoder so the mixer only
/// ever sees one format.
#[derive(Debug)]
pub struct AudioBuffer {
    pub timestamp: MediaTime,
    /// One `Vec` per channel — planar, deinterleaved (§20a.3).
    pub planes: Vec<Vec<f32>>,
    pub sample_rate: u32,
}

impl AudioBuffer {
    pub fn channels(&self) -> usize {
        self.planes.len()
    }

    pub fn frames(&self) -> usize {
        self.planes.first().map_or(0, Vec::len)
    }
}

/// How precise a seek needs to be (§47a.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekMode {
    /// Dragging the playhead. Lowest latency wins; may return a nearby frame.
    Scrub,
    /// Frame-accurate. Correctness wins. Used for stop, arrow-key step, split.
    Precise,
    /// Sequential playback. Never actually seeks; prefetch wins.
    Playback,
}

/// Cooperative cancellation (§48).
///
/// §47a.5: seeking from 00:30 to 12:00 must cancel decode work around 00:30
/// immediately — checked *between frames*, not only between jobs.
pub trait CancellationToken: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

/// A token that never cancels, for call sites that have nothing to cancel.
#[derive(Debug, Clone, Copy, Default)]
pub struct NeverCancelled;

impl CancellationToken for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// The §3 decoder abstraction.
///
/// Implementations are single-threaded and owned by one decode thread; they are
/// deliberately not `Sync`.
pub trait MediaDecoder {
    fn open(&mut self, asset: &MediaAsset) -> Result<(), MediaError>;

    fn seek(&mut self, timestamp: MediaTime, mode: SeekMode) -> Result<(), MediaError>;

    /// Decode the next frame, or `Ok(None)` at end of stream.
    ///
    /// Implementations must check `cancel` inside their decode loop, not only on
    /// entry (§48).
    fn decode_frame(
        &mut self,
        cancel: &dyn CancellationToken,
    ) -> Result<Option<VideoFrame>, MediaError>;

    /// Decode forward to the frame whose span contains `target`, **without
    /// seeking** (§47a.2's `Playback`: "never seeks; reads sequentially").
    ///
    /// This is the mode ordinary playback runs in, and the reason it exists is
    /// §47a.1: a container seek only reaches the preceding keyframe, so a
    /// frame-accurate seek costs up to a whole GOP of decodes — up to 250 —
    /// *per displayed frame*. It also flushes the codec, which throws away the
    /// decoder's pipeline. Neither is acceptable once a frame, and neither is
    /// necessary when the next frame wanted is simply the next frame in the
    /// file.
    ///
    /// The caller is responsible for only calling this when `target` is ahead
    /// of [`MediaDecoder::position`]; there is no rewinding without a seek.
    fn decode_frame_at(
        &mut self,
        target: MediaTime,
        cancel: &dyn CancellationToken,
    ) -> Result<Option<VideoFrame>, MediaError>;

    /// Timestamp of the last video frame returned, if there was one.
    ///
    /// `None` after `open` or `seek`, because neither has decoded anything yet.
    fn position(&self) -> Option<MediaTime>;

    /// How long one video frame lasts, or zero if no video stream is open.
    fn frame_duration(&self) -> MediaTime;

    fn decode_audio(
        &mut self,
        cancel: &dyn CancellationToken,
    ) -> Result<Option<AudioBuffer>, MediaError>;

    fn duration(&self) -> MediaTime;
}

/// Probes a file's metadata without decoding it (§12).
pub trait MediaProber {
    fn probe(&self, path: &std::path::Path) -> Result<MediaAsset, MediaError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_frames_are_the_ones_that_cost_an_upload() {
        let ram = FrameStorage::System {
            data: vec![0; 16],
            stride: 4,
        };
        assert!(ram.needs_upload());
        assert!(!FrameStorage::Gpu { handle: 1 }.needs_upload());
    }

    #[test]
    fn audio_buffer_reports_planar_shape() {
        let buf = AudioBuffer {
            timestamp: MediaTime::ZERO,
            planes: vec![vec![0.0; 480], vec![0.0; 480]],
            sample_rate: 48_000,
        };
        assert_eq!(buf.channels(), 2);
        assert_eq!(buf.frames(), 480);
    }

    #[test]
    fn the_null_token_never_cancels() {
        assert!(!NeverCancelled.is_cancelled());
    }
}
