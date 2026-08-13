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

use bettercut_foundation::{TICKS_PER_AUDIO_SAMPLE, TimelineTime};
use bettercut_media::{CancellationToken, ExportFormat, MediaAsset, SeekMode, VideoWriter};
use bettercut_playback::{AudioSource, FrameSource, PlaybackEngine, layer_requests};
use bettercut_project_format::Project;
use bettercut_renderer::{Compositor, Layer, RenderConfig, wgpu};
use bettercut_timeline::{Resolution, Sequence, TimelineRange};

mod error;
mod readback;

pub use error::ExportError;

/// What to export, and where.
#[derive(Debug, Clone)]
pub struct ExportSettings {
    pub path: PathBuf,
    /// Defaults to the sequence's own format; overridable for a smaller file.
    pub resolution: Resolution,
    /// The span of the timeline to write. Usually the whole sequence.
    pub range: TimelineRange,
    /// §15.1: FFmpeg never gets every core, not even when it is the only job.
    pub threads: u32,
}

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
    let ticks_per_frame = sequence.ticks_per_frame();
    if ticks_per_frame <= 0 {
        return Err(ExportError::EmptyRange);
    }
    let span = settings.range.end.ticks() - settings.range.start.ticks();
    if span <= 0 {
        return Err(ExportError::EmptyRange);
    }

    // Frames that start inside the range. A frame beginning one tick before the
    // end is a frame the user asked for; one beginning at the end is not.
    let total_frames = (span + ticks_per_frame - 1) / ticks_per_frame;
    let total_frames = total_frames.max(1) as u64;
    let channels = audio_channels(sequence);

    let (device, queue) = open_device()?;
    let mut compositor = Compositor::new(
        device.clone(),
        queue.clone(),
        // §46's export configuration: full tier, original media, texture sink.
        RenderConfig::export_to_texture(settings.resolution),
    )?;
    let mut readback = readback::Readback::new(&device, settings.resolution);

    let mut writer = VideoWriter::create(
        &settings.path,
        ExportFormat {
            width: settings.resolution.width,
            height: settings.resolution.height,
            frame_rate: sequence.frame_rate,
            channels,
            threads: settings.threads,
        },
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
    let mut audio = AudioMixdown::new(channels);

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
            &mut frames,
            &mut compositor,
            &mut readback,
            &device,
            &queue,
            cancel,
        )?;
        writer.push_frame(&rgba)?;

        // Samples up to the *end* of this frame, from ticks rather than from a
        // running float — see the module docs.
        let frame_end = settings.range.start.ticks() + (index as i64 + 1) * ticks_per_frame;
        let target = (frame_end - settings.range.start.ticks()) / TICKS_PER_AUDIO_SAMPLE;
        let wanted = (target - samples_written).max(0) as usize;
        if wanted > 0 && channels > 0 {
            let block = audio.read(project, sequence, position, wanted);
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
    frames: &mut FrameSource,
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
        let Some(asset) = project.media_asset(request.media) else {
            continue;
        };
        // §47a.2's `Playback`: an export is a forward walk, so the next frame
        // wanted is the next frame in the file.
        match frames.decode(asset, request.source_time, SeekMode::Playback, cancel) {
            Ok(frame) => decoded.push((request, frame)),
            Err(err) => {
                // §50: one unreadable file must not abort a long export. The
                // clip is missing from that frame and the rest still renders.
                tracing::warn!(
                    file = %asset.file_name,
                    %err,
                    "could not decode a frame for export; that layer is absent"
                );
            }
        }
    }

    let layers: Vec<Layer<'_>> = decoded
        .iter()
        .map(|(request, frame)| Layer {
            frame,
            transform: request.look.transform,
            opacity: request.look.opacity,
            color: request.look.color,
            blur: request.look.blur,
        })
        .collect();

    compositor.composite(&layers)?;
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
/// Uses the same `mix_into` and `finish` the live mixer does (§20a.4), so the
/// exported audio is the audio that was played — the audio half of §51.1's
/// golden comparison.
struct AudioMixdown {
    sources: std::collections::HashMap<bettercut_foundation::MediaId, AudioSource>,
    channels: usize,
}

impl AudioMixdown {
    fn new(channels: usize) -> Self {
        Self {
            sources: std::collections::HashMap::new(),
            channels,
        }
    }

    /// The `frames` samples covering the video frame that starts at
    /// `position`.
    ///
    /// The caller tracks how many samples have been written and asks for the
    /// difference, so this needs no position of its own beyond the timeline
    /// one — which is what keeps the audio locked to §9's ticks rather than to
    /// a count that could drift from them.
    fn read(
        &mut self,
        project: &Project,
        sequence: &Sequence,
        position: TimelineTime,
        frames: usize,
    ) -> Vec<Vec<f32>> {
        let mut interleaved = vec![0.0_f32; frames * self.channels];
        let duration = TimelineTime::from_ticks(frames as i64 * TICKS_PER_AUDIO_SAMPLE);

        for audible in PlaybackEngine::resolve_audio(sequence, position, duration) {
            let Some(asset) = project.media_asset(audible.media) else {
                continue;
            };
            let Some(source) = self.source_for(asset) else {
                continue;
            };

            let offset = (audible.offset.ticks() / TICKS_PER_AUDIO_SAMPLE) as usize;
            let wanted = frames.saturating_sub(offset);
            if wanted == 0 {
                continue;
            }

            match source.read(audible.source_start, wanted) {
                Ok(planes) if !planes.is_empty() => {
                    bettercut_audio::mix_into(
                        &mut interleaved,
                        self.channels,
                        &planes,
                        offset,
                        bettercut_audio::MixParams {
                            clip_gain: audible.gain,
                            ..Default::default()
                        },
                    );
                }
                Ok(_) => {}
                Err(err) => {
                    // §50 again: a clip that will not decode is silent, not
                    // fatal to the export.
                    tracing::warn!(%err, "audio read failed during export");
                }
            }
        }

        // §20a.4's final stage, exactly as playback applies it. Master gain is
        // unity here: the mixer UI's master control is a monitoring level, not
        // something that should bake into the deliverable.
        bettercut_audio::finish(&mut interleaved, 1.0);

        deinterleave(&interleaved, self.channels)
    }

    fn source_for(&mut self, asset: &MediaAsset) -> Option<&mut AudioSource> {
        match self.sources.entry(asset.id) {
            std::collections::hash_map::Entry::Occupied(entry) => Some(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let decoder = bettercut_media::FfmpegDecoder::new(1).ok()?;
                let source = AudioSource::open(asset, Box::new(decoder)).ok()?;
                Some(entry.insert(source))
            }
        }
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
