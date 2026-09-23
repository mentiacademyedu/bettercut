//! Export: the timeline, rendered to a file (Milestone 6, §60 criterion 10).
//!
//! ## The same graph, the other configuration
//!
//! §46 is the rule this crate exists to honour: *there is exactly one render
//! graph implementation, and preview and export are two configurations of it.*
//! So there is no export renderer here. There is a [`bettercut_renderer::
//! Compositor`] built with [`RenderConfig::export_to_texture`], fed by the same
//! [`bettercut_playback::layer_requests`] the preview calls, drawing the same
//! effect nodes at the full quality tier.
//!
//! What actually differs is the four things §46 lists: the resolution, the
//! media (originals, never proxies — §14), the effect tier, and where the
//! pixels go.
//!
//! ## Why a second GPU device
//!
//! The compositor is one implementation, but export runs on a worker thread and
//! must not touch the device egui is drawing with — §2 and §74 both say the UI
//! must never stall behind FFmpeg, and sharing a queue would put every exported
//! frame in front of the next repaint. A second `wgpu::Device` is a second
//! *handle to the same hardware*, not a second renderer.
//!
//! ## Reading sequentially, on purpose
//!
//! Export walks the timeline forward, which is exactly §47a.2's `Playback`
//! mode: never seek, read the next frame. Asking for each frame with a seek is
//! what made playback quadratic, and an export is the longest forward walk the
//! program ever does — the same mistake here would be the same mistake, times
//! the length of the video.
//!
//! ## Timing is integer throughout
//!
//! Frame boundaries and audio sample counts both come from §9 ticks. The number
//! of samples belonging to frame N is derived from the *tick* positions of its
//! edges rather than by accumulating a per-frame float, so a ten-minute export
//! at 29.97 ends with the audio exactly where it started, not 40 ms adrift.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::PathBuf;

use bettercut_foundation::{FrameRate, TICKS_PER_AUDIO_SAMPLE, TimelineTime, ticks_per_frame};
use bettercut_media::{
    CancellationToken, EncodeTarget, ExportFormat, SeekMode, VideoWriter, probe_all,
};
/// Re-exported so callers choose a codec without depending on the media crate.
pub use bettercut_media::{RateControl, VideoCodec};
use bettercut_playback::{
    AudioMixer, AudioPlan, FrameSource, LayerSource, TextFrames, layer_requests, layer_transform,
};
use bettercut_project_format::Project;
use bettercut_renderer::{Compositor, Layer, RenderConfig, wgpu};
use bettercut_timeline::{Resolution, Sequence, TimelineRange};

mod bounce;
mod colour_match;
mod contact_sheet;
mod error;
mod frames;
mod gif;
mod job;
mod readback;
mod still;
mod wav;

pub use bounce::{BounceJob, only_this_lane, track_span};
pub use colour_match::{ColourMatchJob, MatchedGrade, auto_level, colour_match};
pub use contact_sheet::{ContactSheetJob, MAX_TILES, SheetSettings, contact_sheet, shrink};
pub use error::ExportError;
pub use frames::{FRAMES_ENCODER, export_frames, frame_file, frames_folder};
pub use gif::{GIF_ENCODER, GifWriter, MAX_COLOURS, export_gif, frame_delay, quantize};
pub use job::{ExportJob, Outcome};
pub use still::{FrameGrabJob, RenderedFrame, StillJob, render_still, save_still, write_png};
pub use wav::{WAV_ENCODER, WavWriter, export_sound};

