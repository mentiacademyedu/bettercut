//! Export in a browser (the web build).
//!
//! The same frames as [`crate::export`] — the same layers, the same
//! compositor, the same mix — but nothing here may wait: a browser decodes,
//! copies off the GPU and encodes on its own time, and the page must keep
//! drawing meanwhile. So an export is a [`WebExport`] the app steps once a
//! frame, which does whatever is ready and returns.
//!
//! Frames come from the page's own `<video>` sought to each instant
//! ([`FrameSource::wait_for_exact_frames`]); the browser's encoders make
//! H.264 and AAC or Opus (`bettercut_media::browser::export`); and
//! [`bettercut_media::mp4_mux`] writes the MP4, which is handed to the person
//! as a download. Sound only is the mix as a WAV, as on the desktop, made a
//! block a step.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bettercut_foundation::clock::Instant;
use bettercut_foundation::{SequenceId, TICKS_PER_AUDIO_SAMPLE, TimelineTime, ticks_per_frame};
use bettercut_media::browser::{self, export as encoders};
use bettercut_media::mp4_mux::{Mp4Sample, Mp4Track, Mp4TrackKind, mux};
use bettercut_media::{NeverCancelled, SeekMode, VideoCodec};
use bettercut_playback::{FrameSource, TextFrames, layer_requests};
use bettercut_project_format::Project;
use bettercut_renderer::{Compositor, RenderConfig, wgpu};
use bettercut_timeline::Sequence;

use crate::readback::Readback;
use crate::{AudioMixdown, ExportError, ExportSettings, audio_channels, composite_layers, decode_layers};

/// How long one step may work before handing the page back, in
/// milliseconds: most of a 60 Hz frame, so the window stays responsive.
const STEP_BUDGET_MS: u128 = 12;

/// Sound is mixed at this rate, and the browser's encoders take it as is.
const SOUND_RATE: u32 = 48_000;

/// Seconds between keyframes: a player can start or seek at least this often.
const KEYFRAME_SECONDS: f64 = 2.0;

/// What an export is doing, for the interface.
#[derive(Debug, Clone, PartialEq)]
pub enum WebProgress {
    Working { done: u64, total: u64 },
    /// The file has been handed to the browser as a download.
    Finished { name: String, frames: u64 },
    Failed(String),
}

enum Stage {
    /// The browser is choosing and opening its encoders.
    Starting,
    /// Ready for the next frame.
    Rendering,
    /// A frame is being copied off the GPU.
    Reading(Arc<AtomicBool>),
    /// Every frame given; the encoders are finishing.
    Finishing,
    /// Sound only: the mix going into a WAV, a block a step.
    Sound(Box<crate::wav::WavWriter<std::io::Cursor<Vec<u8>>>>),
    Over(WebProgress),
}

/// One export in a browser, stepped by the app once a frame.
pub struct WebExport {
    project: Project,
    sequence: Sequence,
    settings: ExportSettings,
    device: wgpu::Device,
    queue: wgpu::Queue,
    compositor: Compositor,
    readback: Readback,
    frames: FrameSource,
    titles: TextFrames,
    audio: AudioMixdown,
    channels: usize,
    ticks_per_frame: i64,
    total: u64,
    index: u64,
    samples_written: i64,
    key_every: u64,
    stage: Stage,
}

/// The H.264 rate the desktop picks when none is given: 0.2 bits a pixel
/// at 30 frames a second, scaled with the rate.
fn default_bitrate(settings: &ExportSettings) -> u32 {
    let pixels = f64::from(settings.resolution.width) * f64::from(settings.resolution.height);
    let fps = settings.frame_rate.as_f64().clamp(1.0, 120.0);
    (pixels * fps / 5.0).clamp(1_000_000.0, 120_000_000.0) as u32
}

