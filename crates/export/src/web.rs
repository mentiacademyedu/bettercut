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
//! block a step; a GIF is the same frames given to the desktop's own GIF
//! writer rather than the browser's encoders.

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
use bettercut_timeline::{ColorAdjust, Sequence};

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
    /// A GIF being written, instead of the browser's encoders.
    gif: Option<Box<crate::gif::GifWriter<std::io::Cursor<Vec<u8>>>>>,
    /// A bounce: the WAV is given to the page as a new file, not downloaded.
    to_page: bool,
}

/// A contact sheet in a browser: a frame per tile, each rendered as a
/// [`WebStill`] is, laid out as the desktop lays one out.
pub struct WebSheet {
    still: WebStill,
    sheet: Option<crate::contact_sheet::Sheet>,
    instants: Vec<TimelineTime>,
    index: usize,
}

impl WebSheet {
    pub fn start(
        project: Project,
        sequence: SequenceId,
        settings: &crate::SheetSettings,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, ExportError> {
        let instants = settings.instants();
        let Some(first) = instants.first().copied() else {
            return Err(ExportError::EmptyRange);
        };
        let Some(found) = project.sequence(sequence) else {
            return Err(ExportError::EmptyRange);
        };
        let sheet = crate::contact_sheet::Sheet::new(settings, found.resolution);
        Ok(Self {
            still: WebStill::start(project, sequence, first, device, queue)?,
            sheet: Some(sheet),
            instants,
            index: 0,
        })
    }

    /// How many tiles are done, of how many.
    pub fn progress(&self) -> (usize, usize) {
        (self.index, self.instants.len())
    }

    /// Do what is ready. The finished sheet as a PNG's bytes once every tile
    /// is in; `None` while tiles are still coming.
    pub fn step(&mut self) -> Option<Result<Vec<u8>, ExportError>> {
        let (_, _, rgba) = match self.still.step()? {
            Ok(frame) => frame,
            Err(err) => return Some(Err(err)),
        };
        if let Some(sheet) = self.sheet.as_mut() {
            sheet.place(self.index, &rgba);
        }
        self.index += 1;
        if let Some(next) = self.instants.get(self.index) {
            self.still.move_to(*next);
            return None;
        }
        let (size, pixels) = self.sheet.take()?.finish();
        Some(crate::png_bytes(size, &pixels))
    }
}

/// Auto Level or a colour match in a browser: the clip's own frame decoded,
/// and for a match the frame under the playhead rendered without the clip,
/// each when it is ready, then the same maths as the desktop's job.
pub struct WebGrade {
    asset: bettercut_media::MediaAsset,
    at: bettercut_foundation::MediaTime,
    frames: FrameSource,
    sample: Option<bettercut_timeline::colour_match::FrameSample>,
    reference: Option<WebStill>,
    reference_stats: Option<bettercut_timeline::colour_match::FrameStats>,
}

impl WebGrade {
    /// Auto Level `clip`: a neutral grade from its own middle frame.
    pub fn auto_level(project: &Project, sequence: SequenceId, clip: bettercut_foundation::ClipId) -> Result<Self, String> {
        let (asset, at) = crate::colour_match::clip_middle(project, sequence, clip)?;
        let mut frames = FrameSource::new(1);
        frames.wait_for_exact_frames();
        Ok(Self {
            asset,
            at,
            frames,
            sample: None,
            reference: None,
            reference_stats: None,
        })
    }