/// What to export, and where.
#[derive(Debug, Clone)]
pub struct ExportSettings {
    pub path: PathBuf,
    /// Defaults to the sequence's own format; overridable for a smaller file.
    pub resolution: Resolution,
    /// Defaults to the sequence's rate. A different rate resamples by picking
    /// the frame visible at each new instant — positions are absolute ticks
    /// (§9), so there is nothing to convert, only somewhere else to sample.
    pub frame_rate: FrameRate,
    pub codec: VideoCodec,
    /// Video bits per second, or `None` for one derived from the format.
    pub bitrate: Option<i64>,
    pub rate_control: RateControl,
    /// The span of the timeline to write. Usually the whole sequence.
    pub range: TimelineRange,
    /// §15.1: FFmpeg never gets every core, not even when it is the only job.
    pub threads: u32,
    /// Write only the sound, as a WAV at `path`; the video settings are unused
    /// and no GPU is opened (`wav.rs`).
    pub sound_only: bool,
    /// Write a looping animated GIF at `path` (`gif.rs`): the picture at
    /// `resolution` and `frame_rate`, no sound, and no codec or bitrate.
    pub gif: bool,
    /// Write every frame as a numbered PNG in a folder named after `path`
    /// (`frames.rs`): the picture at `resolution` and `frame_rate`, no sound.
    pub image_sequence: bool,
    /// Bring the mix to this loudness, in LUFS, before writing it: measured
    /// over the whole range first, then gained by the difference
    /// (`bettercut_audio::loudness::gain_for`), then limited as always. `None`
    /// writes the mix at the level it was mixed.
    pub loudness_target: Option<f32>,
    /// The sound's bits per second in the file, or `None` for the default
    /// (`bettercut_media::DEFAULT_AUDIO_BITRATE`).
    pub audio_bitrate: Option<i64>,
    /// Write the picture with no sound stream at all.
    ///
    /// What render in place bakes (`bettercut_timeline::render`): a bake
    /// stands in for the *picture* of a stretch, and the sound is mixed live
    /// from the edit as always — so re-cutting the music costs nothing, and a
    /// bake that carried sound would only be a second copy of it to keep in
    /// step.
    pub picture_only: bool,

    /// Write a video with a see-through background at `path` (a `.webm`):
    /// VP9 with alpha, no sound. What no clip covers is transparent.
    pub transparent: bool,
}

impl ExportSettings {
    /// The sequence's own format, which is what most exports want.
    pub fn for_sequence(path: PathBuf, sequence: &Sequence) -> Self {
        Self {
            path,
            resolution: sequence.resolution,
            frame_rate: sequence.frame_rate,
            codec: VideoCodec::default(),
            bitrate: None,
            rate_control: RateControl::default(),
            range: TimelineRange {
                start: TimelineTime::ZERO,
                end: sequence.duration(),
            },
            threads: 2,
            sound_only: false,
            picture_only: false,
            gif: false,
            image_sequence: false,
            loudness_target: None,
            audio_bitrate: None,
            transparent: false,
        }
    }
}

/// Whether this machine can write `codec` at this size and rate.
///
/// Asked by the interface before offering the choice, because H.265 has no
/// software fallback — x265 is GPL and §0.1 forbids linking it — so on a
/// machine without a GPU encoder the option would be a promise we cannot keep.
/// Opening an encoder is the only honest test; it costs a few milliseconds and
/// belongs at the moment the dialog opens, not per frame.
pub fn codec_is_available(
    codec: VideoCodec,
    resolution: Resolution,
    frame_rate: FrameRate,
) -> bool {
    // H.264 needs no probing: `libopenh264` is linked into the binary and is
    // always a candidate, so the answer is yes on every machine. Asking anyway
    // would spend a slow `LoadLibrary` on the vendor runtimes to learn nothing.
    if codec == VideoCodec::H264 {
        return true;
    }

    let width = resolution.width.max(2) & !1;
    let height = resolution.height.max(2) & !1;
    let ratio = frame_rate.as_rational();
    let key = (codec, width, height, ratio.num(), ratio.den());

    // Cached for the process. The answer is a fact about the hardware and its
    // drivers, so it cannot change while the program runs — and the asking is
    // expensive: opening every candidate takes about half a second here,
    // most of it a failed `amfrt64.dll` load on a machine with no AMD card.
    // This runs when the export window opens, on the UI thread, and §74 is
    // explicit that the interface must not stall behind that.
    static CACHE: std::sync::Mutex<Option<Vec<(ProbeKey, bool)>>> = std::sync::Mutex::new(None);

    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entries = cache.get_or_insert_with(Vec::new);
    if let Some((_, usable)) = entries.iter().find(|(cached, _)| *cached == key) {
        return *usable;
    }

    let usable = probe_all(EncodeTarget {
        width,
        height,
        frame_rate,
        codec,
        bitrate: None,
        rate_control: RateControl::default(),
        threads: 1,
        global_header: true,
    })
    .iter()
    .any(bettercut_media::EncoderProbe::is_usable);

    entries.push((key, usable));
    usable
}

