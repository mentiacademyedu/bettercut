//! The playback clock (§20a.1).
//!
//! > **Playback position is derived from the audio device, never from a video
//! > timer.**
//!
//! ```text
//! Audio callback consumes N samples
//! -> playback clock advances by N samples
//! -> video frame is selected to match the clock
//! -> late frames are dropped, not queued
//! ```
//!
//! §20a.1 states the reason bluntly: driving playback from a video timer
//! produces drift that is extremely difficult to diagnose later. A video timer
//! and an audio device disagree by a few parts per million, which is inaudible
//! for a second and a visible lip-sync error after ten minutes.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};

use bettercut_foundation::{TICKS_PER_SECOND, TimelineTime};

/// Shared between the real-time audio callback and everything else.
///
/// Every field is atomic because the callback touches it, and §20a.2 forbids
/// taking a lock there.
#[derive(Debug)]
pub struct AudioClock {
    /// Timeline position the current playback run started from, in ticks.
    origin_ticks: AtomicI64,
    /// Audio frames (per-channel sample groups) the device has consumed since
    /// that origin.
    frames_played: AtomicU64,

    sample_rate: u32,
    channels: u16,

    playing: AtomicBool,
    underruns: AtomicU32,
}

impl AudioClock {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            origin_ticks: AtomicI64::new(0),
            frames_played: AtomicU64::new(0),
            sample_rate: sample_rate.max(1),
            channels: channels.max(1),
            playing: AtomicBool::new(false),
            underruns: AtomicU32::new(0),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Current playback position on the timeline.
    ///
    /// At 48 kHz this is exact: §9's timebase makes one audio sample exactly 20
    /// ticks, so no rounding accumulates however long playback runs.
    pub fn position(&self) -> TimelineTime {
        let frames = self.frames_played.load(Ordering::Acquire);
        let origin = self.origin_ticks.load(Ordering::Acquire);

        // i128 so a long session cannot overflow before the division.
        let elapsed =
            (i128::from(frames) * i128::from(TICKS_PER_SECOND)) / i128::from(self.sample_rate);
        TimelineTime::from_ticks(origin.saturating_add(elapsed as i64))
    }

    /// Move the clock to `position` and restart counting from there.
    ///
    /// Called on every seek. Without resetting `frames_played` the clock would
    /// keep accumulating from the old origin and the picture would run away
    /// from the sound.
    pub fn seek_to(&self, position: TimelineTime) {
        // Order matters: zero the frame count first, so a callback that fires
        // between these two stores reports the new origin with no elapsed
        // time, rather than the new origin plus the old elapsed time.
        self.frames_played.store(0, Ordering::Release);
        self.origin_ticks.store(position.ticks(), Ordering::Release);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    /// Start or stop the clock.
    ///
    /// Pausing folds elapsed time into the origin, so resuming continues from
    /// where it stopped instead of jumping back.
    pub fn set_playing(&self, playing: bool) {
        if !playing && self.is_playing() {
            let position = self.position();
            self.seek_to(position);
        }
        self.playing.store(playing, Ordering::Release);
    }

    /// Advance by `frames`. **Called from the audio callback only.**
    pub fn advance(&self, frames: u64) {
        self.frames_played.fetch_add(frames, Ordering::Release);
    }

    /// Record that the callback ran short of samples.
    ///
    /// §20a.2: an underrun outputs silence and increments a counter; it never
    /// blocks. A rising count means the mixer is not keeping up.
    pub fn note_underrun(&self) {
        self.underruns.fetch_add(1, Ordering::Relaxed);
    }

    pub fn underruns(&self) -> u32 {
        self.underruns.load(Ordering::Relaxed)
    }

    pub fn reset_underruns(&self) {
        self.underruns.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_second_of_audio_advances_the_clock_one_second() {
        let clock = AudioClock::new(48_000, 2);
        clock.advance(48_000);
        assert_eq!(clock.position(), TimelineTime::from_seconds(1));
    }

    /// §9: at 48 kHz one sample is exactly 20 ticks, so an hour of playback
    /// accumulates no error at all. This is the whole reason for the timebase.
    #[test]
    fn the_clock_is_exact_over_an_hour() {
        let clock = AudioClock::new(48_000, 2);
        // Advance in realistic callback-sized chunks rather than one big jump,
        // so any per-callback rounding would show up.
        for _ in 0..(3600 * 48_000 / 480) {
            clock.advance(480);
        }
        assert_eq!(
            clock.position(),
            TimelineTime::from_seconds(3600),
            "drift accumulated over an hour"
        );
    }

    #[test]
    fn seeking_moves_the_origin_and_restarts_counting() {
        let clock = AudioClock::new(48_000, 2);
        clock.advance(48_000);

        clock.seek_to(TimelineTime::from_seconds(30));
        assert_eq!(clock.position(), TimelineTime::from_seconds(30));

        clock.advance(24_000);
        assert_eq!(clock.position(), TimelineTime::from_millis(30_500));
    }

    #[test]
    fn pausing_holds_the_position_and_resuming_continues() {
        let clock = AudioClock::new(48_000, 2);
        clock.set_playing(true);
        clock.advance(48_000);

        clock.set_playing(false);
        let paused_at = clock.position();
        assert_eq!(paused_at, TimelineTime::from_seconds(1));

        // Nothing moves while paused.
        assert_eq!(clock.position(), paused_at);

        clock.set_playing(true);
        clock.advance(48_000);
        assert_eq!(clock.position(), TimelineTime::from_seconds(2));
    }

    #[test]
    fn a_non_48k_device_still_reports_sensible_positions() {
        // §20a.3 normalizes our own audio to 48 kHz, but a device may insist on
        // its own rate; the clock has to cope rather than drift.
        let clock = AudioClock::new(44_100, 2);
        clock.advance(44_100);
        assert_eq!(clock.position(), TimelineTime::from_seconds(1));
    }

    #[test]
    fn underruns_are_counted_not_hidden() {
        let clock = AudioClock::new(48_000, 2);
        assert_eq!(clock.underruns(), 0);
        clock.note_underrun();
        clock.note_underrun();
        assert_eq!(clock.underruns(), 2);
        clock.reset_underruns();
        assert_eq!(clock.underruns(), 0);
    }
}