    /// Match `clip` to the frame at `at`, rendered without the clip.
    pub fn colour_match(
        project: &Project,
        sequence: SequenceId,
        clip: bettercut_foundation::ClipId,
        at: TimelineTime,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, String> {
        let without = crate::colour_match::without_clip(project, sequence, clip)?;
        let mut copy = project.clone();
        if let Some(slot) = copy.sequences.iter_mut().find(|s| s.id == sequence) {
            *slot = without;
        }
        let reference =
            WebStill::start(copy, sequence, at, device, queue).map_err(|e| e.to_string())?;
        let mut grade = Self::auto_level(project, sequence, clip)?;
        grade.reference = Some(reference);
        Ok(grade)
    }

    /// Do what is ready. The grade once both frames are in; `None` while
    /// either is still coming.
    pub fn step(&mut self) -> Option<Result<ColorAdjust, String>> {
        if self.sample.is_none() {
            match self.frames.decode(&self.asset, self.at, SeekMode::Precise, &NeverCancelled) {
                Ok(frame) => match crate::colour_match::sample_of(&frame) {
                    Ok(sample) => self.sample = Some(sample),
                    Err(err) => return Some(Err(err)),
                },
                Err(bettercut_playback::PlaybackError::Media(err)) if browser::is_not_yet(&err) => {}
                Err(err) => return Some(Err(err.to_string())),
            }
        }
        if let Some(reference) = self.reference.as_mut()
            && self.reference_stats.is_none()
        {
            match reference.step()? {
                Ok((width, height, rgba)) => {
                    let Some(sample) =
                        bettercut_timeline::colour_match::FrameSample::from_rgba(&rgba, width, height)
                    else {
                        return Some(Err("the frame under the playhead could not be read".to_owned()));
                    };
                    let stats = sample.stats();
                    if stats.luma < crate::colour_match::EMPTY_REFERENCE {
                        return Some(Err("there is no picture under the playhead to match to".to_owned()));
                    }
                    self.reference_stats = Some(stats);
                }
                Err(err) => return Some(Err(err.to_string())),
            }
        }
        let sample = self.sample.as_ref()?;
        Some(Ok(match &self.reference_stats {
            Some(reference) => bettercut_timeline::colour_match::match_grade(sample, reference),
            None if self.reference.is_none() => bettercut_timeline::colour_match::auto_grade(sample),
            None => return None,
        }))
    }
}

/// The H.264 setup (an `avcC` record) the MP4 carries: the encoder's own when
/// it holds together, else one built from the parameter sets in the first
/// frame. Firefox's has been seen with the SPS's first byte doubled, which a
/// player reads as an SPS that does not parse; its frames carry good ones.
fn avc_config(description: Vec<u8>, first_frame: Option<&[u8]>) -> Vec<u8> {
    if avc_config_holds(&description) {
        return description;
    }
    let Some(frame) = first_frame else {
        return description;
    };
    // The frame's NAL units, each behind a 4-byte length.
    let (mut sps, mut pps) = (None, None);
    let mut at = 0;
    while at + 4 <= frame.len() {
        let length = u32::from_be_bytes([frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]) as usize;
        let Some(unit) = frame.get(at + 4..at + 4 + length) else {
            break;
        };
        match unit.first().map(|header| header & 0x1f) {
            Some(7) if sps.is_none() => sps = Some(unit),
            Some(8) if pps.is_none() => pps = Some(unit),
            _ => {}
        }
        at += 4 + length;
    }
    let (Some(sps), Some(pps)) = (sps, pps) else {
        return description;
    };
    if sps.len() < 4 {
        return description;
    }
    let mut config = vec![1, sps[1], sps[2], sps[3], 0xFF, 0xE1];
    config.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    config.extend_from_slice(sps);
    config.push(1);
    config.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    config.extend_from_slice(pps);
    config
}

/// Whether an `avcC` record's first SPS is an SPS that agrees with the
/// record's own profile and level.
fn avc_config_holds(config: &[u8]) -> bool {
    if config.len() < 8 || config[0] != 1 || config[5] & 0x1f == 0 {
        return false;
    }
    let length = u16::from_be_bytes([config[6], config[7]]) as usize;
    let Some(sps) = config.get(8..8 + length) else {
        return false;
    };
    sps.len() >= 4 && sps[0] & 0x1f == 7 && sps[1..4] == config[1..4]
}

/// A rendered frame: width, height, and tightly packed RGBA.
pub type StillFrame = (u32, u32, Vec<u8>);

/// One frame rendered in a browser — to save, to copy, for the trim window
/// or the scopes — the frame an export would make there, stepped as an
/// export is, since nothing here may wait for the decoders or the GPU.
pub struct WebStill {
    project: Project,
    sequence: Sequence,
    position: TimelineTime,
    device: wgpu::Device,
    queue: wgpu::Queue,
    compositor: Compositor,
    readback: Readback,
    frames: FrameSource,
    titles: TextFrames,
    mapped: Option<Arc<AtomicBool>>,
}

impl WebStill {
    /// Start rendering `sequence` of `project` at `position`, full size.
    pub fn start(
        project: Project,
        sequence: SequenceId,
        position: TimelineTime,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, ExportError> {
        let Some(found) = project.sequence(sequence) else {
            return Err(ExportError::EmptyRange);
        };
        let mut sequence = found.clone();
        // Bakes are files the browser never makes.
        sequence.renders.clear();
        let resolution = sequence.resolution;
        let mut compositor = Compositor::new(
            device.clone(),
            queue.clone(),
            RenderConfig::export_to_texture(resolution),
        )?;
        bettercut_playback::load_luts(&project, &mut std::collections::HashSet::new(), |id, lut| {
            compositor.load_lut(id, lut);
        });
        let readback = Readback::new(&device, resolution);
        let mut frames = FrameSource::new(1);
        frames.wait_for_exact_frames();
        Ok(Self {
            project,
            sequence,
            position,
            device,
            queue,
            compositor,
            readback,
            frames,
            titles: TextFrames::new(),
            mapped: None,
        })
    }

    /// Render another instant with the same renderer: what the next
    /// [`Self::step`]s make.
    pub fn move_to(&mut self, position: TimelineTime) {
        self.position = position;
        self.mapped = None;
    }

    /// Do what is ready. The frame, as width, height and tightly packed
    /// RGBA, once it is here; `None` while it is still coming.
    pub fn step(&mut self) -> Option<Result<StillFrame, ExportError>> {
        let resolution = self.sequence.resolution;
        if let Some(mapped) = &self.mapped {
            if !mapped.load(Ordering::Acquire) {
                let _ = self.device.poll(wgpu::PollType::Poll);
                return None;
            }
            return Some(Ok((resolution.width, resolution.height, self.readback.finish())));
        }
        let requests = layer_requests(&self.project, &self.sequence, self.position);
        let decoded = decode_layers(
            &self.project,
            &self.sequence,
            &requests,
            SeekMode::Precise,
            &mut self.frames,
            &mut self.titles,
            &NeverCancelled,
            true,
        )?;
        if let Err(err) = composite_layers(&self.sequence, self.position, &decoded, &mut self.compositor) {
            return Some(Err(err));
        }
        self.mapped = Some(
            self.readback
                .begin(&self.device, &self.queue, self.compositor.target()),
        );
        None
    }
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
        let unsupported = if settings.sound_only || settings.gif {
            None
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
                "{what} — in a browser, export makes MP4 (H.264), GIF and WAV files for now"
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
        } else if settings.picture_only || settings.gif {
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

        // A GIF: its own writer, and no encoders to wait for.
        let gif = if settings.gif {
            let writer = crate::gif::GifWriter::new(
                std::io::Cursor::new(Vec::new()),
                settings.resolution.width.min(u32::from(u16::MAX)) as u16,
                settings.resolution.height.min(u32::from(u16::MAX)) as u16,
            )
            .map_err(|err| ExportError::Write(err.to_string()))?;
            Some(Box::new(writer))
        } else {
            None
        };
        // Sound only: counted in blocks, and no encoders to wait for.
        let (total, stage) = if gif.is_some() {
            (total, Stage::Rendering)
        } else if settings.sound_only {
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
            gif,
            to_page: false,
        })
    }