/// What a cached availability answer is keyed on: the codec and the exact
/// format, because an encoder can accept one size and refuse another.
type ProbeKey = (VideoCodec, u32, u32, i64, i64);

/// How far along an export is, for the interface to show.
#[derive(Debug, Clone, Copy)]
pub struct ExportProgress {
    pub frames_done: u64,
    pub frames_total: u64,
    /// The encoder actually running, so "why was that fast" has an answer.
    pub encoder: &'static str,
}

impl ExportProgress {
    /// 0.0 to 1.0. Zero total reads as complete rather than as a division by
    /// zero, because an empty range is finished the moment it starts.
    pub fn fraction(self) -> f32 {
        if self.frames_total == 0 {
            return 1.0;
        }
        (self.frames_done as f32 / self.frames_total as f32).clamp(0.0, 1.0)
    }
}

/// What an export produced.
#[derive(Debug, Clone)]
pub struct ExportSummary {
    pub path: PathBuf,
    pub frames: u64,
    pub encoder: &'static str,
    pub hardware: bool,
}

/// Render `settings.range` of `sequence` into a file.
///
/// Blocking, and meant to be called on a worker thread — §74: never block the
/// UI while FFmpeg runs. `cancel` is checked once per frame, so stopping is
/// prompt without leaving a half-written file the user might mistake for a
/// finished one: a cancelled export deletes its output.
pub fn export(
    project: &Project,
    sequence: &Sequence,
    settings: &ExportSettings,
    on_progress: &mut dyn FnMut(ExportProgress),
    cancel: &dyn CancellationToken,
) -> Result<ExportSummary, ExportError> {
    // A bake is written at the sequence's own size (`timeline::render`).
    // Asked for a different size, the export composites the edit again rather
    // than scaling a finished picture up: §46 promises the same picture as the
    // preview, and a soft one is not it. The copy costs nothing — a sequence
    // is references and numbers, never media.
    let full_size;
    let sequence = if settings.resolution == sequence.resolution || sequence.renders.is_empty() {
        sequence
    } else {
        let mut copy = sequence.clone();
        copy.renders.clear();
        full_size = copy;
        &full_size
    };

    if settings.sound_only {
        return export_sound(project, sequence, settings, on_progress, cancel);
    }
    if settings.gif {
        return export_gif(project, sequence, settings, on_progress, cancel);
    }
    if settings.image_sequence {
        return export_frames(project, sequence, settings, on_progress, cancel);
    }

    // The *output* rate, which need not be the sequence's: a 60 fps timeline
    // exported at 30 samples every other instant, and positions are absolute
    // ticks so there is nothing to convert (§9).
    let Some(ticks_per_frame) = ticks_per_frame(settings.frame_rate).filter(|t| *t > 0) else {
        return Err(ExportError::EmptyRange);
    };
    let span = settings.range.end.ticks() - settings.range.start.ticks();
    if span <= 0 {
        return Err(ExportError::EmptyRange);
    }

    // Frames that start inside the range. A frame beginning one tick before the
    // end is a frame the user asked for; one beginning at the end is not.
    let total_frames = (span + ticks_per_frame - 1) / ticks_per_frame;
    let total_frames = total_frames.max(1) as u64;
    let channels = if settings.picture_only {
        0
    } else {
        audio_channels(sequence)
    };

    let (device, queue) = open_device()?;
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        // §46's export configuration: full tier, original media, texture sink.
        RenderConfig::export_to_texture(settings.resolution),
    )?;
    let mut readback = readback::Readback::new(&device, settings.resolution);
    compositor.set_transparent(settings.transparent);
    // Every table the project names, once, before the first frame.
    bettercut_playback::load_luts(project, &mut std::collections::HashSet::new(), |id, lut| {
        compositor.load_lut(id, lut);
    });

    // The sequence's chapters, cut to the exported stretch and re-based to
    // its start, so a player's list matches the file it is playing. Only
    // when there is a mark to make chapters of: an unmarked sequence would
    // otherwise carry one chapter called "Intro", which is noise.
    let chapters = chapter_marks(sequence, settings.range);
    let mut writer = VideoWriter::create_with_chapters(
        &settings.path,
        ExportFormat {
            transparent: settings.transparent,
            width: settings.resolution.width,
            height: settings.resolution.height,
            frame_rate: settings.frame_rate,
            codec: settings.codec,
            bitrate: settings.bitrate,
            rate_control: settings.rate_control,
            channels,
            audio_bitrate: settings.audio_bitrate,
            threads: settings.threads,
        },
        &chapters,
    )?;
    let encoder = writer.encoder();
    tracing::info!(
        encoder = encoder.name,
        frames = total_frames,
        "export started"
    );

    // §14: export reads originals. No proxy source is ever set on this pool,
    // which is the whole of that rule — a proxy here would export the 540p copy
    // the user was editing with.
    let mut frames = FrameSource::new(settings.threads);
    // §26.1: built here rather than borrowed from the preview, because an
    // export runs on a job thread while the editor keeps going. Same crate,
    // same parameters, same bitmap.
    let mut titles = TextFrames::new();
    let mut audio = AudioMixdown::new(project, sequence, channels);
    if let Some(target) = settings.loudness_target
        && channels > 0
        && let Some(measured) = audio.normalise_to(settings.range, target, cancel)
    {
        tracing::info!(measured, target, "export loudness normalised");
    }

    let mut samples_written: i64 = 0;

    for index in 0..total_frames {
        if cancel.is_cancelled() {
            // Drop the writer without finishing, then remove the file: a
            // cancelled export must not leave something that looks finished.
            drop(writer);
            let _ = std::fs::remove_file(&settings.path);
            return Err(ExportError::Cancelled);
        }

        let position =
            TimelineTime::from_ticks(settings.range.start.ticks() + index as i64 * ticks_per_frame);

        let rgba = render_frame(
            project,
            sequence,
            position,
            SeekMode::Playback,
            &mut frames,
            &mut titles,
            &mut compositor,
            &mut readback,
            &device,
            &queue,
            cancel,
        )?;
        if settings.transparent {
            let mut straight = rgba;
            unpremultiply(&mut straight);
            writer.push_frame(&straight)?;
        } else {
            writer.push_frame(&rgba)?;
        }

        // Samples up to the *end* of this frame, from ticks rather than from a
        // running float — see the module docs.
        let frame_end = settings.range.start.ticks() + (index as i64 + 1) * ticks_per_frame;
        let target = (frame_end - settings.range.start.ticks()) / TICKS_PER_AUDIO_SAMPLE;
        let wanted = (target - samples_written).max(0) as usize;
        if wanted > 0 && channels > 0 {
            let block = audio.read(position, wanted);
            writer.push_audio(&block)?;
            samples_written = target;
        }

        on_progress(ExportProgress {
            frames_done: index + 1,
            frames_total: total_frames,
            encoder: encoder.name,
        });
    }

    writer.finish()?;
    tracing::info!(path = %settings.path.display(), "export finished");

    Ok(ExportSummary {
        path: settings.path.clone(),
        frames: total_frames,
        encoder: encoder.name,
        hardware: encoder.kind.is_hardware(),
    })
}

