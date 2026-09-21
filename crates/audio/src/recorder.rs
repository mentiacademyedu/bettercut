//! Recording from the microphone: a voiceover spoken over the edit.
//!
//! Not the playback path, so §20a.2's no-locking rule is relaxed here: the
//! input callback appends to a buffer behind a mutex held for a copy's length.
//! A missed deadline on input costs a few samples of a voice, not a click in
//! the mix, and the recording is the only thing reading the buffer.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::error::AudioError;

/// What has been captured so far, and the loudest sample since last asked.
#[derive(Default)]
struct Captured {
    samples: Vec<f32>,
    peak: f32,
}

/// An open microphone, recording until [`Recorder::stop`].
pub struct Recorder {
    stream: cpal::Stream,
    captured: Arc<Mutex<Captured>>,
    sample_rate: u32,
    channels: u16,
    device_name: String,
}

impl Recorder {
    /// Start recording from the default input device.
    pub fn start() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(AudioError::NoInputDevice)?;
        let device_name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "unknown".to_owned());
        let supported = device
            .default_input_config()
            .map_err(|e| AudioError::DeviceConfig(e.to_string()))?;
        let format = supported.sample_format();
        let config = supported.config();
        let (sample_rate, channels) = (config.sample_rate, config.channels);

        let captured = Arc::new(Mutex::new(Captured::default()));
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, config, &captured),
            cpal::SampleFormat::I16 => build::<i16>(&device, config, &captured),
            cpal::SampleFormat::I32 => build::<i32>(&device, config, &captured),
            cpal::SampleFormat::U16 => build::<u16>(&device, config, &captured),
            other => {
                return Err(AudioError::DeviceConfig(format!(
                    "microphone gives {other:?} samples, which cannot be recorded"
                )));
            }
        }?;
        stream.play().map_err(|e| AudioError::Play(e.to_string()))?;
        tracing::info!(device = %device_name, sample_rate, channels, "recording started");

        Ok(Self {
            stream,
            captured,
            sample_rate,
            channels,
            device_name,
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// How long has been recorded, in seconds.
    pub fn seconds(&self) -> f64 {
        let frames = self
            .captured
            .lock()
            .map_or(0, |c| c.samples.len() / usize::from(self.channels.max(1)));
        frames as f64 / f64::from(self.sample_rate.max(1))
    }

    /// The loudest sample since this was last asked, 0–1, for a level meter.
    pub fn take_peak(&self) -> f32 {
        self.captured
            .lock()
            .map_or(0.0, |mut c| std::mem::take(&mut c.peak))
    }

    /// Stop, and hand back what was recorded.
    pub fn stop(self) -> Recording {
        drop(self.stream);
        let samples = self
            .captured
            .lock()
            .map(|mut c| std::mem::take(&mut c.samples))
            .unwrap_or_default();
        Recording {
            samples,
            sample_rate: self.sample_rate,
            channels: self.channels,
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    captured: &Arc<Mutex<Captured>>,
) -> Result<cpal::Stream, AudioError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let captured = Arc::clone(captured);
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                if let Ok(mut into) = captured.lock() {
                    let mut peak = into.peak;
                    into.samples.reserve(data.len());
                    for sample in data {
                        let value = <f32 as cpal::FromSample<T>>::from_sample_(*sample);
                        peak = peak.max(value.abs());
                        into.samples.push(value);
                    }
                    into.peak = peak;
                }
            },
            move |err| tracing::error!(%err, "microphone stream error"),
            None,
        )
        .map_err(|e| AudioError::BuildStream(e.to_string()))
}

/// Recorded sound: interleaved samples at the device's own rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Recording {
    /// How long it runs, in seconds.
    pub fn seconds(&self) -> f64 {
        let frames = self.samples.len() / usize::from(self.channels.max(1));
        frames as f64 / f64::from(self.sample_rate.max(1))
    }

    /// The recording as a 16-bit PCM WAV file's bytes.
    pub fn wav_bytes(&self) -> Vec<u8> {
        let channels = self.channels.max(1);
        let data_len = (self.samples.len() * 2) as u32;
        let byte_rate = self.sample_rate * u32::from(channels) * 2;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16_u32.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&(channels * 2).to_le_bytes());
        out.extend_from_slice(&16_u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for sample in &self.samples {
            let value = if sample.is_finite() {
                (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16
            } else {
                0
            };
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    /// Write it to `path` as a WAV file, making the folder if need be.
    pub fn write_wav(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(path)?;
        file.write_all(&self.wav_bytes())?;
        file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_header_describes_its_samples() {
        let recording = Recording {
            samples: vec![0.0, 1.0, -1.0, 0.5],
            sample_rate: 44_100,
            channels: 2,
        };
        let bytes = recording.wav_bytes();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            44_100
        );
        assert_eq!(bytes.len(), 44 + 8);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 32767);
        assert_eq!(i16::from_le_bytes([bytes[48], bytes[49]]), -32767);
        assert!((recording.seconds() - 2.0 / 44_100.0).abs() < 1e-9);
    }
}
