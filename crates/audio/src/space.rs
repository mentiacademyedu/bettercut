//! Echo and reverb: putting a recording somewhere.
//!
//! Three places, the ones a short-form editor offers: an **echo** (the voice
//! coming back off a far wall, a few times, quieter each time), a **room**
//! (a small, close reverb that takes the dryness off a phone recording), and a
//! **hall** (a big, long one). One amount blends the effect in.
//!
//! The reverbs are Schroeder–Moorer in the Freeverb arrangement: eight damped
//! comb filters in parallel, then four all-passes in series, per channel, with
//! the right channel's delays spread a little so the space is wide rather than
//! centred. Delay lengths are fixed sample counts at the working rate, so the
//! result is the same on every machine — and, like the equaliser, all state
//! carries from block to block, so the preview and the export agree (§46).
//!
//! The tail stops where the clip stops: the mixer only reads a clip inside its
//! own span. A clip that should ring on is one trimmed a little longer.

use bettercut_foundation::AUDIO_SAMPLE_RATE;
/// Where a recording is put. Dry is no [`Space`] at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    /// The sound coming back a few times, quieter each time.
    Echo,
    /// A small, close space.
    Room,
    /// A big space with a long tail.
    Hall,
}

/// Echo: how far apart the repeats are, and how much each keeps.
const ECHO_DELAY_SECONDS: f32 = 0.32;
const ECHO_FEEDBACK: f32 = 0.45;

/// Freeverb's comb and all-pass lengths at 44.1 kHz, scaled to the working
/// rate, and the extra samples the right channel's delays get.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const ALLPASS_FEEDBACK: f32 = 0.5;
/// Keeps eight combs summed from clipping.
const INPUT_GAIN: f32 = 0.015;

fn scaled(samples: usize) -> usize {
    samples * AUDIO_SAMPLE_RATE as usize / 44_100
}

struct Comb {
    buffer: Vec<f32>,
    at: usize,
    feedback: f32,
    damp: f32,
    stored: f32,
}

impl Comb {
    fn new(length: usize, feedback: f32, damp: f32) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            at: 0,
            feedback,
            damp,
            stored: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.at];
        // A one-pole low pass in the loop: high frequencies die away first,
        // as they do off real walls.
        self.stored = output * (1.0 - self.damp) + self.stored * self.damp;
        self.buffer[self.at] = input + self.stored * self.feedback;
        self.at = (self.at + 1) % self.buffer.len();
        output
    }
}

struct AllPass {
    buffer: Vec<f32>,
    at: usize,
}

impl AllPass {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            at: 0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.at];
        let output = delayed - input;
        self.buffer[self.at] = input + delayed * ALLPASS_FEEDBACK;
        self.at = (self.at + 1) % self.buffer.len();
        output
    }
}

/// One channel's worth of a space.
enum Channel {
    Echo {
        buffer: Vec<f32>,
        at: usize,
    },
    Reverb {
        combs: Vec<Comb>,
        allpasses: Vec<AllPass>,
    },
}

impl Channel {
    fn new(place: Place, index: usize) -> Self {
        let spread = index * STEREO_SPREAD;
        let (feedback, damp) = match place {
            Place::Echo => {
                // The right channel a touch later, so the repeats are wide.
                let length = (ECHO_DELAY_SECONDS * AUDIO_SAMPLE_RATE as f32) as usize + spread * 8;
                return Self::Echo {
                    buffer: vec![0.0; length],
                    at: 0,
                };
            }
            Place::Room => (0.72, 0.4),
            Place::Hall => (0.88, 0.2),
        };
        Self::Reverb {
            combs: COMBS
                .iter()
                .map(|&n| Comb::new(scaled(n + spread), feedback, damp))
                .collect(),
            allpasses: ALLPASSES
                .iter()
                .map(|&n| AllPass::new(scaled(n + spread)))
                .collect(),
        }
    }

    /// The effect alone for one input sample.
    fn wet(&mut self, input: f32) -> f32 {
        match self {
            Self::Echo { buffer, at } => {
                let delayed = buffer[*at];
                buffer[*at] = input + delayed * ECHO_FEEDBACK;
                *at = (*at + 1) % buffer.len();
                delayed
            }
            Self::Reverb { combs, allpasses } => {
                let fed = input * INPUT_GAIN;
                let mut out = combs.iter_mut().map(|c| c.process(fed)).sum::<f32>();
                for allpass in allpasses.iter_mut() {
                    out = allpass.process(out);
                }
                out
            }
        }
    }
}

/// A recording put in a space, sample by sample.
pub struct Space {
    place: Place,
    mix: f32,
    channels: Vec<Channel>,
}