/// Composite one instant and read it back as tightly packed RGBA.
#[allow(clippy::too_many_arguments)]
fn render_frame(
    project: &Project,
    sequence: &Sequence,
    position: TimelineTime,
    seek: SeekMode,
    frames: &mut FrameSource,
    titles: &mut TextFrames,
    compositor: &mut Compositor,
    readback: &mut readback::Readback,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cancel: &dyn CancellationToken,
) -> Result<Vec<u8>, ExportError> {
    // The same rule the preview uses (§46), not a second copy of it.
    let requests = layer_requests(project, sequence, position);

    let mut decoded = Vec::with_capacity(requests.len());
    for request in &requests {
        match request.source {
            LayerSource::Media(media) => {
                let Some(asset) = project.media_asset(media) else {
                    continue;
                };
                // §47a.2: an export is a forward walk (`Playback`, the next
                // frame wanted is the next in the file); a still is one instant
                // anywhere (`Precise`). The caller says which.
                let at = asset.frame_time(request.source_time);
                match frames.decode(asset, at, seek, cancel) {
                    Ok(frame) => decoded.push((request, frame)),
                    Err(err) => {
                        // §50: one unreadable file must not abort a long
                        // export. The clip is missing from that frame and the
                        // rest still renders.
                        tracing::warn!(
                            file = %asset.file_name,
                            %err,
                            "could not decode a frame for export; that layer is absent"
                        );
                    }
                }
            }
            // §25: generated, not read — and generated by the same function
            // the preview calls, so the flash in the file is the one on screen
            // (§46).
            LayerSource::Solid { rgb } => {
                decoded.push((
                    request,
                    std::sync::Arc::new(bettercut_playback::solid_frame(rgb)),
                ));
            }
            // §26.1: the same rasterizer the preview uses, so the exported
            // title is the one the user watched. Cached across frames, because
            // a three-second title is the same picture ninety times over.
            // The burn-in: built here from the same function the preview
            // calls, so the copy sent out says what the editor was looking at.
            LayerSource::BurnIn => {
                if let Some(clip) = bettercut_playback::engine::burn_in_clip(
                    project,
                    sequence,
                    TimelineTime::from_ticks(request.source_time.ticks()),
                ) && let Some(frame) = titles.frame_for(&clip, None)
                {
                    decoded.push((request, frame));
                }
            }
            LayerSource::Text(clip) => {
                if let Some(text) = sequence.text_clip(clip)
                    && let Some(frame) = titles.frame_at(
                        text,
                        request.reveal,
                        TimelineTime::from_ticks(request.source_time.ticks()),
                    )
                {
                    decoded.push((request, frame));
                }
            }
        }
    }

    let layers: Vec<Layer<'_>> = decoded
        .iter()
        .map(|(request, frame)| Layer {
            frame,
            // §26: the same function the preview calls, so the exported title
            // is the size the user watched.
            // Passed through whole rather than unpacked field by field: this
            // used to be six lines, and the preview had its own six. A look
            // that grew a seventh reached whichever list someone remembered
            // (§46). The transform is the one place the two paths legitimately
            // differ from the request, and they work it out with the same
            // shared function.
            look: bettercut_timeline::ClipLook {
                corner_pin: Default::default(),
                transform: layer_transform(request, frame, sequence.resolution),
                ..request.look
            },
        })
        .collect();

    // Adjustment lanes, graded over the pictures actually drawn. Both halves
    // come from the functions the preview calls, so an exported grade covers
    // exactly what the user watched it cover (§46).
    let beneath = bettercut_playback::graded_beneath(
        sequence,
        decoded.iter().map(|(request, _)| request.track),
    );
    let grades: Vec<bettercut_renderer::Grade> =
        bettercut_playback::adjustments_at(sequence, position)
            .into_iter()
            .map(|look| bettercut_renderer::Grade { beneath, look })
            .collect();

    // §46: the same adjustment the preview shows, on the same finished image.
    compositor.set_grain_seed(bettercut_playback::grain_seed(sequence, position));
    compositor.composite_graded(&layers, &grades, sequence.master)?;
    Ok(readback.read(device, queue, compositor.target()))
}

