//! Measuring the loudness of the whole mix (`bettercut_audio::loudness`).
//!
//! The meter needs samples, and the only samples that are *the mix* are the
//! ones the mixer makes — so this runs the same [`crate::mixer::AudioMixer`]
//! the preview and the export use, block by block, from the start of the
//! sequence to its end, and pushes what comes out through the meter (§46:
//! measuring anything else would be measuring a different edit).
//!
//! It decodes every sound in the sequence once, so it is a job the caller
//! should expect to take a moment on a long edit — a minute of sound is a few
//! hundred blocks.

use bettercut_audio::loudness::{LoudnessMeter, SAMPLE_RATE};
use bettercut_foundation::TimelineTime;
use bettercut_project_format::Project;
use bettercut_timeline::Sequence;

use crate::mixer::{AudioMixer, AudioPlan};

/// How much sound is mixed at a time. A tenth of a second, as the export uses.
const BLOCK_FRAMES: usize = SAMPLE_RATE as usize / 10;

/// The integrated loudness of `sequence`'s mix, in LUFS.
///
/// `None` when there is nothing to measure: no sound at all, or nothing above
/// the standard's silence gate.
pub fn measure(project: &Project, sequence: &Sequence, decoder_threads: u32) -> Option<f32> {
    let plan = AudioPlan::of(project, sequence);
    if plan.tracks.iter().all(|track| track.clips().is_empty()) {
        return None;
    }
    let channels = 2;
    let mut mixer = AudioMixer::new(decoder_threads);
    let mut meter = LoudnessMeter::new(channels);
    let mut block = vec![0.0_f32; BLOCK_FRAMES * channels];

    let end = sequence.duration();
    let step = TimelineTime::from_ticks(
        BLOCK_FRAMES as i64 * bettercut_foundation::TICKS_PER_SECOND / i64::from(SAMPLE_RATE),
    );
    let mut at = TimelineTime::ZERO;
    while at < end {
        block.fill(0.0);
        mixer.mix_block(&plan, at, BLOCK_FRAMES, channels, &mut block);
        meter.push_interleaved(&block, channels);
        at = TimelineTime::from_ticks(at.ticks() + step.ticks());
    }
    meter.integrated()
}
