//! Sound made rather than read: a line-up tone, a sync beep, silence.
//!
//! Every cut needs these at some point and none of them is worth keeping a
//! file for. A line-up tone is what a delivery is checked against — a steady
//! 1 kHz at a known level, so whoever receives the file can see at a glance
//! whether their chain is right. A sync beep is the one-frame tone at the head
//! of a reel that a picture mark lines up against. And a stretch of silence is
//! how a gap is *made explicit*: a lane with nothing on it and a lane holding
//! deliberate silence look the same on a timeline and mean different things.
//!
//! There is no file behind any of them, so nothing decodes, probes or proxies
//! them ([`crate::Generated`] does the same for a picture). The samples come
//! from a formula, worked out from the *absolute source time* asked for —
//! never from how much has been played — so a block is the same samples
//! however it is cut up, and the preview and the export agree exactly (§46).

use bettercut_foundation::{AUDIO_SAMPLE_RATE, MediaTime, TICKS_PER_AUDIO_SAMPLE};
use serde::{Deserialize, Serialize};

/// The line-up tone: 1 kHz, the pitch every meter and every pair of ears is
/// calibrated against.
pub const LINE_UP_HZ: u32 = 1_000;

/// And its level, in dBFS. EBU R68's alignment level: loud enough to read
/// plainly on a meter, far enough below full scale to leave the headroom a
/// mix needs above it.
pub const LINE_UP_DB: f32 = -18.0;

/// How high and low a tone may be pitched, in Hz. Below the first is felt
/// rather than heard; above the second is past what most delivery chains keep.
pub const MIN_TONE_HZ: u32 = 20;
pub const MAX_TONE_HZ: u32 = 20_000;

/// Sound with no file behind it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GeneratedSound {
    /// A steady sine at `hz`, at `level_db` dBFS.
    Tone { hz: u32, level_db: f32 },
    /// Nothing at all — but nothing *placed*, which a gap is not.
    Silence,
}

impl GeneratedSound {
    /// The line-up tone: 1 kHz at -18 dBFS.
    pub const LINE_UP: Self = Self::Tone {
        hz: LINE_UP_HZ,
        level_db: LINE_UP_DB,
    };

    /// What the media list calls it.
    pub fn name(self) -> String {
        match self {
            Self::Tone { hz, level_db } if hz == LINE_UP_HZ && level_db == LINE_UP_DB => {
                "1 kHz tone".to_owned()
            }
            Self::Tone { hz, level_db } => format!("{hz} Hz tone at {level_db:.0} dB"),
            Self::Silence => "Silence".to_owned(),
        }
    }

    /// The tone held to what can be made and heard: a rate the samples cannot
    /// carry would alias into something else entirely, and a level above full
    /// scale is one the limiter would take back off anyway.
    pub fn clamped(self) -> Self {
        match self {
            Self::Tone { hz, level_db } => Self::Tone {
                hz: hz.clamp(MIN_TONE_HZ, MAX_TONE_HZ.min(AUDIO_SAMPLE_RATE as u32 / 2)),
                level_db: if level_db.is_finite() {
                    level_db.clamp(-96.0, 0.0)
                } else {
                    LINE_UP_DB
                },
            },
            Self::Silence => Self::Silence,
        }
    }

    /// The amplitude a tone is made at, 0 to 1.
    pub fn amplitude(self) -> f32 {
        match self.clamped() {
            Self::Tone { level_db, .. } => 10.0_f32.powf(level_db / 20.0),
            Self::Silence => 0.0,
        }
    }

