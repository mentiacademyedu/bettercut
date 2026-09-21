//! What each sound lane is putting into the mix, for the meters in the track
//! heads.
//!
//! The master meter answers "is this about to clip?". A lane meter answers a
//! different question — *which* lane is loud, or silent when it should not be
//! — and that one cannot be answered after the lanes are summed. So the level
//! is taken as the block is mixed (`bettercut_audio::mix_into` gives back what
//! it added) and left here for the interface to read.
//!
//! Atomics rather than a lock, for the same reason the master meter uses them:
//! §20a.2 forbids the mixer thread ever waiting on the interface, and a meter
//! that misses a block is one block stale, which nobody can see. Each level is
//! an `f32`'s bits in a `u32` — and because a peak is never negative, the bits
//! compare in the same order as the numbers, so `fetch_max` merges two clips
//! on one lane without a read-modify-write race.

use std::sync::atomic::{AtomicU32, Ordering};

/// How many sound lanes have meters.
///
/// A short-form edit has a handful of sound lanes; past this the track head is
/// off the bottom of most windows anyway. Lanes beyond it mix exactly as they
/// always did and simply have no meter, which is better than a fixed cost per
/// lane on the audio thread.
pub const METERED_LANES: usize = 32;

/// The live level of every sound lane, written by the mixer thread and read by
/// the interface.
#[derive(Debug)]
pub struct LaneMeters {
    /// Two cells per lane: left, then right.
    cells: Vec<AtomicU32>,
}

impl Default for LaneMeters {
    fn default() -> Self {
        Self::new()
    }
}

impl LaneMeters {
    pub fn new() -> Self {
        Self {
            cells: (0..METERED_LANES * 2).map(|_| AtomicU32::new(0)).collect(),
        }
    }

    /// Replace every lane's reading with what it put into this block.
    ///
    /// Takes the whole block at once, because a lane missing from `levels` is
    /// a lane that was *silent* this block, not one to leave holding an old
    /// level: a meter still showing the last thing it heard is a meter lying
    /// about now.
    pub fn publish(&self, levels: &[(f32, f32)]) {
        for lane in 0..METERED_LANES {
            let (left, right) = levels.get(lane).copied().unwrap_or((0.0, 0.0));
            self.cells[lane * 2].store(keep(left), Ordering::Relaxed);
            self.cells[lane * 2 + 1].store(keep(right), Ordering::Relaxed);
        }
    }

    /// Raise one lane's reading, leaving it alone if it is already higher.
    /// For a mix built in pieces — a scrub burst, say — where nothing has the
    /// whole block's levels to hand.
    pub fn raise(&self, lane: usize, (left, right): (f32, f32)) {
        if lane >= METERED_LANES {
            return;
        }
        self.cells[lane * 2].fetch_max(keep(left), Ordering::Relaxed);
        self.cells[lane * 2 + 1].fetch_max(keep(right), Ordering::Relaxed);
    }

    /// Every lane to silence: what the meters must read the moment playback
    /// stops, however loud the last block was.
    pub fn silence(&self) {
        for cell in &self.cells {
            cell.store(0, Ordering::Relaxed);
        }
    }

    /// One lane's level, per side.
    pub fn read(&self, lane: usize) -> (f32, f32) {
        if lane >= METERED_LANES {
            return (0.0, 0.0);
        }
        (
            f32::from_bits(self.cells[lane * 2].load(Ordering::Relaxed)),
            f32::from_bits(self.cells[lane * 2 + 1].load(Ordering::Relaxed)),
        )
    }

    /// Every lane's level, in lane order.
    pub fn all(&self) -> Vec<(f32, f32)> {
        (0..METERED_LANES).map(|lane| self.read(lane)).collect()
    }
}

/// A level as storable bits: negative, infinite and NaN readings are nothing,
/// or the ordering `fetch_max` relies on would not hold.
fn keep(level: f32) -> u32 {
    if level.is_finite() && level > 0.0 {
        level.to_bits()
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_published_block_is_what_the_lanes_read() {
        let meters = LaneMeters::new();
        meters.publish(&[(0.5, 0.25), (0.0, 0.0), (1.0, 1.0)]);

        assert_eq!(meters.read(0), (0.5, 0.25));
        assert_eq!(meters.read(1), (0.0, 0.0));
        assert_eq!(meters.read(2), (1.0, 1.0));
        // Past what was published is silence, not the last thing there.
        assert_eq!(meters.read(3), (0.0, 0.0));
    }

    /// The reading is *this* block: a lane that has gone quiet must read quiet
    /// rather than hold what it last said.
    #[test]
    fn a_lane_that_falls_silent_reads_silent() {
        let meters = LaneMeters::new();
        meters.publish(&[(0.8, 0.8), (0.4, 0.4)]);
        meters.publish(&[(0.1, 0.1)]);

        assert_eq!(meters.read(0), (0.1, 0.1));
        assert_eq!(meters.read(1), (0.0, 0.0));
    }

    /// Raising keeps the louder of the two, which is what two clips on one
    /// lane through a crossfade need.
    #[test]
    fn raising_keeps_the_louder() {
        let meters = LaneMeters::new();
        meters.raise(0, (0.3, 0.9));
        meters.raise(0, (0.7, 0.2));
        assert_eq!(meters.read(0), (0.7, 0.9));
    }

    /// Stopping reads as silence everywhere, whatever was playing.
    #[test]
    fn silence_clears_every_lane() {
        let meters = LaneMeters::new();
        meters.publish(&[(1.0, 1.0); 4]);
        meters.silence();
        assert!(meters.all().iter().all(|level| *level == (0.0, 0.0)));
    }

    /// A reading that is not a level cannot be stored as one: it would break
    /// the ordering `raise` depends on, and a meter cannot draw it anyway.
    #[test]
    fn nonsense_levels_read_as_silence() {
        let meters = LaneMeters::new();
        meters.publish(&[(f32::NAN, f32::INFINITY), (-0.5, -0.0)]);
        assert_eq!(meters.read(0), (0.0, 0.0));
        assert_eq!(meters.read(1), (0.0, 0.0));

        meters.raise(0, (0.5, 0.5));
        meters.raise(0, (f32::NAN, f32::INFINITY));
        assert_eq!(meters.read(0), (0.5, 0.5));
    }

    /// A lane past the last metered one is ignored rather than a panic: a
    /// sequence may have more lanes than there are meters.
    #[test]
    fn a_lane_past_the_last_one_is_ignored() {
        let meters = LaneMeters::new();
        meters.raise(METERED_LANES, (1.0, 1.0));
        meters.raise(METERED_LANES + 9, (1.0, 1.0));
        assert_eq!(meters.read(METERED_LANES), (0.0, 0.0));
        assert_eq!(meters.all().len(), METERED_LANES);
    }
}
