//! A typed timecode: what the readout accepts when it is clicked and typed
//! into, so "go to 1:02:03" is typed rather than scrolled to.
//!
//! Lenient on purpose, because nobody types a full `00:01:02:03`:
//!
//! - `1:02:03:04` is hours, minutes, seconds, frames; `1:02:03` is hours,
//!   minutes, seconds; `2:03` is minutes and seconds; `45` is seconds.
//! - The seconds may carry a decimal (`2:03.5`); a trailing `f` means frames
//!   (`120f`).
//! - A leading `+` or `-` moves from where the playhead is instead.
//!
//! Frames count at the sequence's rate, non-drop, the way the ruler shows
//! them.

use bettercut_editor_core::foundation::{FrameRate, TICKS_PER_SECOND, TimelineTime};

/// Where a typed timecode asks to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jump {
    /// To this time, as the readout calls it (from the start timecode).
    To(TimelineTime),
    /// By this much from where the playhead is; negative goes back.
    By(TimelineTime),
}

/// One frame at `rate`, in ticks, at the rate's whole-number name.
fn frame_ticks(rate: FrameRate) -> i64 {
    let nominal = rate.as_f64().round().max(1.0);
    (TICKS_PER_SECOND as f64 / nominal).round() as i64
}

/// Seconds, possibly with a decimal, to ticks.
fn seconds_ticks(text: &str) -> Option<i64> {
    let seconds: f64 = text.trim().parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some((seconds * TICKS_PER_SECOND as f64).round() as i64)
}

fn whole(text: &str) -> Option<i64> {
    text.trim().parse::<u32>().ok().map(i64::from)
}

/// Parse `text` as a timecode at `rate`. `None` for anything that is not one.
pub fn parse(text: &str, rate: FrameRate) -> Option<Jump> {
    let text = text.trim();
    let (relative, sign, body) = match text.chars().next()? {
        '+' => (true, 1, &text[1..]),
        '-' => (true, -1, &text[1..]),
        _ => (false, 1, text),
    };
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    let ticks = if let Some(frames) = body.strip_suffix(['f', 'F']) {
        whole(frames)? * frame_ticks(rate)
    } else {
        let parts: Vec<&str> = body.split(':').collect();
        match parts.as_slice() {
            [seconds] => seconds_ticks(seconds)?,
            [minutes, seconds] => whole(minutes)? * 60 * TICKS_PER_SECOND + seconds_ticks(seconds)?,
            [hours, minutes, seconds] => {
                (whole(hours)? * 3600 + whole(minutes)? * 60) * TICKS_PER_SECOND
                    + seconds_ticks(seconds)?
            }
            [hours, minutes, seconds, frames] => {
                (whole(hours)? * 3600 + whole(minutes)? * 60 + whole(seconds)?) * TICKS_PER_SECOND
                    + whole(frames)? * frame_ticks(rate)
            }
            _ => return None,
        }
    };
    let time = TimelineTime::from_ticks(sign * ticks);
    Some(if relative {
        Jump::By(time)
    } else {
        Jump::To(time)
    })
}

/// Where the playhead goes for `jump`, given where it is and what the
/// sequence calls its first frame: a typed time is what the readout shows,
/// so the start timecode comes off it. Never before the start.
pub fn resolve(jump: Jump, playhead: TimelineTime, start_timecode: TimelineTime) -> TimelineTime {
    let target = match jump {
        Jump::To(called) => called - start_timecode,
        Jump::By(delta) => playhead + delta,
    };
    target.max(TimelineTime::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: i64) -> TimelineTime {
        TimelineTime::from_seconds(s)
    }

    #[test]
    fn every_short_form_is_read() {
        let rate = FrameRate::PAL_25;
        assert_eq!(parse("45", rate), Some(Jump::To(secs(45))));
        assert_eq!(parse("2:03", rate), Some(Jump::To(secs(123))));
        assert_eq!(parse("1:02:03", rate), Some(Jump::To(secs(3723))));
        assert_eq!(
            parse("1:02:03:05", rate),
            Some(Jump::To(secs(3723) + TimelineTime::from_millis(200)))
        );
        assert_eq!(
            parse("2:03.5", rate),
            Some(Jump::To(secs(123) + TimelineTime::from_millis(500)))
        );
        assert_eq!(parse("50f", rate), Some(Jump::To(secs(2))));
        assert_eq!(parse("  1:00 ", rate), Some(Jump::To(secs(60))));
    }

    #[test]
    fn a_sign_makes_it_a_move() {
        let rate = FrameRate::PAL_25;
        assert_eq!(parse("+10", rate), Some(Jump::By(secs(10))));
        assert_eq!(parse("-1:00", rate), Some(Jump::By(secs(-60))));
        assert_eq!(parse("-25f", rate), Some(Jump::By(secs(-1))));
    }

    #[test]
    fn nonsense_is_refused() {
        let rate = FrameRate::PAL_25;
        for text in ["", "abc", "1:2:3:4:5", "-", "1::2", "1.5:00", "nan"] {
            assert_eq!(parse(text, rate), None, "{text:?}");
        }
    }

    #[test]
    fn the_start_timecode_comes_off_a_typed_time_and_nothing_goes_negative() {
        let hour = secs(3600);
        assert_eq!(resolve(Jump::To(hour + secs(5)), secs(0), hour), secs(5));
        assert_eq!(resolve(Jump::To(secs(5)), secs(0), hour), secs(0));
        assert_eq!(resolve(Jump::By(secs(-10)), secs(4), secs(0)), secs(0));
        assert_eq!(resolve(Jump::By(secs(10)), secs(4), hour), secs(14));
    }
}