    /// `frames` samples per channel, starting at `from` in the source.
    ///
    /// Two planes, because that is what a decoded file hands back and the
    /// mixer's panning is written for a stereo source; both carry the same
    /// sound, a tone having no side to be on.
    ///
    /// Worked out from `from` rather than from anything kept between calls:
    /// that is what makes a tone the same sound whether it is played, scrubbed
    /// or exported, and what lets a clip of it be cut, moved and reversed like
    /// any other.
    pub fn read(self, from: MediaTime, frames: usize) -> Vec<Vec<f32>> {
        let sound = self.clamped();
        let amplitude = sound.amplitude();
        let hz = match sound {
            Self::Tone { hz, .. } => f64::from(hz),
            Self::Silence => 0.0,
        };
        if amplitude <= 0.0 || hz <= 0.0 {
            return vec![vec![0.0; frames], vec![0.0; frames]];
        }

        let first = from.ticks().max(0) / TICKS_PER_AUDIO_SAMPLE;
        let plane: Vec<f32> = (0..frames)
            .map(|index| {
                // The phase of an absolute sample number, kept inside one turn
                // so a tone an hour in is as exact as one at the start: taken
                // as radians it would be billions of them by then, and a sine
                // of that has lost most of its precision.
                let sample = (first + index as i64) as f64;
                let turns = (sample * hz / AUDIO_SAMPLE_RATE as f64).fract();
                (std::f64::consts::TAU * turns).sin() as f32 * amplitude
            })
            .collect();
        vec![plane.clone(), plane]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0_f32, |most, s| most.max(s.abs()))
    }

    /// The line-up tone is at the level it says it is: the whole point of it
    /// is that the number on the meter is known in advance.
    #[test]
    fn the_line_up_tone_is_at_its_stated_level() {
        let planes = GeneratedSound::LINE_UP.read(MediaTime::ZERO, AUDIO_SAMPLE_RATE as usize);
        let expected = 10.0_f32.powf(LINE_UP_DB / 20.0);

        assert_eq!(planes.len(), 2);
        assert!(
            (peak(&planes[0]) - expected).abs() < 0.002,
            "{} against {expected}",
            peak(&planes[0])
        );
        assert_eq!(planes[0], planes[1], "the sides differ");
    }

    /// And at the pitch it says: one second of 1 kHz crosses zero going up a
    /// thousand times.
    #[test]
    fn a_tone_is_at_the_pitch_it_says() {
        let planes = GeneratedSound::LINE_UP.read(MediaTime::ZERO, AUDIO_SAMPLE_RATE as usize);
        let rises = planes[0]
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert_eq!(rises, LINE_UP_HZ as usize);
    }

    /// The reason it is worked out from the source time: one block or a
    /// hundred is the same sound, so preview and export cannot drift (§46).
    #[test]
    fn blocks_join_seamlessly() {
        let whole = GeneratedSound::LINE_UP.read(MediaTime::ZERO, 4_800);
        let mut joined = Vec::new();
        for block in 0..10 {
            let from = MediaTime::from_ticks(block * 480 * TICKS_PER_AUDIO_SAMPLE);
            joined.extend(GeneratedSound::LINE_UP.read(from, 480).remove(0));
        }
        assert_eq!(whole[0], joined);
    }

    /// Reading from partway in gives that part of the same tone, not the
    /// beginning of a new one.
    #[test]
    fn reading_from_partway_is_that_part_of_the_tone() {
        let whole = GeneratedSound::LINE_UP.read(MediaTime::ZERO, 1_000);
        let later =
            GeneratedSound::LINE_UP.read(MediaTime::from_ticks(600 * TICKS_PER_AUDIO_SAMPLE), 400);
        assert_eq!(whole[0][600..], later[0][..]);
    }

    #[test]
    fn silence_is_silent_and_still_the_right_length() {
        let planes = GeneratedSound::Silence.read(MediaTime::ZERO, 512);
        assert_eq!(planes.len(), 2);
        assert_eq!(planes[0].len(), 512);
        assert!(planes[0].iter().all(|s| *s == 0.0));
    }

    /// A pitch the samples cannot carry, or a level above full scale, is held
    /// to what can be made — an unclamped 30 kHz tone would alias into a
    /// different note altogether.
    #[test]
    fn nonsense_settings_are_held_to_what_can_be_made() {
        let wild = GeneratedSound::Tone {
            hz: 90_000,
            level_db: 40.0,
        };
        match wild.clamped() {
            GeneratedSound::Tone { hz, level_db } => {
                assert!(hz <= AUDIO_SAMPLE_RATE as u32 / 2, "{hz}");
                assert_eq!(level_db, 0.0);
            }
            GeneratedSound::Silence => panic!("a tone became silence"),
        }
        assert!(peak(&wild.read(MediaTime::ZERO, 256)[0]) <= 1.0);

        let broken = GeneratedSound::Tone {
            hz: 1_000,
            level_db: f32::NAN,
        };
        assert!(broken.amplitude().is_finite());
    }

    /// A tone an hour in is still the tone: worked out in turns rather than in
    /// radians, so nothing is lost to floating point.
    #[test]
    fn a_tone_far_into_the_source_is_still_exact() {
        let hour = MediaTime::from_seconds(3_600);
        let planes = GeneratedSound::LINE_UP.read(hour, 4_800);
        let expected = 10.0_f32.powf(LINE_UP_DB / 20.0);
        assert!(
            (peak(&planes[0]) - expected).abs() < 0.002,
            "{}",
            peak(&planes[0])
        );
        // 1 kHz divides into a second exactly, so an hour in the phase is back
        // where it started.
        assert!(planes[0][0].abs() < 1e-5, "{}", planes[0][0]);
    }

    #[test]
    fn each_kind_says_what_it_is() {
        assert_eq!(GeneratedSound::LINE_UP.name(), "1 kHz tone");
        assert_eq!(GeneratedSound::Silence.name(), "Silence");
        assert_eq!(
            GeneratedSound::Tone {
                hz: 440,
                level_db: -12.0
            }
            .name(),
            "440 Hz tone at -12 dB"
        );
    }
}
