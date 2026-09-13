//! Continuous audio reading for one media asset.
//!
//! The decoder hands back buffers of whatever size the codec happens to use —
//! 1024 samples for AAC, 1152 for MP3, a whole packet for PCM. The mixer needs
//! *exactly* the samples covering a given span. This bridges the two by
//! keeping the leftovers.
//!
//! Everything here is in the §20a.3 internal format (48 kHz, `f32`, planar), so
//! a sample is exactly 20 ticks (§9) and sample-accurate positioning is plain
//! integer arithmetic rather than a rounded seek.

use bettercut_foundation::{MediaTime, TICKS_PER_AUDIO_SAMPLE};
use bettercut_media::{MediaAsset, MediaDecoder, NeverCancelled, SeekMode};

use crate::error::PlaybackError;

/// How far ahead of the buffer a request may land before seeking rather than
/// decoding forward.
///
/// Decoding forward a short distance is cheaper than a seek plus the decoder
/// flush that follows it; beyond about a second it stops being true.
const FORWARD_DECODE_LIMIT: i64 = bettercut_foundation::TICKS_PER_SECOND;

/// A rewind to within this of the file's start reopens the file and decodes
/// forward instead of seeking.
///
/// The first packets of a compressed audio stream carry the encoder's
/// priming — samples the decoder is told to drop when it reads from the
/// beginning. A seek lands on them without that instruction, and the sound
/// comes out shifted by a few thousand samples: harmless mid-file, where a
/// seek lands on ordinary packets, and audibly wrong at the start, which is
/// exactly where a reversed clip's last block reads. Reopening costs a file
/// open; decoding this far is a few milliseconds.
const REOPEN_BELOW: i64 = bettercut_foundation::TICKS_PER_SECOND;

pub struct AudioSource {
    decoder: Box<dyn MediaDecoder + Send>,
    /// The file, for reopening it (see [`REOPEN_BELOW`]).
    asset: MediaAsset,
    /// Planar samples currently held, one `Vec` per channel.
    buffered: Vec<Vec<f32>>,
    /// Source position of `buffered[..][0]`.
    buffered_start: MediaTime,
    channels: usize,
    drained: bool,
}

