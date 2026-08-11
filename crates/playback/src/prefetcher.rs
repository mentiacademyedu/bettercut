//! The decode-ahead thread (§47a.3, §47a.5).
//!
//! Owns one thread and one [`FrameSource`], and fills a [`PrefetchBuffer`] with
//! frames the playhead is about to need. The UI thread computes *what* to
//! decode — that needs the project, which lives there — and sends a plan; this
//! does the decoding, which is what must not happen on the UI thread (§2).
//!
//! ```text
//! UI thread                        decode thread
//! ---------                        -------------
//! plan next 1s of frames  ──────►  seek, decode, push
//! take(key) from buffer   ◄──────  (blocks while full)
//! seek → reset()          ──────►  abandons plan at next frame
//! ```
//!
//! One thread, not a pool. §74 forbids unbounded background threads, and a
//! second decoder for the same file would fight the first over its read
//! position rather than going faster.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use bettercut_media::{CancellationToken, MediaAsset, SeekMode};

use crate::frame_source::FrameSource;
use crate::prefetch::PrefetchBuffer;

/// What to decode ahead, in playback order.
pub struct Plan {
    pub generation: u64,
    /// Which copy to read (§14).
    ///
    /// Carried on the plan rather than set through a side channel: the frame
    /// key is `(media, timestamp)` and says nothing about which copy produced
    /// it, so a decode thread reading originals while the preview expects
    /// proxies would quietly fill the buffer with the wrong frames.
    pub proxy: Option<crate::engine::ProxySource>,
    /// Cloned assets: the decode thread cannot borrow the project.
    pub items: Vec<(MediaAsset, bettercut_foundation::MediaTime)>,
}

/// Cancels as soon as the buffer's generation moves past this plan (§47a.5).
///
/// Handed to the decoder so the check happens *inside* its loop, not merely
/// between frames — a long GOP walk must abandon partway, not at the end.
struct PlanCancelled {
    buffer: Arc<PrefetchBuffer>,
    generation: u64,
}

impl CancellationToken for PlanCancelled {
    fn is_cancelled(&self) -> bool {
        self.buffer.is_closed() || self.buffer.generation() != self.generation
    }
}

pub struct Prefetcher {
    buffer: Arc<PrefetchBuffer>,
    /// `Option` so `Drop` can drop the sender *before* joining.
    ///
    /// The decode thread parks in `requests.recv()`, which only returns once
    /// every sender is gone. Joining while still holding this one deadlocks —
    /// the thread waits for a plan that cannot come, and the join waits for the
    /// thread.
    plans: Option<Sender<Plan>>,
    /// Frames decoded since start, for diagnostics and tests.
    decoded: Arc<AtomicU64>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Prefetcher {
    /// Start the decode thread.
    pub fn start(budget_bytes: usize, threads: u32) -> Self {
        let buffer = Arc::new(PrefetchBuffer::new(budget_bytes));
        let (plans, requests) = channel();
        let decoded = Arc::new(AtomicU64::new(0));

        let handle = {
            let buffer = Arc::clone(&buffer);
            let decoded = Arc::clone(&decoded);
            std::thread::Builder::new()
                .name("bettercut-decode-ahead".to_owned())
                .spawn(move || {
                    // §69: below-normal priority. Decode-ahead is work for
                    // frames nobody is waiting on yet, so it must never take
                    // the CPU from the frame being shown now.
                    bettercut_jobs::thread_priority::lower_current_thread();
                    run(&buffer, &requests, &decoded, threads);
                })
                .ok()
        };

        if handle.is_none() {
            tracing::warn!("could not start the decode-ahead thread; playback will decode inline");
        }

        Self {
            buffer,
            plans: Some(plans),
            decoded,
            handle,
        }
    }

    pub fn buffer(&self) -> &Arc<PrefetchBuffer> {
        &self.buffer
    }

    pub fn decoded_frames(&self) -> u64 {
        self.decoded.load(Ordering::Relaxed)
    }

    /// Abandon work in flight and start a new generation (§47a.5).
    pub fn reset(&self) -> u64 {
        self.buffer.reset()
    }

    /// Queue a plan. Returns false if the thread is gone.
    pub fn submit(&self, plan: Plan) -> bool {
        self.plans.as_ref().is_some_and(|p| p.send(plan).is_ok())
    }

    pub fn generation(&self) -> u64 {
        self.buffer.generation()
    }
}

impl Drop for Prefetcher {
    fn drop(&mut self) {
        // Order matters (§48):
        //   1. close wakes a thread blocked pushing into a full buffer,
        //   2. dropping the sender ends its `recv` loop,
        //   3. only then is joining safe.
        // Skipping step 2 deadlocks: `recv` returns when the last sender goes.
        self.buffer.close();
        self.plans = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run(
    buffer: &Arc<PrefetchBuffer>,
    requests: &Receiver<Plan>,
    decoded: &Arc<AtomicU64>,
    threads: u32,
) {
    let mut source = FrameSource::new(threads);
    let mut proxy: Option<crate::engine::ProxySource> = None;

    while let Ok(mut plan) = requests.recv() {
        // Only the newest plan matters. Draining first avoids decoding a
        // second of frames for a position the playhead has already left.
        loop {
            match requests.try_recv() {
                Ok(newer) => plan = newer,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        // Only when it actually changed: `set_proxy_source` drops every open
        // decoder, and doing that per plan would reopen the file on every
        // seek — the opposite of what decode-ahead is for.
        if plan.proxy != proxy {
            proxy = plan.proxy.clone();
            source.set_proxy_source(proxy.clone());
        }

        for (asset, source_time) in plan.items {
            if buffer.is_closed() || buffer.generation() != plan.generation {
                break; // §47a.5: abandon at the frame boundary
            }

            let key = crate::cache::FrameKey {
                media: asset.id,
                timestamp: source_time,
            };
            if buffer.contains(&key) {
                continue;
            }

            let cancel = PlanCancelled {
                buffer: Arc::clone(buffer),
                generation: plan.generation,
            };

            // `Precise` because the consumer asks for exact instants; §13.1's
            // all-intra proxies are what keep that to one decode.
            let frame = match source.decode(&asset, source_time, SeekMode::Precise, &cancel) {
                Ok(frame) => frame,
                Err(err) => {
                    // Not fatal: the UI thread will decode it itself and report
                    // properly. Decode-ahead failing is a performance problem,
                    // not a correctness one (§50).
                    tracing::debug!(file = %asset.file_name, %err, "decode-ahead skipped a frame");
                    continue;
                }
            };

            if !buffer.push(plan.generation, key, frame) {
                break; // superseded or closed
            }
            decoded.fetch_add(1, Ordering::Relaxed);
        }
    }
}