impl WebExport {
    /// Start exporting `sequence` of `project` as `settings` asks, on the
    /// page's GPU. Only what a browser can make: an MP4 of H.264, with sound.
    pub fn start(
        project: Project,
        sequence: SequenceId,
        settings: ExportSettings,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, ExportError> {
        let unsupported = if settings.sound_only {
            None
        } else if settings.gif {
            Some("a GIF")
        } else if settings.image_sequence {
            Some("a folder of pictures")
        } else if settings.transparent {
            Some("a see-through video")
        } else if settings.prores || settings.codec != VideoCodec::H264 {
            Some("anything but an H.264 MP4")
        } else {
            None
        };
        if let Some(what) = unsupported {
            return Err(ExportError::Write(format!(
                "{what} — in a browser, export makes MP4 (H.264) and WAV files for now"
            )));
        }

        let Some(found) = project.sequence(sequence) else {
            return Err(ExportError::EmptyRange);
        };
        let mut sequence = found.clone();
        // Bakes are files the browser never makes; and at another size the
        // edit is composited again, as on the desktop.
        sequence.renders.clear();

        let Some(ticks_per_frame) = ticks_per_frame(settings.frame_rate).filter(|t| *t > 0) else {
            return Err(ExportError::EmptyRange);
        };
        let span = settings.range.end.ticks() - settings.range.start.ticks();
        if span <= 0 {
            return Err(ExportError::EmptyRange);
        }
        let total = ((span + ticks_per_frame - 1) / ticks_per_frame).max(1) as u64;
        let channels = if settings.sound_only {
            if !sequence.audio_tracks.iter().any(|track| track.enabled) {
                return Err(ExportError::NoSound);
            }
            usize::from(crate::wav::CHANNELS)
        } else if settings.picture_only {
            0
        } else {
            audio_channels(&sequence)
        };

        let mut compositor = Compositor::new(
            device.clone(),
            queue.clone(),
            RenderConfig::export_to_texture(settings.resolution),
        )?;
        bettercut_playback::load_luts(&project, &mut std::collections::HashSet::new(), |id, lut| {
            compositor.load_lut(id, lut);
        });
        let readback = Readback::new(&device, settings.resolution);
        let mut frames = FrameSource::new(1);
        frames.wait_for_exact_frames();
        let mut audio = AudioMixdown::new(&project, &sequence, channels);
        if let Some(target) = settings.loudness_target
            && channels > 0
        {
            audio.normalise_to(settings.range, target, &NeverCancelled);
        }

        // Sound only: counted in blocks, and no encoders to wait for.
        let (total, stage) = if settings.sound_only {
            let writer = crate::wav::WavWriter::new(std::io::Cursor::new(Vec::new()))
                .map_err(|err| ExportError::Write(err.to_string()))?;
            let samples = span / TICKS_PER_AUDIO_SAMPLE;
            let blocks = (samples + crate::wav::BLOCK - 1) / crate::wav::BLOCK;
            (blocks.max(1) as u64, Stage::Sound(Box::new(writer)))
        } else {
            let bitrate = settings
                .bitrate
                .map_or_else(|| default_bitrate(&settings), |b| b.clamp(100_000, 200_000_000) as u32);
            encoders::start(
                settings.resolution.width,
                settings.resolution.height,
                settings.frame_rate,
                bitrate,
                channels as u32,
            );
            (total, Stage::Starting)
        };
        let key_every = (settings.frame_rate.as_f64() * KEYFRAME_SECONDS).round().max(1.0) as u64;

        Ok(Self {
            project,
            sequence,
            settings,
            device,
            queue,
            compositor,
            readback,
            frames,
            titles: TextFrames::new(),
            audio,
            channels,
            ticks_per_frame,
            total,
            index: 0,
            samples_written: 0,
            key_every,
            stage,
        })
    }

    /// The file's name, as the download will be called.
    pub fn name(&self) -> String {
        self.settings
            .path
            .file_name()
            .map_or_else(|| "video.mp4".to_owned(), |n| n.to_string_lossy().into_owned())
    }

    /// Stop, and throw away what was made.
    pub fn cancel(&mut self) {
        if matches!(self.stage, Stage::Sound(_)) {
            self.stage = Stage::Over(WebProgress::Failed("cancelled".to_owned()));
            return;
        }
        encoders::cancel();
        self.stage = Stage::Over(WebProgress::Failed("cancelled".to_owned()));
    }

    /// Do what is ready, for up to [`STEP_BUDGET_MS`], and say where things are.
    pub fn step(&mut self) -> WebProgress {
        let began = Instant::now();
        loop {
            match self.advance() {
                Ok(true) if began.elapsed().as_millis() < STEP_BUDGET_MS => {}
                Ok(_) => break,
                Err(err) => {
                    encoders::cancel();
                    self.stage = Stage::Over(WebProgress::Failed(err.to_string()));
                    break;
                }
            }
        }
        match &self.stage {
            Stage::Over(progress) => progress.clone(),
            _ => WebProgress::Working {
                done: self.index,
                total: self.total,
            },
        }
    }

    /// One move forward. `Ok(true)` when there may be another to make now,
    /// `Ok(false)` when what comes next is waiting on the browser.
    fn advance(&mut self) -> Result<bool, ExportError> {
        match &self.stage {
            Stage::Starting => match encoders::state() {
                // A long video's sound still arriving: exported once it all has.
                encoders::State::Ready if browser::sound_coming() => Ok(false),
                encoders::State::Ready => {
                    if self.channels > 0 && encoders::sound_codec().is_none() {
                        tracing::warn!("this browser cannot encode sound; exporting the picture only");
                        self.channels = 0;
                    }
                    self.stage = Stage::Rendering;
                    Ok(true)
                }
                encoders::State::Failed(why) => Err(ExportError::Write(why)),
                _ => Ok(false),
            },
            Stage::Rendering => {
                if !encoders::has_room() {
                    return Ok(false);
                }
                let position = self.position(self.index);
                let requests = layer_requests(&self.project, &self.sequence, position);
                let Some(decoded) = decode_layers(
                    &self.project,
                    &self.sequence,
                    &requests,
                    SeekMode::Precise,
                    &mut self.frames,
                    &mut self.titles,
                    &NeverCancelled,
                    true,
                ) else {
                    return Ok(false);
                };
                composite_layers(&self.sequence, position, &decoded, &mut self.compositor)?;
                let mapped = self
                    .readback
                    .begin(&self.device, &self.queue, self.compositor.target());
                self.stage = Stage::Reading(mapped);
                Ok(true)
            }
            Stage::Reading(mapped) => {
                if !mapped.load(Ordering::Acquire) {
                    // Native wgpu needs asking; a browser answers on its own.
                    let _ = self.device.poll(wgpu::PollType::Poll);
                    return Ok(false);
                }
                let rgba = self.readback.finish();
                encoders::frame(
                    &rgba,
                    self.settings.resolution.width,
                    self.settings.resolution.height,
                    self.index as u32,
                    self.index.is_multiple_of(self.key_every),
                );
                self.push_sound();
                self.index += 1;
                if self.index == self.total {
                    encoders::finish();
                    self.stage = Stage::Finishing;
                } else {
                    self.stage = Stage::Rendering;
                }
                Ok(true)
            }
            Stage::Finishing => match encoders::state() {
                encoders::State::Done => {
                    let encoded = encoders::take().ok_or_else(|| {
                        ExportError::Write("the browser's encoders gave nothing back".to_owned())
                    })?;
                    let file = self.write(encoded);
                    let name = self.name();
                    browser::download(&file, &name, "video/mp4");
                    tracing::info!(name, bytes = file.len(), frames = self.total, "web export finished");
                    self.stage = Stage::Over(WebProgress::Finished {
                        name,
                        frames: self.total,
                    });
                    Ok(false)
                }
                encoders::State::Failed(why) => Err(ExportError::Write(why)),
                _ => Ok(false),
            },
            Stage::Sound(_) => self.mix_block(),
            Stage::Over(_) => Ok(false),
        }
    }

    /// Sound only: the next block of the mix into the WAV, and the WAV
    /// downloaded after the last.
    fn mix_block(&mut self) -> Result<bool, ExportError> {
        let start = self.settings.range.start.ticks();
        let samples = (self.settings.range.end.ticks() - start) / TICKS_PER_AUDIO_SAMPLE;
        let frames = (samples - self.samples_written).min(crate::wav::BLOCK);
        let write_failed = |err: std::io::Error| ExportError::Write(err.to_string());
        if frames > 0 {
            let position =
                TimelineTime::from_ticks(start + self.samples_written * TICKS_PER_AUDIO_SAMPLE);
            let planes = self.audio.read(position, frames as usize);
            let interleaved: Vec<f32> = (0..frames as usize)
                .flat_map(|i| planes.iter().map(move |plane| plane[i]))
                .collect();
            if let Stage::Sound(writer) = &mut self.stage {
                writer.push(&interleaved).map_err(write_failed)?;
            }
            self.samples_written += frames;
            self.index += 1;
        }
        if self.samples_written < samples {
            return Ok(true);
        }
        let Stage::Sound(writer) = std::mem::replace(&mut self.stage, Stage::Finishing) else {
            return Ok(false);
        };
        let file = writer.finish().map_err(write_failed)?.into_inner();
        let name = self.name();
        browser::download(&file, &name, "audio/wav");
        tracing::info!(name, bytes = file.len(), "web sound export finished");
        self.stage = Stage::Over(WebProgress::Finished { name, frames: 0 });
        Ok(false)
    }

    fn position(&self, index: u64) -> TimelineTime {
        TimelineTime::from_ticks(
            self.settings.range.start.ticks() + index as i64 * self.ticks_per_frame,
        )
    }

    /// The sound up to the end of the frame just given — counted from ticks,
    /// as the desktop counts it, so the two never drift apart.
    fn push_sound(&mut self) {
        if self.channels == 0 {
            return;
        }
        let start = self.settings.range.start.ticks();
        let frame_end = start + (self.index as i64 + 1) * self.ticks_per_frame;
        let target = (frame_end - start) / TICKS_PER_AUDIO_SAMPLE;
        let wanted = (target - self.samples_written).max(0) as usize;
        if wanted > 0 {
            let block = self.audio.read(self.position(self.index), wanted);
            encoders::sound(&block);
            self.samples_written = target;
        }
    }

    /// What the encoders made, as an MP4.
    fn write(&self, encoded: encoders::Encoded) -> Vec<u8> {
        let rate = self.settings.frame_rate.as_rational();
        let mut tracks = vec![Mp4Track {
            kind: Mp4TrackKind::Video {
                width: self.settings.resolution.width,
                height: self.settings.resolution.height,
                avcc: encoded.video_description,
            },
            timescale: rate.num() as u32,
            samples: encoded
                .video
                .into_iter()
                .map(|(data, sync)| Mp4Sample {
                    data,
                    duration: rate.den() as u32,
                    sync,
                })
                .collect(),
        }];
        if let Some(sound) = encoded.sound {
            let channels = sound.channels.max(1);
            let kind = if sound.codec == "aac" {
                Mp4TrackKind::Aac {
                    channels,
                    sample_rate: SOUND_RATE,
                    // AAC-LC at 48 kHz, when the browser does not say.
                    config: sound
                        .description
                        .unwrap_or_else(|| vec![0x11, 0x80 | ((channels as u8 & 0x0f) << 3)]),
                    bitrate: 192_000,
                }
            } else {
                Mp4TrackKind::Opus {
                    channels,
                    // From the OpusHead the browser gives, or the encoder's
                    // usual 6.5 ms.
                    pre_skip: sound
                        .description
                        .as_deref()
                        .filter(|head| head.len() >= 12 && head.starts_with(b"OpusHead"))
                        .map_or(312, |head| u16::from_le_bytes([head[10], head[11]])),
                }
            };
            tracks.push(Mp4Track {
                kind,
                timescale: SOUND_RATE,
                samples: sound
                    .chunks
                    .into_iter()
                    .map(|(data, micros)| Mp4Sample {
                        data,
                        duration: (micros * f64::from(SOUND_RATE) / 1e6).round() as u32,
                        sync: true,
                    })
                    .collect(),
            });
        }
        mux(&tracks)
    }
}