    /// Bounce `track`: its mix, alone, as a WAV called `name`, given to the
    /// page as a new file once made — `WebProgress::Finished` names the path
    /// the page knows it by. Where it starts on the timeline, with it.
    pub fn bounce(
        project: Project,
        sequence: SequenceId,
        track: bettercut_foundation::TrackId,
        name: &str,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<(Self, TimelineTime), ExportError> {
        let range =
            crate::bounce::track_span(&project, sequence, track).ok_or(ExportError::NoSound)?;
        let project =
            crate::bounce::only_this_lane(project, sequence, track).ok_or(ExportError::NoSound)?;
        let Some(found) = project.sequence(sequence) else {
            return Err(ExportError::EmptyRange);
        };
        let mut settings = ExportSettings::for_sequence(std::path::PathBuf::from(name), found);
        settings.sound_only = true;
        settings.range = range;
        let mut export = Self::start(project, sequence, settings, device, queue)?;
        export.to_page = true;
        Ok((export, range.start))
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
        if matches!(self.stage, Stage::Sound(_)) || self.gif.is_some() {
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
                if self.gif.is_none() && !encoders::has_room() {
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
                if let Some(gif) = self.gif.as_mut() {
                    let delay = crate::gif::frame_delay(self.index, self.ticks_per_frame);
                    gif.push(&rgba, delay)
                        .map_err(|err| ExportError::Write(err.to_string()))?;
                    self.index += 1;
                    return if self.index == self.total {
                        self.finish_gif()
                    } else {
                        self.stage = Stage::Rendering;
                        Ok(true)
                    };
                }
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

    /// The GIF finished and downloaded.
    fn finish_gif(&mut self) -> Result<bool, ExportError> {
        let Some(gif) = self.gif.take() else {
            return Ok(false);
        };
        let file = gif
            .finish()
            .map_err(|err| ExportError::Write(err.to_string()))?
            .into_inner();
        let name = self.name();
        browser::download(&file, &name, "image/gif");
        tracing::info!(name, bytes = file.len(), frames = self.total, "web GIF export finished");
        self.stage = Stage::Over(WebProgress::Finished {
            name,
            frames: self.total,
        });
        Ok(false)
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
        let mut name = self.name();
        if self.to_page {
            name = browser::give_file(&file, &name, "audio/wav")
                .to_string_lossy()
                .into_owned();
        } else {
            browser::download(&file, &name, "audio/wav");
        }
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
                avcc: avc_config(
                    encoded.video_description,
                    encoded.video.first().map(|(data, _)| data.as_slice()),
                ),
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
