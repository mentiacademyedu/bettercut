//! Events from the core to the UI (§56).
//!
//! Delivered over a **bounded** channel that drops rather than blocks when full.
//! A slow UI frame must never stall a decode, export, or audio thread — §56 and
//! §69 both say playback has priority over everything, and blocking a producer
//! on the UI would invert that.
//!
//! High-frequency events are throttled at the source: §56 requires playhead
//! position to update at most once per UI frame, not once per decoded frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use bettercut_foundation::{MediaId, SequenceId, TimelineTime};

/// Channel depth. Deep enough to absorb a burst of job progress between UI
/// frames, shallow enough that a stalled UI is noticed rather than buffered.
pub const EVENT_QUEUE_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Project data changed and any cached view of it is stale (§54's dirty flag).
    ProjectChanged,
    /// A different project was opened or created.
    ProjectLoaded,
    ProjectSaved,

    MediaImported(MediaId),
    /// §66: a referenced file has gone missing.
    MediaMissing(MediaId),
    /// §13: a proxy finished generating and preview should switch to it.
    ProxyReady(MediaId),

    PlaybackPositionChanged(TimelineTime),
    PlaybackStateChanged {
        playing: bool,
    },

    ActiveSequenceChanged(SequenceId),

    JobStarted {
        id: u64,
        label: String,
    },
    JobProgress {
        id: u64,
        fraction: f32,
    },
    JobFinished {
        id: u64,
    },
    JobFailed {
        id: u64,
        message: String,
    },
}

/// Send half. Cloneable and `Send`, so background threads can emit.
#[derive(Debug, Clone)]
pub struct EventSender {
    tx: SyncSender<Event>,
    dropped: Arc<AtomicU64>,
}

impl EventSender {
    /// Emit an event, dropping it if the queue is full.
    ///
    /// Returns whether it was delivered. Callers that care can check; most do
    /// not, because a dropped `ProjectChanged` is harmless — the next one will
    /// arrive and the UI repaints from current state either way.
    pub fn emit(&self, event: Event) -> bool {
        match self.tx.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(dropped)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(?dropped, "event queue full; event dropped");
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// How many events have been dropped. A non-zero, growing value means the
    /// UI is not draining fast enough — worth surfacing in diagnostics.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Receive half. Lives on the UI thread and is drained once per frame.
#[derive(Debug)]
pub struct EventReceiver {
    rx: Receiver<Event>,
}

impl EventReceiver {
    /// Take everything queued without blocking. Called once per UI frame.
    pub fn drain(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }

    /// Non-blocking single receive, for callers that want to avoid the `Vec`.
    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

pub fn event_channel() -> (EventSender, EventReceiver) {
    channel_with_depth(EVENT_QUEUE_DEPTH)
}

pub fn channel_with_depth(depth: usize) -> (EventSender, EventReceiver) {
    let (tx, rx) = sync_channel(depth);
    (
        EventSender {
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
        },
        EventReceiver { rx },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_arrive_in_order() {
        let (tx, rx) = event_channel();
        assert!(tx.emit(Event::ProjectChanged));
        assert!(tx.emit(Event::ProjectSaved));

        assert_eq!(rx.drain(), vec![Event::ProjectChanged, Event::ProjectSaved]);
        assert!(rx.drain().is_empty());
    }

    /// The property that matters: a full queue drops the event instead of
    /// blocking the producer (§56, §69).
    #[test]
    fn a_full_queue_drops_rather_than_blocking() {
        let (tx, rx) = channel_with_depth(2);
        assert!(tx.emit(Event::ProjectChanged));
        assert!(tx.emit(Event::ProjectChanged));

        // Third would block a plain bounded send. It must return instead.
        assert!(!tx.emit(Event::ProjectSaved));
        assert_eq!(tx.dropped_count(), 1);

        assert_eq!(rx.drain().len(), 2);
        // Draining makes room again.
        assert!(tx.emit(Event::ProjectSaved));
    }

    #[test]
    fn emitting_after_the_receiver_is_gone_is_not_an_error() {
        let (tx, rx) = event_channel();
        drop(rx);
        assert!(!tx.emit(Event::ProjectChanged));
        // And it did not count as a queue-full drop.
        assert_eq!(tx.dropped_count(), 0);
    }

    #[test]
    fn senders_can_be_moved_to_other_threads() {
        let (tx, rx) = event_channel();
        let handle = std::thread::spawn(move || {
            tx.emit(Event::JobFinished { id: 7 });
        });
        handle.join().expect("thread");
        assert_eq!(rx.drain(), vec![Event::JobFinished { id: 7 }]);
    }
}
