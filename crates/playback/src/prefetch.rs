//! Decode-ahead ring buffer (§47a.3, §47a.5).
//!
//! ```text
//! Target: 0.5-1.0 seconds of decoded frames ahead of the playhead
//! Bounded by byte count, not frame count
//! Backpressure: decode thread blocks when full - never grows unbounded
//! ```
//!
//! # Why this is not the frame cache
//!
//! [`crate::cache::FrameCache`] is byte-bounded too, but it is an **LRU**: when
//! it fills, it throws the oldest frame away. That is right for scrubbing,
//! where the user may come back to anything. It is wrong for decode-ahead,
//! where filling up means *stop decoding*, not *discard what you already did*.
//! An LRU producer would run flat out, evicting frames it was about to need,
//! and burn a core doing it. So this is a bounded queue with real backpressure:
//! the decode thread waits until the consumer takes something.
//!
//! # Cancellation is a counter
//!
//! §47a.5: *"Seeking from 00:30 to 12:00 cancels all decode work around 00:30
//! immediately — checked between frames, not only between jobs."* Every request
//! carries a generation. Seeking bumps it, which empties the buffer and makes
//! the decode thread abandon its current plan at the next frame boundary. No
//! thread is killed and no work is waited on; the stale frames are simply never
//! inserted.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

use bettercut_foundation::{MediaId, MediaTime};
use bettercut_media::VideoFrame;

use crate::cache::FrameKey;

/// One frame the prefetcher should decode, in playback order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefetchItem {
    pub media: MediaId,
    /// Where in the source, already resolved from the timeline.
    pub source: MediaTime,
}

impl PrefetchItem {
    pub fn key(&self) -> FrameKey {
        FrameKey {
            media: self.media,
            timestamp: self.source,
        }
    }
}

struct Inner {
    frames: HashMap<FrameKey, Arc<VideoFrame>>,
    /// Insertion order, so the oldest can be dropped if a consumer never takes
    /// one — a paused preview must not deadlock the decode thread forever.
    order: VecDeque<FrameKey>,
    bytes: usize,
    max_bytes: usize,
    /// Bumped on every seek. Frames from an older generation are discarded.
    generation: u64,
    closed: bool,
}

impl Inner {
    fn drop_oldest(&mut self) -> bool {
        let Some(key) = self.order.pop_front() else {
            return false;
        };
        if let Some(frame) = self.frames.remove(&key) {
            self.bytes = self.bytes.saturating_sub(frame_bytes(&frame));
        }
        true
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.order.clear();
        self.bytes = 0;
    }
}

/// A bounded, byte-limited buffer of decoded frames shared with a decode
/// thread.
pub struct PrefetchBuffer {
    inner: Mutex<Inner>,
    /// Signalled when space is freed or the generation changes, so a blocked
    /// producer wakes rather than polling.
    space: Condvar,
    /// How long a producer waits for room before making room itself.
    patience: std::time::Duration,
}

