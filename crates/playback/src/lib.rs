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
pub mod beat_markers;
pub mod cache;
pub mod compound;
pub mod duck;
pub mod engine;
pub mod error;
pub mod filmstrip_job;
pub mod frame_source;
pub mod lane_meters;
pub mod loudness_mix;
pub mod mixer;
pub mod normalise;
pub mod prefetch;
pub mod prefetcher;
pub mod proxy_job;
pub mod rendered;
pub mod scene_job;
pub mod scenes;
pub mod silence;
pub mod speech;
pub mod sync;
pub mod text_frames;
pub mod thumbnail_job;
pub mod tracker;
pub mod waveform_job;

pub use audio_source::AudioSource;
pub use beat_markers::{beat_markers, visualizer_levels};
pub use cache::{FrameCache, FrameKey};
pub use duck::{DuckSettings, duck_envelope};
pub use engine::{
    AudibleClip, LayerRequest, LayerSource, MAX_TRANSITION_BLUR, PlaybackEngine, ProxySource,
    ResolvedLayer, SMEAR_SAMPLES, adjustments_at, flash_alpha, graded_beneath, grain_seed,
    layer_requests, layer_transform, load_luts, resolve_audio_tracks, smear, solid_frame,
    source_time_of, transition_blur,
};
pub use error::PlaybackError;
pub use filmstrip_job::{FilmstripJob, TILE_WIDTH, TILES};
pub use frame_source::FrameSource;
pub use mixer::{AudioMixer, AudioPlan, BLOCK_FRAMES, MixerThread};
pub use normalise::{TARGET_DB, normalise};
pub use prefetch::{PrefetchBuffer, PrefetchItem, budget_for};
pub use prefetcher::{Plan, Prefetcher};
pub use proxy_job::ProxyJob;
pub use scene_job::{SceneJob, SceneReport};
pub use scenes::{FrameDigest, SceneSettings, scene_cuts};
pub use silence::{SilenceSettings, silent_ranges};
pub use speech::{SpeechSettings, speech_ranges};
pub use sync::{FramePlan, SyncDecision, plan_frame};
pub use text_frames::TextFrames;
pub use thumbnail_job::ThumbnailJob;
pub use waveform_job::WaveformJob;
