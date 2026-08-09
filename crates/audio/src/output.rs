//! The audio device and the buffer feeding it (§20a.2).
//!
//! ```text
//! decode -> resample -> mixer thread -> ring buffer -> cpal callback
//! ```
//!
//! # The callback is a real-time thread
//!
//! §20a.2 lists what it must never do:
//!
//! ```text
//! MUST NOT allocate
//! MUST NOT lock a mutex
//! MUST NOT touch project state
//! MUST NOT perform I/O
//! MUST NOT panic
//! ```
//!
//! The callback below reads a lock-free ring buffer, writes samples, and
//! touches two atomics. Nothing else. In particular there is **no logging**,
//! which §49 also calls out — `tracing` allocates and can take a lock.
//!
//! An underrun writes silence and increments a counter. It never blocks and
//! never waits: §69 puts the audio callback at priority 0, above everything,
//! and a callback that waits for the mixer would invert that.

use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};

use crate::clock::AudioClock;
use crate::error::AudioError;

/// Ring buffer depth, in milliseconds of audio.
///
/// §20a.2 asks for 100–200 ms. Deeper hides mixer hiccups but adds latency
/// between pressing play and hearing sound; 150 ms sits in the middle of the
/// range the guide specifies.
const RING_BUFFER_MS: usize = 150;

/// The mixer's end of the ring buffer.
///
/// Lives on the mixer thread. `push` returns how many samples were accepted;
/// a short write means the device has not drained yet, which is normal and not
/// an error — the mixer simply tries again next round.
pub struct AudioSink {
    producer: ringbuf::HeapProd<f32>,
    channels: usize,
    sample_rate: u32,
}

impl AudioSink {
    /// Push interleaved samples. Returns how many were accepted.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        self.producer.push_slice(samples)
    }

    /// Free space, in samples.
    pub fn vacant(&self) -> usize {
        self.producer.vacant_len()
    }

    /// Free space, in whole audio frames.
    pub fn vacant_frames(&self) -> usize {
        self.vacant() / self.channels.max(1)
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// An open output device.
///
/// Dropping this stops playback, so it has to outlive the clock it drives.
pub struct AudioOutput {
    _stream: cpal::Stream,
    clock: Arc<AudioClock>,
    device_name: String,
}

impl AudioOutput {
    /// Open the default output device.
    ///
    /// Asks for 48 kHz first, because §20a.3 normalizes everything internally
    /// to that rate and matching it avoids a second resample on the way out.
    pub fn open() -> Result<(Self, AudioSink), AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoOutputDevice)?;

        let device_name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "unknown".to_owned());

        let default_config = device
            .default_output_config()
            .map_err(|e| AudioError::DeviceConfig(e.to_string()))?;

        let at_48k = device
            .supported_output_configs()
            .ok()
            .and_then(|mut ranges| {
                ranges.find_map(|range| {
                    (range.sample_format() == cpal::SampleFormat::F32)
                        .then(|| range.try_with_sample_rate(48_000))
                        .flatten()
                })
            });

        let supported = at_48k.unwrap_or(default_config);
        let sample_format = supported.sample_format();
        let config = supported.config();

        if sample_format != cpal::SampleFormat::F32 {
            return Err(AudioError::DeviceConfig(format!(
                "device wants {sample_format:?} samples; only f32 is supported"
            )));
        }

        let channels = config.channels as usize;
        let sample_rate = config.sample_rate;

        let clock = Arc::new(AudioClock::new(sample_rate, config.channels));

        let capacity = (sample_rate as usize * RING_BUFFER_MS / 1000) * channels;
        let (producer, mut consumer) = HeapRb::<f32>::new(capacity.max(channels * 256)).split();

        let callback_clock = Arc::clone(&clock);
        let stream = device
            .build_output_stream(
                config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // ---- REAL-TIME SECTION (§20a.2) ----
                    // No allocation. No locks. No I/O. No logging. No panics.
                    let frames = (out.len() / channels) as u64;

                    if !callback_clock.is_playing() {
                        out.fill(0.0);
                        return;
                    }

                    let taken = consumer.pop_slice(out);
                    if taken < out.len() {
                        // §20a.2: underrun outputs silence and counts itself.
                        // It must never block waiting for the mixer.
                        out[taken..].fill(0.0);
                        callback_clock.note_underrun();
                    }

                    // The device consumed this much wall time whether or not
                    // the mixer kept up, so the clock advances regardless.
                    // §47a.4: video holds its last frame; audio never stops.
                    callback_clock.advance(frames);
                    // ---- END REAL-TIME SECTION ----
                },
                move |err| {
                    // Not the real-time thread, so reporting here is allowed.
                    tracing::error!(%err, "audio stream error");
                },
                Some(Duration::from_secs(2)),
            )
            .map_err(|e| AudioError::BuildStream(e.to_string()))?;

        stream.play().map_err(|e| AudioError::Play(e.to_string()))?;

        tracing::info!(
            device = %device_name,
            sample_rate,
            channels,
            buffer_ms = RING_BUFFER_MS,
            "audio output opened"
        );

        Ok((
            Self {
                _stream: stream,
                clock,
                device_name,
            },
            AudioSink {
                producer,
                channels,
                sample_rate,
            },
        ))
    }

    pub fn clock(&self) -> &Arc<AudioClock> {
        &self.clock
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }
}