impl PrefetchBuffer {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                frames: HashMap::new(),
                order: VecDeque::new(),
                bytes: 0,
                // A budget below one frame would block forever; one 4K RGBA
                // frame is ~33 MB, so refuse to go under that.
                max_bytes: max_bytes.max(8 * 1024 * 1024),
                generation: 0,
                closed: false,
            }),
            space: Condvar::new(),
            patience: std::time::Duration::from_millis(100),
        }
    }

    /// The generation a producer should be working on.
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// Abandon everything and start a new generation (§47a.5).
    ///
    /// Returns the new generation. Called on seek and on any change that makes
    /// the decoded-ahead frames wrong.
    pub fn reset(&self) -> u64 {
        let mut inner = self.lock();
        inner.clear();
        inner.generation = inner.generation.wrapping_add(1);
        let generation = inner.generation;
        drop(inner);
        // Wake a producer blocked on a full buffer so it notices the change
        // rather than sleeping through the seek.
        self.space.notify_all();
        generation
    }

    /// Take a frame out, freeing its bytes.
    ///
    /// Removing rather than borrowing is what makes this a queue: the space is
    /// returned to the producer the moment the consumer has what it needs.
    pub fn take(&self, key: &FrameKey) -> Option<Arc<VideoFrame>> {
        let mut inner = self.lock();
        let frame = inner.frames.remove(key)?;
        inner.bytes = inner.bytes.saturating_sub(frame_bytes(&frame));
        if let Some(at) = inner.order.iter().position(|k| k == key) {
            inner.order.remove(at);
        }
        drop(inner);
        self.space.notify_all();
        Some(frame)
    }

    pub fn contains(&self, key: &FrameKey) -> bool {
        self.lock().frames.contains_key(key)
    }

    pub fn bytes(&self) -> usize {
        self.lock().bytes
    }

    pub fn len(&self) -> usize {
        self.lock().frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Insert a decoded frame, waiting while the buffer is full.
    ///
    /// Returns `false` when the frame should be thrown away: the generation
    /// moved on (a seek), or the buffer was closed. The producer treats that as
    /// "stop working on this plan".
    ///
    /// The wait is bounded. A paused preview takes nothing, and a producer that
    /// blocked forever would hold its decoder and never see the next seek —
    /// so after the timeout the oldest frame is dropped to make room. That
    /// keeps the invariant that matters (never grow unbounded) without the
    /// deadlock.
    pub fn push(&self, generation: u64, key: FrameKey, frame: Arc<VideoFrame>) -> bool {
        let bytes = frame_bytes(&frame);
        let mut inner = self.lock();

        loop {
            if inner.closed || inner.generation != generation {
                return false;
            }
            if inner.bytes + bytes <= inner.max_bytes || inner.frames.is_empty() {
                break;
            }

            let (guard, timeout) = self
                .space
                .wait_timeout(inner, self.patience)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner = guard;

            if timeout.timed_out() {
                // Nobody is consuming. Make room rather than stalling here.
                if !inner.drop_oldest() {
                    break;
                }
            }
        }

        if inner.closed || inner.generation != generation {
            return false;
        }

        if inner.frames.insert(key, frame).is_none() {
            inner.order.push_back(key);
        }
        inner.bytes += bytes;
        true
    }

    /// Stop accepting frames and wake anything blocked (§48).
    /// The same buffer, with a producer waiting `patience` for room before it
    /// drops the oldest frame itself.
    pub fn with_patience(mut self, patience: std::time::Duration) -> Self {
        self.patience = patience;
        self
    }

    pub fn close(&self) {
        let mut inner = self.lock();
        inner.closed = true;
        inner.clear();
        drop(inner);
        self.space.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// A poisoned lock means a producer panicked mid-insert. The buffer is
    /// regenerable by definition, so carrying on with whatever is there beats
    /// taking the whole application down.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Bytes a frame occupies, for the byte budget (§47a.3).
fn frame_bytes(frame: &VideoFrame) -> usize {
    match &frame.storage {
        bettercut_media::FrameStorage::System { data, .. } => data.len(),
        // A GPU frame costs a handle here; its memory is the renderer's.
        bettercut_media::FrameStorage::Gpu { .. } => 0,
    }
}

/// How many bytes 0.5-1.0 seconds of decoded frames need (§47a.3).
///
/// Derived rather than guessed: at 1080p RGBA a frame is ~8.3 MB, so one second
/// at 30 fps is ~250 MB — far past §81's whole 500 MB budget. That is exactly
/// why §14 puts editing on proxies: at 540p a frame is ~2 MB, so a second is
/// ~62 MB, which fits.
///
/// The caller passes the frame size it actually expects, and this returns a
/// budget for `seconds` of it, clamped so a mistake cannot swallow the heap.
pub fn budget_for(frame_bytes: usize, fps: f64, seconds: f64) -> usize {
    let frames = (fps * seconds).ceil().max(1.0) as usize;
    let wanted = frame_bytes.saturating_mul(frames);
    wanted.clamp(8 * 1024 * 1024, 192 * 1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_media::{FrameStorage, VideoFrame};

    fn frame(bytes: usize) -> Arc<VideoFrame> {
        Arc::new(VideoFrame {
            timestamp: MediaTime::ZERO,
            width: 16,
            height: 16,
            color: Default::default(),
            storage: FrameStorage::System {
                data: vec![0_u8; bytes],
                stride: 64,
            },
        })
    }

    fn key(n: i64) -> FrameKey {
        FrameKey {
            media: MediaId::new(),
            timestamp: MediaTime::from_ticks(n),
        }
    }

    #[test]
    fn a_pushed_frame_can_be_taken_once() {
        let buffer = PrefetchBuffer::new(16 * 1024 * 1024);
        let k = key(1);

        assert!(buffer.push(0, k, frame(1024)));
        assert!(buffer.contains(&k));
        assert!(buffer.take(&k).is_some());
        assert!(buffer.take(&k).is_none(), "the same frame came back twice");
        assert!(buffer.is_empty());
    }

    #[test]
    fn taking_frees_the_bytes() {
        let buffer = PrefetchBuffer::new(16 * 1024 * 1024);
        let k = key(1);
        buffer.push(0, k, frame(4096));
        assert_eq!(buffer.bytes(), 4096);
        buffer.take(&k);
        assert_eq!(buffer.bytes(), 0, "bytes were not returned to the budget");
    }

    /// §47a.5: a seek must invalidate work in flight, not merely stop new work.
    #[test]
    fn a_reset_discards_frames_and_rejects_the_old_generation() {
        let buffer = PrefetchBuffer::new(16 * 1024 * 1024);
        let old = buffer.generation();
        buffer.push(old, key(1), frame(1024));
        assert_eq!(buffer.len(), 1);

        let new = buffer.reset();
        assert_ne!(new, old);
        assert!(buffer.is_empty(), "a seek left stale frames behind");
        assert!(
            !buffer.push(old, key(2), frame(1024)),
            "a frame from before the seek was accepted"
        );
        assert!(buffer.push(new, key(3), frame(1024)));
    }

    /// The invariant §47a.3 exists for: never grow unbounded.
    #[test]
    fn the_buffer_never_exceeds_its_budget_by_more_than_one_frame() {
        // The floor is 8 MB, so use frames big enough to hit it.
        let buffer = PrefetchBuffer::new(8 * 1024 * 1024);
        let frame_size = 1024 * 1024;

        for i in 0..40 {
            buffer.push(0, key(i), frame(frame_size));
        }

        assert!(
            buffer.bytes() <= 8 * 1024 * 1024 + frame_size,
            "buffer grew to {} bytes",
            buffer.bytes()
        );
    }

    /// Backpressure: a producer blocks while full, and is released the instant
    /// the consumer takes something.
    #[test]
    fn a_full_buffer_blocks_the_producer_until_space_is_taken() {
        let buffer = Arc::new(PrefetchBuffer::new(8 * 1024 * 1024));
        let frame_size = 4 * 1024 * 1024;

        let first = key(1);
        assert!(buffer.push(0, first, frame(frame_size)));
        assert!(buffer.push(0, key(2), frame(frame_size)));

        let producer = {
            let buffer = Arc::clone(&buffer);
            std::thread::spawn(move || buffer.push(0, key(3), frame(frame_size)))
        };

        // Give the producer time to reach the wait, then release it.
        std::thread::sleep(std::time::Duration::from_millis(50));
        buffer.take(&first);

        assert!(producer.join().expect("producer thread"), "push failed");
    }

    /// A paused preview consumes nothing. The producer must not be stuck
    /// forever holding its decoder — it drops the oldest and carries on.
    #[test]
    fn a_producer_is_not_deadlocked_when_nothing_consumes() {
        let buffer = Arc::new(PrefetchBuffer::new(8 * 1024 * 1024));
        let frame_size = 3 * 1024 * 1024;

        let buffer2 = Arc::clone(&buffer);
        let producer = std::thread::spawn(move || {
            for i in 0..6 {
                if !buffer2.push(0, key(i), frame(frame_size)) {
                    return false;
                }
            }
            true
        });

        assert!(
            producer.join().expect("producer thread"),
            "the producer never finished; it is deadlocked on a full buffer"
        );
        assert!(buffer.bytes() <= 8 * 1024 * 1024 + frame_size);
    }

    #[test]
    fn closing_releases_a_blocked_producer() {
        // Patient enough that only the close can release it: with the usual
        // 100 ms, a slow machine's 30 ms sleep ran past it and the producer
        // made room itself (seen on GitHub's Macs).
        let buffer = Arc::new(
            PrefetchBuffer::new(8 * 1024 * 1024).with_patience(std::time::Duration::from_secs(10)),
        );
        buffer.push(0, key(1), frame(6 * 1024 * 1024));

        let buffer2 = Arc::clone(&buffer);
        let producer = std::thread::spawn(move || buffer2.push(0, key(2), frame(6 * 1024 * 1024)));

        std::thread::sleep(std::time::Duration::from_millis(30));
        buffer.close();

        assert!(
            !producer.join().expect("producer thread"),
            "a push succeeded after close"
        );
        assert!(buffer.is_closed());
    }

    /// A frame larger than the whole budget must still go in, or playback of
    /// 4K originals would stall forever waiting for space that never comes.
    #[test]
    fn a_frame_larger_than_the_budget_is_still_accepted_when_empty() {
        let buffer = PrefetchBuffer::new(8 * 1024 * 1024);
        assert!(buffer.push(0, key(1), frame(40 * 1024 * 1024)));
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn the_budget_covers_the_requested_span() {
        // 540p RGBA is about 2 MB; a second at 30 fps should fit comfortably.
        let bytes = 960 * 540 * 4;
        let budget = budget_for(bytes, 30.0, 1.0);
        assert!(budget >= bytes * 30, "budget too small for one second");
        assert!(budget <= 192 * 1024 * 1024, "budget past its ceiling");
    }

    /// §81 budgets 500 MB for everything. A 1080p original at 30 fps would want
    /// 250 MB for one second, so the ceiling has to bite.
    #[test]
    fn the_budget_is_capped_for_large_frames() {
        let bytes = 1920 * 1080 * 4;
        assert_eq!(budget_for(bytes, 30.0, 1.0), 192 * 1024 * 1024);
    }

    #[test]
    fn the_budget_has_a_floor() {
        assert_eq!(budget_for(1024, 30.0, 0.5), 8 * 1024 * 1024);
    }
}
