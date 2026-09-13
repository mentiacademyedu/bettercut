//! Counters: a title whose words are a number that counts — a countdown to a
//! launch, a stopwatch over a speed-run, "3, 2, 1".
//!
//! The clip's own text stays as its name on the timeline; what is drawn is
//! [`Counter::text_at`] for the instant being drawn, worked out from how far
//! into the clip it is. Preview and export both ask that one function, so the
//! number in the file is the number on screen (§46).
//!
//! Everything is whole ticks until the very last step — no floats (§9) — and
//! the rounding follows what a person expects of a clock: a countdown from 3
//! shows 3 for its whole first second and reaches 0 exactly when the time is
//! up; a count up shows 0 for its whole first second.

use bettercut_foundation::{TICKS_PER_SECOND, TimelineTime};
use serde::{Deserialize, Serialize};

/// Which way the number goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountDirection {
    #[default]
    Down,
    Up,
}

impl CountDirection {
    pub const ALL: [Self; 2] = [Self::Down, Self::Up];

    pub fn label(self) -> &'static str {
        match self {
            Self::Down => "Count down",
            Self::Up => "Count up",
        }
    }
}

/// How the number is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountFormat {
    /// Whole seconds: `10`, `9` … `0`.
    #[default]
    Seconds,
    /// A clock: `1:05`.
    MinutesSeconds,
    /// A clock with tenths: `1:05.3`, or `5.3` under a minute.
    Tenths,
}

impl CountFormat {
    pub const ALL: [Self; 3] = [Self::Seconds, Self::MinutesSeconds, Self::Tenths];

    pub fn label(self) -> &'static str {
        match self {
            Self::Seconds => "10",
            Self::MinutesSeconds => "0:10",
            Self::Tenths => "10.0",
        }
    }

    /// Ticks in the smallest step this format shows.
    fn step(self) -> i64 {
        match self {
            Self::Seconds | Self::MinutesSeconds => TICKS_PER_SECOND,
            Self::Tenths => TICKS_PER_SECOND / 10,
        }
    }
}

/// The longest a counter may start from: ten hours, past which the clock
/// format stops being readable and nobody is counting anyway.
pub const MAX_COUNT: TimelineTime = TimelineTime::from_ticks(10 * 3_600 * TICKS_PER_SECOND);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Counter {
    pub direction: CountDirection,
    /// The value shown at the clip's first frame.
    pub from: TimelineTime,
    pub format: CountFormat,
}

impl Counter {
    /// A countdown from `from` to zero, in whole seconds.
    pub fn countdown(from: TimelineTime) -> Self {
        Self {
            direction: CountDirection::Down,
            from,
            format: CountFormat::Seconds,
        }
    }

    /// A stopwatch from zero, in a clock with tenths.
    pub fn stopwatch() -> Self {
        Self {
            direction: CountDirection::Up,
            from: TimelineTime::ZERO,
            format: CountFormat::Tenths,
        }
    }

    /// This counter with its start held to `0..=MAX_COUNT`.
    pub fn sanitized(self) -> Self {
        Self {
            from: TimelineTime::from_ticks(self.from.ticks().clamp(0, MAX_COUNT.ticks())),
            ..self
        }
    }

    /// What it shows `into_clip` after the clip starts.
    pub fn text_at(&self, into_clip: TimelineTime) -> String {
        let into = into_clip.ticks().max(0);
        let from = self.sanitized().from.ticks();
        let step = self.format.step();
        // Whole steps shown: a countdown rounds up, so it shows its starting
        // number for the whole first step and zero only when time is up; a
        // count up rounds down, so it shows zero for the whole first step.
        let steps = match self.direction {
            CountDirection::Down => {
                let left = (from - into).max(0);
                (left + step - 1) / step
            }
            CountDirection::Up => (from + into) / step,
        };
        match self.format {
            CountFormat::Seconds => steps.to_string(),
            CountFormat::MinutesSeconds => format!("{}:{:02}", steps / 60, steps % 60),
            CountFormat::Tenths => {
                let (seconds, tenths) = (steps / 10, steps % 10);
                if seconds >= 60 {
                    format!("{}:{:02}.{tenths}", seconds / 60, seconds % 60)
                } else {
                    format!("{seconds}.{tenths}")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: i64) -> TimelineTime {
        TimelineTime::from_ticks(value * TICKS_PER_SECOND / 1000)
    }

    #[test]
    fn a_countdown_shows_its_number_for_the_whole_second_and_ends_on_zero() {
        let counter = Counter::countdown(TimelineTime::from_seconds(3));
        let shown: Vec<String> = [0, 500, 999, 1000, 2500, 2999, 3000, 9000]
            .into_iter()
            .map(|t| counter.text_at(ms(t)))
            .collect();
        assert_eq!(shown, ["3", "3", "3", "2", "1", "1", "0", "0"]);
    }

    #[test]
    fn a_count_up_starts_on_zero_and_rounds_down() {
        let counter = Counter {
            direction: CountDirection::Up,
            from: TimelineTime::ZERO,
            format: CountFormat::Seconds,
        };
        assert_eq!(counter.text_at(ms(0)), "0");
        assert_eq!(counter.text_at(ms(999)), "0");
        assert_eq!(counter.text_at(ms(1000)), "1");
    }

    #[test]
    fn clock_formats() {
        let clock = Counter {
            direction: CountDirection::Up,
            from: TimelineTime::from_seconds(60),
            format: CountFormat::MinutesSeconds,
        };
        assert_eq!(clock.text_at(TimelineTime::from_seconds(5)), "1:05");
        assert_eq!(clock.text_at(TimelineTime::ZERO), "1:00");

        let watch = Counter::stopwatch();
        assert_eq!(watch.text_at(ms(5_340)), "5.3");
        assert_eq!(watch.text_at(ms(65_300)), "1:05.3");

        let down = Counter {
            format: CountFormat::Tenths,
            ..Counter::countdown(TimelineTime::from_seconds(2))
        };
        assert_eq!(down.text_at(ms(1_950)), "0.1");
        assert_eq!(down.text_at(ms(0)), "2.0");
    }

    #[test]
    fn a_start_out_of_range_is_held_to_it() {
        let wild = Counter::countdown(TimelineTime::from_ticks(-5));
        assert_eq!(wild.sanitized().from, TimelineTime::ZERO);
        assert_eq!(wild.text_at(TimelineTime::ZERO), "0");
        let huge = Counter::countdown(TimelineTime::from_ticks(i64::MAX / 4));
        assert_eq!(huge.sanitized().from, MAX_COUNT);
    }

    #[test]
    fn a_saved_counter_reads_back() {
        let counter = Counter::stopwatch();
        let json = serde_json::to_string(&counter).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Counter>(&json).expect("parse"),
            counter
        );
    }
}
