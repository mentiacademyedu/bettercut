//! The audio engine (§20a).
//!
//! Three pieces, in the order they matter:
//!
//! 1. [`AudioClock`] — **the master clock.** §20a.1: playback position comes
//!    from the audio device, never from a video timer.
//! 2. [`AudioOutput`] — the cpal stream and the lock-free buffer feeding it,
//!    obeying §20a.2's real-time rules.
//! 3. [`mixer`] — the §20a.4 summing order, which preview and export share so
//!    that §46 holds for sound as well as picture.
//!
//! What is *not* here: any knowledge of clips, tracks, or projects. This crate
//! turns sample buffers into sound and reports what time it is. Deciding which
//! samples belong where is the playback engine's job (§86).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod beats;
pub mod clock;
pub mod deesser;
pub mod eq;
pub mod error;
pub mod gate;
pub mod leveller;
pub mod loudness;
pub mod mixer;
pub mod output;
pub mod pitch;
pub mod recorder;
pub mod resample;
pub mod robot;
pub mod space;
pub mod voice;

pub use clock::AudioClock;
pub use deesser::{DeEsser, ESS_HZ, MAX_REDUCTION_DB};
pub use eq::{Equalizer, PRESENCE_HZ};
pub use error::AudioError;
pub use gate::{Gate, MAX_CLOSED_DB};
pub use leveller::Leveller;
pub use mixer::{
    FadeCurve, Fades, GainRamp, MAX_STEREO_WIDTH, MixParams, finish, mix_into, peaks, widen,
};
pub use output::{AudioOutput, AudioSink};
pub use pitch::{MAX_SEMITONES, PitchShifter};
pub use recorder::{Recorder, Recording};
pub use resample::{input_frames_needed, resample};
pub use robot::{ROBOT_HZ, robot};
pub use space::{Place, Space};
pub use voice::{MAX_DENOISE, VoiceCleaner};

use std::sync::Arc;

use bettercut_foundation::TimelineTime;

/// The playback clock, whether or not a device could be opened.
///
/// §20a.1: *"When a sequence has no audio, run a monotonic clock in its place
/// with the same interface."* The same applies when the machine has no working
/// sound device — editing must continue (§50), so the fallback is not an error
/// path but a supported mode.
pub enum PlaybackClock {
    Device(Box<AudioOutput>),
    /// No device. Time comes from `Instant`, pumped by the UI each frame.
    Monotonic {
        clock: Arc<AudioClock>,
        last_tick: std::cell::Cell<std::time::Instant>,
    },
}

impl PlaybackClock {
    /// Open the default device, falling back to a monotonic clock.
    ///
    /// Returns the sink when there is a real device; `None` means nothing will
    /// consume audio and the mixer should not bother producing it.
    pub fn open() -> (Self, Option<AudioSink>) {
        match AudioOutput::open() {
            Ok((output, sink)) => (Self::Device(Box::new(output)), Some(sink)),
            Err(err) => {
                tracing::warn!(%err, "no audio output; falling back to a monotonic clock");
                (
                    Self::Monotonic {
                        // 48 kHz so positions stay tick-exact (§9).
                        clock: Arc::new(AudioClock::new(48_000, 2)),
                        last_tick: std::cell::Cell::new(std::time::Instant::now()),
                    },
                    None,
                )
            }
        }
    }

    pub fn clock(&self) -> &Arc<AudioClock> {
        match self {
            Self::Device(output) => output.clock(),
            Self::Monotonic { clock, .. } => clock,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Device(output) => format!(
                "{} @ {} Hz, {} ch",
                output.device_name(),
                output.clock().sample_rate(),
                output.clock().channels()
            ),
            Self::Monotonic { .. } => "no device (monotonic clock)".to_owned(),
        }
    }

    /// Advance the fallback clock. A no-op when a real device is driving.
    ///
    /// Called once per UI frame. The device clock advances itself from its own
    /// callback, which is the entire point of §20a.1.
    pub fn tick(&self) {
        let Self::Monotonic { clock, last_tick } = self else {
            return;
        };

        let now = std::time::Instant::now();
        let elapsed = now.duration_since(last_tick.get());
        last_tick.set(now);

        if !clock.is_playing() {
            return;
        }
        let frames = (elapsed.as_secs_f64() * f64::from(clock.sample_rate())) as u64;
        clock.advance(frames);
    }

    pub fn position(&self) -> TimelineTime {
        self.clock().position()
    }

    pub fn is_playing(&self) -> bool {
        self.clock().is_playing()
    }

    pub fn set_playing(&self, playing: bool) {
        // Reset the fallback's reference point, so time spent paused is not
        // credited to the clock the moment it resumes.
        if let Self::Monotonic { last_tick, .. } = self {
            last_tick.set(std::time::Instant::now());
        }
        self.clock().set_playing(playing);
    }

    pub fn seek_to(&self, position: TimelineTime) {
        if let Self::Monotonic { last_tick, .. } = self {
            last_tick.set(std::time::Instant::now());
        }
        self.clock().seek_to(position);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback must behave like a clock, because on a machine with no
    /// sound device it *is* the clock.
    #[test]
    fn the_monotonic_fallback_advances_only_while_playing() {
        let clock = Arc::new(AudioClock::new(48_000, 2));
        let fallback = PlaybackClock::Monotonic {
            clock: Arc::clone(&clock),
            last_tick: std::cell::Cell::new(std::time::Instant::now()),
        };

        fallback.tick();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fallback.tick();
        assert_eq!(
            fallback.position(),
            TimelineTime::ZERO,
            "clock advanced while paused"
        );

        fallback.set_playing(true);
        std::thread::sleep(std::time::Duration::from_millis(30));
        fallback.tick();
        assert!(
            fallback.position().ticks() > 0,
            "clock did not advance while playing"
        );
    }

    #[test]
    fn seeking_the_fallback_moves_it() {
        let clock = Arc::new(AudioClock::new(48_000, 2));
        let fallback = PlaybackClock::Monotonic {
            clock,
            last_tick: std::cell::Cell::new(std::time::Instant::now()),
        };

        fallback.seek_to(TimelineTime::from_seconds(12));
        assert_eq!(fallback.position(), TimelineTime::from_seconds(12));
    }
}