/// Audio channels the output should have.
///
/// Stereo when the sequence has any enabled audio track, none otherwise. A
/// silent export writes no audio stream at all rather than an empty one.
fn audio_channels(sequence: &Sequence) -> usize {
    if sequence.audio_tracks.iter().any(|track| track.enabled) {
        2
    } else {
        0
    }
}

/// Mixing the timeline's audio, one video frame at a time.
///
/// Through the same [`AudioMixer::mix_block`] the preview's mixer thread calls
/// (§46), so the exported audio is the audio that was played — the audio half
/// of .1's golden comparison. This used to be a copy of the preview's loop,
/// and the copy had already fallen behind: it did not resample a sped-up clip
///, so a 2× clip exported with its sound at normal speed, cut off half
/// way through.
struct AudioMixdown {
    mixer: AudioMixer,
    /// Taken once, at the start: an export renders a snapshot of the project,
    /// and the plan is that snapshot's audio half.
    plan: AudioPlan,
    channels: usize,
    /// The normalising gain, one unless `normalise_to` set it.
    gain: f32,
}

impl AudioMixdown {
    fn new(project: &Project, sequence: &Sequence, channels: usize) -> Self {
        Self {
            // One decoder thread: §15.1 keeps an export from taking every
            // core, and audio decode is not where an export spends its time.
            mixer: AudioMixer::new(1),
            plan: AudioPlan::of(project, sequence),
            channels,
            gain: 1.0,
        }
    }