impl AudioSource {
    pub fn open(
        asset: &MediaAsset,
        mut decoder: Box<dyn MediaDecoder + Send>,
    ) -> Result<Self, PlaybackError> {
        decoder.open(asset)?;
        Ok(Self {
            decoder,
            asset: asset.clone(),
            buffered: Vec::new(),
            buffered_start: MediaTime::ZERO,
            channels: 0,
            drained: false,
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    fn buffered_frames(&self) -> usize {
        self.buffered.first().map_or(0, Vec::len)
    }

    fn buffered_end(&self) -> MediaTime {
        MediaTime::from_ticks(
            self.buffered_start.ticks() + self.buffered_frames() as i64 * TICKS_PER_AUDIO_SAMPLE,
        )
    }

    /// Read `frames` samples starting at `from`.
    ///
    /// Returns planar samples, shorter than requested at end of stream. Missing
    /// audio is silence, never an error: one clip running out must not stop the
    /// others (§50), and it certainly must not stall the device (§20a.2).
    pub fn read(&mut self, from: MediaTime, frames: usize) -> Result<Vec<Vec<f32>>, PlaybackError> {
        if frames == 0 {
            return Ok(Vec::new());
        }

        // Rewinding, or jumping far ahead, is cheaper as a seek.
        let behind = from < self.buffered_start;
        let far_ahead = from.ticks() > self.buffered_end().ticks() + FORWARD_DECODE_LIMIT;
        if behind || far_ahead {
            self.seek(from)?;
        }

        // Decode until the buffer covers the requested span.
        let wanted_end =
            MediaTime::from_ticks(from.ticks() + frames as i64 * TICKS_PER_AUDIO_SAMPLE);
        while !self.drained && self.buffered_end() < wanted_end {
            let Some(buffer) = self.decoder.decode_audio(&NeverCancelled)? else {
                self.drained = true;
                break;
            };

            // Nothing held — the first decode, *or the first after a seek*.
            // A seek empties the planes but keeps them, so asking whether the
            // list of planes is empty only caught the first case: after a
            // rewind the next buffer was filed under the seek target rather
            // than its own timestamp, and every sample read from it came from
            // the wrong instant. Reversed sound rewinds on every block, which
            // is how this surfaced; scrubbing backwards had it too.
            if self.buffered_frames() == 0 {
                self.channels = buffer.channels();
                self.buffered = vec![Vec::new(); self.channels];
                self.buffered_start = buffer.timestamp;
            }
            for (channel, plane) in buffer.planes.into_iter().enumerate() {
                if let Some(existing) = self.buffered.get_mut(channel) {
                    existing.extend_from_slice(&plane);
                }
            }
        }

        if self.buffered.is_empty() {
            return Ok(Vec::new());
        }

        // Where the request starts inside the buffer.
        let offset_ticks = from.ticks() - self.buffered_start.ticks();
        let offset = (offset_ticks / TICKS_PER_AUDIO_SAMPLE).max(0) as usize;
        let available = self.buffered_frames().saturating_sub(offset);
        let count = available.min(frames);

        let out: Vec<Vec<f32>> = self
            .buffered
            .iter()
            .map(|plane| {
                plane[offset.min(plane.len())..][..count.min(plane.len() - offset.min(plane.len()))]
                    .to_vec()
            })
            .collect();

        // Drop everything before the request; the mixer only ever moves
        // forward, so keeping it would grow the buffer without bound (§68).
        self.discard_before(from);

        Ok(out)
    }

    fn discard_before(&mut self, position: MediaTime) {
        let offset_ticks = position.ticks() - self.buffered_start.ticks();
        if offset_ticks <= 0 {
            return;
        }
        let drop = (offset_ticks / TICKS_PER_AUDIO_SAMPLE) as usize;
        let held = self.buffered_frames();
        let drop = drop.min(held);
        if drop == 0 {
            return;
        }
        for plane in &mut self.buffered {
            plane.drain(..drop.min(plane.len()));
        }
        self.buffered_start = MediaTime::from_ticks(
            self.buffered_start.ticks() + drop as i64 * TICKS_PER_AUDIO_SAMPLE,
        );
    }

    fn seek(&mut self, to: MediaTime) -> Result<(), PlaybackError> {
        if to.ticks() < REOPEN_BELOW {
            self.decoder.open(&self.asset)?;
        } else {
            self.decoder.seek(to, SeekMode::Precise)?;
        }
        for plane in &mut self.buffered {
            plane.clear();
        }
        self.buffered_start = to;
        self.drained = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_media::{AudioBuffer, CancellationToken, MediaError, VideoFrame};

    /// A decoder that emits a counting ramp in fixed-size blocks, so a test can
    /// tell exactly which samples it received.
    struct RampDecoder {
        next_sample: i64,
        block: usize,
        limit: i64,
    }

    impl MediaDecoder for RampDecoder {
        fn open(&mut self, _asset: &MediaAsset) -> Result<(), MediaError> {
            Ok(())
        }

        fn seek(&mut self, timestamp: MediaTime, _mode: SeekMode) -> Result<(), MediaError> {
            self.next_sample = timestamp.ticks() / TICKS_PER_AUDIO_SAMPLE;
            Ok(())
        }

        fn decode_frame(
            &mut self,
            _cancel: &dyn CancellationToken,
        ) -> Result<Option<VideoFrame>, MediaError> {
            Ok(None)
        }

        // Audio only, so there is no video position to report and nothing for
        // the sequential-read path to work with.
        fn decode_frame_at(
            &mut self,
            _target: MediaTime,
            _cancel: &dyn CancellationToken,
        ) -> Result<Option<VideoFrame>, MediaError> {
            Ok(None)
        }

        fn position(&self) -> Option<MediaTime> {
            None
        }

        fn frame_duration(&self) -> MediaTime {
            MediaTime::ZERO
        }

        fn decode_audio(
            &mut self,
            _cancel: &dyn CancellationToken,
        ) -> Result<Option<AudioBuffer>, MediaError> {
            if self.next_sample >= self.limit {
                return Ok(None);
            }
            let start = self.next_sample;
            let count = self.block.min((self.limit - start) as usize);
            let plane: Vec<f32> = (0..count).map(|i| (start + i as i64) as f32).collect();
            self.next_sample += count as i64;

            Ok(Some(AudioBuffer {
                timestamp: MediaTime::from_ticks(start * TICKS_PER_AUDIO_SAMPLE),
                planes: vec![plane],
                sample_rate: 48_000,
            }))
        }

        fn duration(&self) -> MediaTime {
            MediaTime::from_ticks(self.limit * TICKS_PER_AUDIO_SAMPLE)
        }
    }

    fn source(block: usize, limit: i64) -> AudioSource {
        let asset = MediaAsset::new(
            bettercut_media::MediaKind::Audio,
            "test.wav",
            MediaTime::from_seconds(10),
        );
        AudioSource::open(
            &asset,
            Box::new(RampDecoder {
                next_sample: 0,
                block,
                limit,
            }),
        )
        .expect("open")
    }

    fn at(sample: i64) -> MediaTime {
        MediaTime::from_ticks(sample * TICKS_PER_AUDIO_SAMPLE)
    }

    /// The core job: hand back exactly the samples asked for, regardless of the
    /// block size the decoder happens to use.
    #[test]
    fn reads_return_exactly_the_requested_samples() {
        let mut source = source(100, 10_000);

        let first = source.read(at(0), 250).expect("read");
        assert_eq!(first[0].len(), 250);
        assert_eq!(first[0][0], 0.0);
        assert_eq!(first[0][249], 249.0);
    }

    /// Consecutive reads must be seamless: a gap or a repeat here is an audible
    /// click on every buffer boundary.
    #[test]
    fn consecutive_reads_are_contiguous() {
        let mut source = source(100, 10_000);

        let mut expected = 0.0;
        for _ in 0..20 {
            let block = source.read(at(expected as i64), 137).expect("read");
            assert_eq!(block[0].len(), 137);
            for sample in &block[0] {
                assert_eq!(*sample, expected, "discontinuity in the sample stream");
                expected += 1.0;
            }
        }
    }

    #[test]
    fn a_read_that_does_not_align_with_the_decoder_blocks_still_works() {
        // Block size 100, reads of 7: every read straddles a block boundary.
        let mut source = source(100, 1_000);
        let mut expected = 0.0;
        for _ in 0..100 {
            let block = source.read(at(expected as i64), 7).expect("read");
            for sample in &block[0] {
                assert_eq!(*sample, expected);
                expected += 1.0;
            }
        }
    }

    #[test]
    fn seeking_backwards_rereads_from_the_new_position() {
        let mut source = source(100, 10_000);
        let _ = source.read(at(0), 500).expect("read");

        let back = source.read(at(10), 20).expect("read");
        assert_eq!(back[0][0], 10.0, "did not rewind");
    }

    #[test]
    fn jumping_far_ahead_seeks_rather_than_decoding_everything() {
        let mut source = source(100, 10_000_000);
        let _ = source.read(at(0), 100).expect("read");

        // A minute ahead: decoding forward would take a million samples.
        let far = 48_000 * 60;
        let block = source.read(at(far), 10).expect("read");
        assert_eq!(block[0][0], far as f32);
    }

    /// End of stream is short data, not an error - the mixer fills the rest
    /// with silence and playback continues (§50).
    #[test]
    fn reading_past_the_end_returns_what_is_left() {
        let mut source = source(100, 250);

        let block = source.read(at(200), 500).expect("read should not fail");
        assert_eq!(block[0].len(), 50, "expected only the remaining samples");
        assert_eq!(block[0][0], 200.0);
    }

    #[test]
    fn reading_entirely_past_the_end_returns_nothing() {
        let mut source = source(100, 250);
        let block = source.read(at(1_000), 100).expect("read");
        assert!(block.first().is_none_or(Vec::is_empty));
    }

    /// §68: the buffer must not grow without bound over a long playback run.
    #[test]
    fn the_internal_buffer_does_not_grow_without_bound() {
        let mut source = source(1_000, 10_000_000);

        let mut position = 0_i64;
        for _ in 0..500 {
            let _ = source.read(at(position), 480).expect("read");
            position += 480;
        }

        assert!(
            source.buffered_frames() < 10_000,
            "buffer grew to {} frames over a long read sequence",
            source.buffered_frames()
        );
    }

    #[test]
    fn a_zero_length_read_is_harmless() {
        let mut source = source(100, 1_000);
        assert!(source.read(at(0), 0).expect("read").is_empty());
    }
}
