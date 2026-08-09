//! Background work (§15, §48, §69).
//!
//! Everything expensive that is not playback runs through here: proxy
//! generation, thumbnails, waveforms, analysis, and eventually export.
//!
//! Three rules shape it, and all three come from the same worry — that
//! background work will make the editor feel slow:
//!
//! * **Bounded.** §74 forbids unlimited threads; §15 caps *heavy* jobs by core
//!   count.
//! * **Below normal priority.** §15.1 — capping the count is not enough, since
//!   one job at normal priority still competes with playback.
//! * **Cancellable.** §48 — checked inside loops, not just between jobs.
//!
//! §80 puts playback above all of this, and §88 says the editor must feel fast
//! before it feels powerful. This crate is where that gets enforced.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod priority;
pub mod scheduler;
pub mod thread_priority;

pub use priority::Priority;
pub use scheduler::{CancelToken, JobContext, JobEvent, JobId, JobScheduler, Task};