    /// Bring the mix to `target` LUFS: the range is mixed once to measure it
    /// (the mix is deterministic, §46, so measuring it is playing it), and the
    /// difference becomes a gain on every block written after. Returns what
    /// was measured, or `None` for a range too quiet to measure, which is
    /// left as it is.
    fn normalise_to(
        &mut self,
        range: TimelineRange,
        target: f32,
        cancel: &dyn CancellationToken,
    ) -> Option<f32> {
        const BLOCK: usize = 4_800;
        let mut meter = bettercut_audio::loudness::LoudnessMeter::new(self.channels);
        let total = (range.duration().ticks() / TICKS_PER_AUDIO_SAMPLE).max(0) as usize;
        let mut done = 0;
        while done < total {
            if cancel.is_cancelled() {
                return None;
            }
            let frames = BLOCK.min(total - done);
            let position = TimelineTime::from_ticks(
                range.start.ticks() + done as i64 * TICKS_PER_AUDIO_SAMPLE,
            );
            let block = self.read_interleaved(position, frames);
            meter.push_interleaved(&block, self.channels);
            done += frames;
        }
        let measured = meter.integrated()?;
        self.gain = bettercut_audio::loudness::gain_for(measured, target);
        // A fresh mixer for the real pass: the measuring pass ran every
        // stateful stage forward, and the export must start from the top.
        self.mixer = AudioMixer::new(1);
        Some(measured)
    }

    fn read_interleaved(&mut self, position: TimelineTime, frames: usize) -> Vec<f32> {
        let mut interleaved = vec![0.0_f32; frames * self.channels];
        self.mixer.mix_block(
            &self.plan,
            position,
            frames,
            self.channels,
            &mut interleaved,
        );

        // The normalising gain sits before the master and the limiter, so a
        // mix brought up to its target is still held under full scale.
        if (self.gain - 1.0).abs() > 1e-6 {
            for sample in &mut interleaved {
                *sample *= self.gain;
            }
        }

        // §20a.4's final stage, exactly as playback applies it — the same
        // master volume, from the same plan (§46).
        bettercut_audio::finish(&mut interleaved, self.plan.master_volume);
        interleaved
    }

    /// The `frames` samples covering the video frame that starts at
    /// `position`.
    ///
    /// The caller tracks how many samples have been written and asks for the
    /// difference, so this needs no position of its own beyond the timeline
    /// one — which is what keeps the audio locked to §9's ticks rather than to
    /// a count that could drift from them.
    fn read(&mut self, position: TimelineTime, frames: usize) -> Vec<Vec<f32>> {
        let interleaved = self.read_interleaved(position, frames);
        deinterleave(&interleaved, self.channels)
    }
}