impl Space {
    /// `place`, blended in by `mix` (0–1). `None` for no blend, which
    /// changes nothing.
    pub fn new(place: Place, mix: f32) -> Option<Self> {
        let mix = if mix.is_finite() {
            mix.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if mix <= 0.0 {
            return None;
        }
        Some(Self {
            place,
            mix,
            channels: Vec::new(),
        })
    }

    /// Process `planes` (one per channel) in place, carrying on from the last
    /// call as if the two were one block.
    ///
    /// The dry sound stays at full level up to half the blend and eases down
    /// past it, while the effect rises all the way: at full blend the echo or
    /// the room is as loud as the voice, never louder than the voice was.
    pub fn process(&mut self, planes: &mut [Vec<f32>]) {
        if self.channels.len() != planes.len() {
            self.channels = (0..planes.len())
                .map(|i| Channel::new(self.place, i))
                .collect();
        }
        let wet_gain = self.mix;
        let dry_gain = 1.0 - (self.mix - 0.5).max(0.0);
        for (plane, channel) in planes.iter_mut().zip(self.channels.iter_mut()) {
            for sample in plane.iter_mut() {
                let wet = channel.wet(*sample);
                *sample = *sample * dry_gain + wet * wet_gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(frames: usize) -> Vec<f32> {
        let mut plane = vec![0.0; frames];
        plane[0] = 1.0;
        plane
    }

    #[test]
    fn no_blend_is_nothing_to_do() {
        assert!(Space::new(Place::Hall, 0.0).is_none());
        assert!(Space::new(Place::Hall, f32::NAN).is_none());
    }

    /// A click comes back after the echo's delay, quieter, and again after
    /// twice it, quieter still.
    #[test]
    fn an_echo_repeats_a_click_quieter_each_time() {
        let delay = (ECHO_DELAY_SECONDS * AUDIO_SAMPLE_RATE as f32) as usize;
        let mut space = Space::new(Place::Echo, 1.0).unwrap();
        let mut planes = vec![click(delay * 3 + 10)];
        space.process(&mut planes);
        let plane = &planes[0];
        let first = plane[delay];
        let second = plane[delay * 2];
        assert!(first > 0.3, "no first repeat: {first}");
        assert!(
            second > 0.0 && second < first,
            "the second repeat is not quieter"
        );
        assert!(
            plane[1..delay].iter().all(|s| *s == 0.0),
            "sound before the echo"
        );
    }

    /// A hall rings on far longer than a room.
    #[test]
    fn a_hall_rings_longer_than_a_room() {
        let tail = |kind| {
            let mut space = Space::new(kind, 1.0).unwrap();
            let frames = AUDIO_SAMPLE_RATE as usize * 2;
            let mut planes = vec![click(frames)];
            space.process(&mut planes);
            // Energy in the second second.
            planes[0][frames / 2..].iter().map(|s| s * s).sum::<f32>()
        };
        let (room, hall) = (tail(Place::Room), tail(Place::Hall));
        assert!(hall > room * 4.0, "room {room}, hall {hall}");
    }

    /// One block or many: the same samples.
    #[test]
    fn blocks_join_seamlessly() {
        let frames = 20_000;
        let signal: Vec<f32> = (0..frames)
            .map(|i| ((i as f32) * 0.05).sin() * 0.5)
            .collect();
        let mut whole = Space::new(Place::Hall, 0.6).unwrap();
        let mut one = vec![signal.clone(), signal.clone()];
        whole.process(&mut one);

        let mut pieces = Space::new(Place::Hall, 0.6).unwrap();
        let mut joined = vec![Vec::new(), Vec::new()];
        for chunk in signal.chunks(1_024) {
            let mut block = vec![chunk.to_vec(), chunk.to_vec()];
            pieces.process(&mut block);
            joined[0].extend_from_slice(&block[0]);
            joined[1].extend_from_slice(&block[1]);
        }
        assert_eq!(one, joined);
    }

    /// The two channels differ, so the space is wide.
    #[test]
    fn the_space_is_stereo() {
        let mut space = Space::new(Place::Room, 1.0).unwrap();
        let mut planes = vec![click(10_000), click(10_000)];
        space.process(&mut planes);
        assert_ne!(planes[0], planes[1]);
    }

    /// Stays finite and bounded on a loud, long input.
    #[test]
    fn a_loud_input_does_not_run_away() {
        for kind in [Place::Echo, Place::Room, Place::Hall] {
            let mut space = Space::new(kind, 1.0).unwrap();
            let mut planes = vec![vec![1.0; AUDIO_SAMPLE_RATE as usize * 3]];
            space.process(&mut planes);
            let peak = planes[0].iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            assert!(peak.is_finite() && peak < 4.0, "{kind:?} peaked at {peak}");
        }
    }
}
