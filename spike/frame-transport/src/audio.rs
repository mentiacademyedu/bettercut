//! Milestone 0 spike — the audio master clock (§20a.1).
//!
//! The rule being validated: **playback position is derived from the audio
//! device, never from a video timer.** The cpal callback advances a sample
//! counter; the UI reads that counter to decide which video frame to show.
//!
//! The callback obeys §20a.2: no allocation, no locking, no I/O, no logging,
//! no panics. It touches two atomics and writes samples.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Shared between the real-time audio callback and the UI thread.
pub struct AudioClock {
    /// Audio frames (per-channel sample groups) handed to the device so far.
    frames_played: AtomicU64,
    /// Callbacks that ran while stopped — the "underrun" proxy for this spike.
    silent_callbacks: AtomicU32,
    playing: AtomicBool,
    sample_rate: u32,
    channels: u16,
}

impl AudioClock {
    /// Playback position in seconds, as derived from the audio device.
    pub fn position_secs(&self) -> f64 {
        self.frames_played.load(Ordering::Acquire) as f64 / self.sample_rate as f64
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::Relaxed);
    }

    pub fn reset(&self) {
        self.frames_played.store(0, Ordering::Release);
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn silent_callbacks(&self) -> u32 {
        self.silent_callbacks.load(Ordering::Relaxed)
    }
}

/// Fallback used when no audio device exists. §20a.1: "When a sequence has no
/// audio, run a monotonic clock in its place with the same interface."
pub struct MonotonicClock {
    origin: std::time::Instant,
    accumulated: std::cell::Cell<f64>,
    last: std::cell::Cell<f64>,
}

pub enum PlaybackClock {
    Device(AudioEngine),
    Monotonic {
        clock: Arc<AudioClock>,
        wall: MonotonicClock,
    },
}

pub struct AudioEngine {
    // Dropping the stream stops audio, so it must outlive the clock.
    _stream: cpal::Stream,
    clock: Arc<AudioClock>,
    device_name: String,
}

impl PlaybackClock {
    pub fn clock(&self) -> &Arc<AudioClock> {
        match self {
            Self::Device(engine) => &engine.clock,
            Self::Monotonic { clock, .. } => clock,
        }
    }

    pub fn description(&self) -> String {
        match self {
            Self::Device(engine) => format!(
                "{} @ {} Hz, {} ch",
                engine.device_name,
                engine.clock.sample_rate(),
                engine.clock.channels()
            ),
            Self::Monotonic { .. } => "no device — monotonic fallback".to_owned(),
        }
    }

    /// Only the monotonic fallback needs pumping; the device clock advances itself.
    pub fn tick(&self) {
        if let Self::Monotonic { clock, wall } = self {
            let now = wall.origin.elapsed().as_secs_f64();
            let delta = now - wall.last.get();
            wall.last.set(now);
            if clock.is_playing() {
                wall.accumulated.set(wall.accumulated.get() + delta);
                let frames = (wall.accumulated.get() * clock.sample_rate() as f64) as u64;
                clock.frames_played.store(frames, Ordering::Release);
            }
        }
    }

    pub fn reset(&self) {
        self.clock().reset();
        if let Self::Monotonic { wall, .. } = self {
            wall.accumulated.set(0.0);
        }
    }

    pub fn start() -> Self {
        match Self::try_open_device() {
            Ok(engine) => Self::Device(engine),
            Err(err) => {
                eprintln!("audio device unavailable ({err}); using monotonic clock");
                let clock = Arc::new(AudioClock {
                    frames_played: AtomicU64::new(0),
                    silent_callbacks: AtomicU32::new(0),
                    playing: AtomicBool::new(false),
                    sample_rate: 48_000,
                    channels: 2,
                });
                Self::Monotonic {
                    clock,
                    wall: MonotonicClock {
                        origin: std::time::Instant::now(),
                        accumulated: std::cell::Cell::new(0.0),
                        last: std::cell::Cell::new(0.0),
                    },
                }
            }
        }
    }

    fn try_open_device() -> Result<AudioEngine, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no default output device".to_owned())?;
        let device_name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "unknown".to_owned());

        let default_config = device
            .default_output_config()
            .map_err(|e| format!("default_output_config: {e}"))?;

        // §9/§20a.3: everything internal runs at 48 kHz. Ask the device for it;
        // accept its default if no supported range covers 48 kHz.
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

        let config = at_48k.unwrap_or(default_config).config();

        let clock = Arc::new(AudioClock {
            frames_played: AtomicU64::new(0),
            silent_callbacks: AtomicU32::new(0),
            playing: AtomicBool::new(false),
            sample_rate: config.sample_rate,
            channels: config.channels,
        });

        let cb_clock = Arc::clone(&clock);
        let channels = config.channels as usize;
        let sample_rate = config.sample_rate as f32;
        let mut phase = 0.0f32;

        let stream = device
            .build_output_stream(
                config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // ---- REAL-TIME SECTION (§20a.2) ----
                    // No allocation. No locks. No I/O. No logging. No panics.
                    let frames = out.len() / channels;

                    if !cb_clock.playing.load(Ordering::Relaxed) {
                        for sample in out.iter_mut() {
                            *sample = 0.0;
                        }
                        cb_clock.silent_callbacks.fetch_add(1, Ordering::Relaxed);
                        return;
                    }

                    // A 220 Hz tone with a click every second: the click is what
                    // makes A/V sync error audible against the video's flash frame.
                    let step = 220.0 * std::f32::consts::TAU / sample_rate;
                    let played = cb_clock.frames_played.load(Ordering::Relaxed);

                    for f in 0..frames {
                        phase += step;
                        if phase > std::f32::consts::TAU {
                            phase -= std::f32::consts::TAU;
                        }
                        let abs_frame = played + f as u64;
                        // Click on the first 25 ms of every second.
                        let in_click = abs_frame % sample_rate as u64 <= sample_rate as u64 / 40;
                        let amp = if in_click { 0.25 } else { 0.06 };
                        let value = phase.sin() * amp;
                        for c in 0..channels {
                            out[f * channels + c] = value;
                        }
                    }

                    cb_clock
                        .frames_played
                        .fetch_add(frames as u64, Ordering::Release);
                    // ---- END REAL-TIME SECTION ----
                },
                move |err| {
                    // Error callback is not the RT thread; printing here is allowed.
                    eprintln!("audio stream error: {err}");
                },
                None,
            )
            .map_err(|e| format!("build_output_stream: {e}"))?;

        stream.play().map_err(|e| format!("stream.play: {e}"))?;

        Ok(AudioEngine {
            _stream: stream,
            clock,
            device_name,
        })
    }
}