/// Interleaved to planar: the mixer works interleaved, the encoder wants planes.
fn deinterleave(interleaved: &[f32], channels: usize) -> Vec<Vec<f32>> {
    if channels == 0 {
        return Vec::new();
    }
    (0..channels)
        .map(|channel| {
            interleaved
                .iter()
                .skip(channel)
                .step_by(channels)
                .copied()
                .collect()
        })
        .collect()
}

/// A GPU device of our own, on this thread.
fn open_device() -> Result<(wgpu::Device, wgpu::Queue), ExportError> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|err| ExportError::NoGpu(err.to_string()))?;

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("export device"),
        ..Default::default()
    }))
    .map_err(|err| ExportError::NoGpu(err.to_string()))
}

/// Premultiplied sRGB-encoded RGBA, as the compositor reads back, turned into
/// straight alpha for an encoder that stores colour and alpha apart.
///
/// Divided in linear light: the premultiplying happened there, before the
/// target encoded it, so dividing the encoded bytes would come out too dark at
/// every soft edge.
pub fn unpremultiply(rgba: &mut [u8]) {
    let to_linear: Vec<f32> = (0..256)
        .map(|v| {
            let c = v as f32 / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect();
    let to_srgb = |linear: f32| -> u8 {
        let c = linear.clamp(0.0, 1.0);
        let encoded = if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round().clamp(0.0, 255.0) as u8
    };
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha == 255 {
            continue;
        }
        if alpha == 0 {
            pixel[..3].fill(0);
            continue;
        }
        let a = f32::from(alpha) / 255.0;
        for channel in &mut pixel[..3] {
            *channel = to_srgb(to_linear[usize::from(*channel)] / a);
        }
    }
}

/// The chapters to write into a file of `range`: the sequence's chapters
/// (`bettercut_timeline::chapter_ranges`) that fall inside the stretch, each
/// cut to it and moved so the file's first frame is time zero. None unless
/// the sequence has at least one mark inside the stretch, so an unmarked
/// export carries no chapter list at all.
pub fn chapter_marks(
    sequence: &bettercut_timeline::Sequence,
    range: TimelineRange,
) -> Vec<bettercut_media::ChapterMark> {
    let marked_inside = sequence
        .markers
        .iter()
        .any(|marker| marker.time > range.start && marker.time < range.end);
    if !marked_inside {
        return Vec::new();
    }
    bettercut_timeline::chapter_ranges(&sequence.markers, sequence.duration())
        .into_iter()
        .filter_map(|(title, chapter)| {
            let start = chapter.start.max(range.start);
            let end = chapter.end.min(range.end);
            (end > start).then(|| bettercut_media::ChapterMark {
                title,
                start: bettercut_foundation::MediaTime::from_ticks(
                    start.ticks() - range.start.ticks(),
                ),
                end: bettercut_foundation::MediaTime::from_ticks(end.ticks() - range.start.ticks()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reports_a_sane_fraction() {
        let at = |done, total| {
            ExportProgress {
                frames_done: done,
                frames_total: total,
                encoder: "test",
            }
            .fraction()
        };
        assert_eq!(at(0, 100), 0.0);
        assert_eq!(at(50, 100), 0.5);
        assert_eq!(at(100, 100), 1.0);
        // Nothing to do is done, not a division by zero.
        assert_eq!(at(0, 0), 1.0);
        // A miscount must not report more than finished.
        assert_eq!(at(150, 100), 1.0);
    }

    #[test]
    fn deinterleaving_keeps_the_channels_apart() {
        let interleaved = [1.0, -1.0, 2.0, -2.0, 3.0, -3.0];
        let planes = deinterleave(&interleaved, 2);
        assert_eq!(planes.len(), 2);
        assert_eq!(planes[0], vec![1.0, 2.0, 3.0]);
        assert_eq!(planes[1], vec![-1.0, -2.0, -3.0]);
    }

    #[test]
    fn deinterleaving_nothing_is_harmless() {
        assert!(deinterleave(&[], 2).iter().all(Vec::is_empty));
        assert!(deinterleave(&[1.0], 0).is_empty());
    }
}
