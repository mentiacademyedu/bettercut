//! Sound only: the timeline's mix, written as a WAV file.
//!
//! For the voice-over sent to someone else to master, the podcast cut of a
//! video, the music bed checked on another speaker. The picture is not
//! rendered at all — no GPU device is opened — so it is quick, and it works on
//! a machine with no GPU.
//!
//! ## The same mix as the video's sound
//!
//! Through [`crate::AudioMixdown`], the very mixer an export's audio stream
//! comes from, which is the mixer the preview plays (§46). Sample positions
//! come from §9 ticks, exactly as in [`crate::export`], so a WAV of a range and
//! the sound of a video of the same range hold the same samples.
//!
//! ## 16-bit PCM at 48 kHz
//!
//! The one WAV every tool opens. The mix is limited before it gets here
//! (`bettercut_audio::finish`), so conversion is a clamp and a round, not a
//! place for clipping to appear.

use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use bettercut_foundation::{AUDIO_SAMPLE_RATE, TICKS_PER_AUDIO_SAMPLE, TimelineTime};
use bettercut_media::CancellationToken;
use bettercut_project_format::Project;
use bettercut_timeline::Sequence;

use crate::{AudioMixdown, ExportError, ExportProgress, ExportSettings, ExportSummary};

/// What the status bar names as the "encoder" for a WAV.
pub const WAV_ENCODER: &str = "WAV, 16-bit PCM";

const CHANNELS: u16 = 2;
const BITS: u16 = 16;
const HEADER_BYTES: u64 = 44;

/// Samples mixed per step: a tenth of a second, so progress moves smoothly and
/// cancelling is prompt without mixing in slivers.
const BLOCK: i64 = AUDIO_SAMPLE_RATE / 10;

/// A WAV file being written: the header goes first with placeholder sizes, and
/// `finish` fills them in once the length is known.
pub struct WavWriter<W: Write + Seek> {
    out: W,
    samples: u64,
}

impl WavWriter<std::io::BufWriter<std::fs::File>> {
    pub fn create(path: &Path) -> Result<Self, ExportError> {
        let file = std::fs::File::create(path)
            .map_err(|err| ExportError::Write(format!("{}: {err}", path.display())))?;
        Self::new(std::io::BufWriter::new(file))
            .map_err(|err| ExportError::Write(format!("{}: {err}", path.display())))
    }
}

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut out: W) -> std::io::Result<Self> {
        write_header(&mut out, 0)?;
        Ok(Self { out, samples: 0 })
    }

    /// Append interleaved stereo samples in -1..1.
    pub fn push(&mut self, interleaved: &[f32]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(interleaved.len() * 2);
        for sample in interleaved {
            let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        self.out.write_all(&bytes)?;
        self.samples += interleaved.len() as u64;
        Ok(())
    }

    /// Fill in the sizes and hand back the stream.
    pub fn finish(mut self) -> std::io::Result<W> {
        let data = self.samples * u64::from(BITS / 8);
        // RIFF sizes are 32-bit. Six hours of stereo at 48 kHz is where that
        // runs out; an edit that long gets a clear error, not a corrupt file.
        let data = u32::try_from(data).map_err(|_| {
            std::io::Error::other("the sound is too long for a WAV file (over six hours)")
        })?;
        self.out.seek(SeekFrom::Start(0))?;
        write_header(&mut self.out, data)?;
        self.out.seek(SeekFrom::End(0))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

fn write_header(out: &mut impl Write, data_bytes: u32) -> std::io::Result<()> {
    let rate = AUDIO_SAMPLE_RATE as u32;
    let block_align = CHANNELS * (BITS / 8);
    out.write_all(b"RIFF")?;
    out.write_all(&(data_bytes + HEADER_BYTES as u32 - 8).to_le_bytes())?;
    out.write_all(b"WAVE")?;
    out.write_all(b"fmt ")?;
    out.write_all(&16_u32.to_le_bytes())?;
    out.write_all(&1_u16.to_le_bytes())?; // PCM
    out.write_all(&CHANNELS.to_le_bytes())?;
    out.write_all(&rate.to_le_bytes())?;
    out.write_all(&(rate * u32::from(block_align)).to_le_bytes())?;
    out.write_all(&block_align.to_le_bytes())?;
    out.write_all(&BITS.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_bytes.to_le_bytes())
}

/// Write `settings.range` of the sequence's sound to `settings.path`.
///
/// Blocking, for a worker thread. A cancelled or failed write removes its file,
/// as a video export does.
pub fn export_sound(
    project: &Project,
    sequence: &Sequence,
    settings: &ExportSettings,
    on_progress: &mut dyn FnMut(ExportProgress),
    cancel: &dyn CancellationToken,
) -> Result<ExportSummary, ExportError> {
    let start = settings.range.start.ticks();
    let span = settings.range.end.ticks() - start;
    if span <= 0 {
        return Err(ExportError::EmptyRange);
    }
    if !sequence.audio_tracks.iter().any(|track| track.enabled) {
        return Err(ExportError::NoSound);
    }

    let total = span / TICKS_PER_AUDIO_SAMPLE;
    let blocks = (total + BLOCK - 1) / BLOCK;
    let mut mixdown = AudioMixdown::new(project, sequence, usize::from(CHANNELS));
    if let Some(target) = settings.loudness_target {
        mixdown.normalise_to(settings.range, target, cancel);
    }
    let mut writer = WavWriter::create(&settings.path)?;

    let result = (|| {
        let mut written = 0_i64;
        for block in 0..blocks {
            if cancel.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            let frames = (total - written).min(BLOCK);
            let position = TimelineTime::from_ticks(start + written * TICKS_PER_AUDIO_SAMPLE);
            let planes = mixdown.read(position, frames as usize);
            let interleaved: Vec<f32> = (0..frames as usize)
                .flat_map(|i| planes.iter().map(move |plane| plane[i]))
                .collect();
            writer
                .push(&interleaved)
                .map_err(|err| ExportError::Write(format!("{}: {err}", settings.path.display())))?;
            written += frames;
            on_progress(ExportProgress {
                frames_done: block as u64 + 1,
                frames_total: blocks as u64,
                encoder: WAV_ENCODER,
            });
        }
        writer
            .finish()
            .map_err(|err| ExportError::Write(format!("{}: {err}", settings.path.display())))?;
        Ok(ExportSummary {
            path: settings.path.clone(),
            frames: 0,
            encoder: WAV_ENCODER,
            hardware: false,
        })
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&settings.path);
    }
    result
}
