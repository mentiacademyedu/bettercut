//! Playback coordination (§47a).
//!
//! Sits between the editor's project state and the decoder, audio, and
//! renderer crates. It knows what a clip is; it does not know what a window is.
//!
//! ```text
//! audio device -> AudioClock -> position -> which clips -> decode -> layers
//! ```
//!
//! §20a.1's rule runs through everything here: the audio device sets the time
//! and the picture follows it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod audio_source;
pub mod cache;
pub mod engine;
pub mod error;
pub mod frame_source;
pub mod prefetch;
pub mod prefetcher;
pub mod proxy_job;
pub mod sync;
pub mod thumbnail_job;
pub mod waveform_job;

pub use audio_source::AudioSource;
pub use cache::{FrameCache, FrameKey};
pub use engine::{AudibleClip, PlaybackEngine, ProxySource, ResolvedLayer, source_time_of};
pub use error::PlaybackError;
pub use frame_source::FrameSource;
pub use prefetch::{PrefetchBuffer, PrefetchItem, budget_for};
pub use prefetcher::{Plan, Prefetcher};
pub use proxy_job::ProxyJob;
pub use sync::{FramePlan, SyncDecision, plan_frame};
pub use thumbnail_job::ThumbnailJob;
pub use waveform_job::WaveformJob;
