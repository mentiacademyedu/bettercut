//! Voice effects by name — Monster, Helium, Telephone and the rest — each a
//! ready-made mix of the voice controls a sound clip already has: pitch, the
//! robot, the equaliser and the echo or room around it.
//!
//! CapCut's voice effects, out of nothing new. One click sets all four, as one
//! undo step, so a voice is never left half way between two of them; the
//! controls stay in the inspector for anyone who wants it a little deeper.

use bettercut_foundation::ClipId;
use bettercut_timeline::{AudioClip, ClipEq, ClipSpace, SpaceKind};

use crate::command::ClipProperty;
use crate::editor::Editor;
use crate::error::EditorError;

/// One voice effect: everything it sets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceEffect {
    pub name: &'static str,
    pub description: &'static str,
    /// Semitones up or down, at the same speed.
    pub pitch: f32,
    /// The robot, 0–100.
    pub robot: f32,
    pub eq: ClipEq,
    pub space: ClipSpace,
}

const FLAT: ClipEq = ClipEq {
    low_cut: 0.0,
    high_cut: 0.0,
    presence: 0.0,
    hum: 0.0,
};

const DRY: ClipSpace = ClipSpace {
    kind: SpaceKind::Dry,
    mix: 0.0,
};

/// The equaliser preset at `index` in `ClipEq::PRESETS`, so a telephone here
/// is the same telephone as in the inspector (a test checks the names line
/// up).
const fn eq_preset(index: usize) -> ClipEq {
    ClipEq::PRESETS[index].2
}

/// Every voice effect, in the order they are offered. The first is the voice
/// as recorded: everything off.
pub const VOICE_EFFECTS: [VoiceEffect; 12] = [
    VoiceEffect {
        name: "Normal",
        description: "The voice as recorded: every voice effect off",
        pitch: 0.0,
        robot: 0.0,
        eq: FLAT,
        space: DRY,
    },
    VoiceEffect {
        name: "Chipmunk",
        description: "Higher and squeakier, at the same speed",
        pitch: 7.0,
        robot: 0.0,
        eq: FLAT,
        space: DRY,
    },
    VoiceEffect {
        name: "Helium",
        description: "A whole octave up, as after a breath from a balloon",
        pitch: 12.0,
        robot: 0.0,
        eq: FLAT,
        space: DRY,
    },
    VoiceEffect {
        name: "Deep",
        description: "Lower and heavier, at the same speed",
        pitch: -5.0,
        robot: 0.0,
        eq: FLAT,
        space: DRY,
    },
    VoiceEffect {
        name: "Monster",
        description: "Far down, rough and filling the room",
        pitch: -10.0,
        robot: 30.0,
        eq: FLAT,
        space: ClipSpace {
            kind: SpaceKind::Room,
            mix: 0.3,
        },
    },
    VoiceEffect {
        name: "Robot",
        description: "A machine's voice: the words stay, the warmth goes",
        pitch: 0.0,
        robot: 100.0,
        eq: FLAT,
        space: DRY,
    },
    VoiceEffect {
        name: "Alien",
        description: "A little high, metallic and echoing",
        pitch: 4.0,
        robot: 60.0,
        eq: FLAT,
        space: ClipSpace {
            kind: SpaceKind::Echo,
            mix: 0.3,
        },
    },
    VoiceEffect {
        name: "Telephone",
        description: "The voice on the other end of a call",
        pitch: 0.0,
        robot: 0.0,
        eq: eq_preset(1),
        space: DRY,
    },
    VoiceEffect {
        name: "Radio",
        description: "An old set in the corner: thin and bright",
        pitch: 0.0,
        robot: 0.0,
        eq: eq_preset(2),
        space: DRY,
    },
    VoiceEffect {
        name: "Megaphone",
        description: "Shouted through a horn: narrow, hard and forward",
        pitch: 0.0,
        robot: 0.0,
        eq: eq_preset(3),
        space: DRY,
    },
    VoiceEffect {
        name: "Cave",
        description: "A little lower, in a huge echoing space",
        pitch: -2.0,
        robot: 0.0,
        eq: FLAT,
        space: ClipSpace {
            kind: SpaceKind::Hall,
            mix: 0.6,
        },
    },
    VoiceEffect {
        name: "Echo",
        description: "Each word answered back, as across a valley",
        pitch: 0.0,
        robot: 0.0,
        eq: FLAT,
        space: ClipSpace {
            kind: SpaceKind::Echo,
            mix: 0.5,
        },
    },
];

impl VoiceEffect {
    /// Found by name as anyone might write it: any case, spaces or none.
    pub fn named(name: &str) -> Option<&'static Self> {
        let plain = |s: &str| {
            s.chars()
                .filter(char::is_ascii_alphanumeric)
                .map(|c| c.to_ascii_lowercase())
                .collect::<String>()
        };
        VOICE_EFFECTS.iter().find(|v| plain(v.name) == plain(name))
    }

    /// Whether `clip` sounds exactly like this effect.
    pub fn is_on(&self, clip: &AudioClip) -> bool {
        (clip.pitch - self.pitch).abs() < 0.05
            && (clip.robot - self.robot).abs() < 0.5
            && clip.eq == self.eq
            && clip.space.kind == self.space.kind
            && (clip.space.mix - self.space.mix).abs() < 0.005
    }
}

impl Editor {
    /// Give each of `clips` the voice `effect`, as one undo step. Picture
    /// clips are taken to mean the sound linked to them, so selecting a shot
    /// and clicking Monster changes the voice in it. Returns the sound clips
    /// changed.
    pub fn apply_voice_effect(
        &mut self,
        clips: &[ClipId],
        effect: &VoiceEffect,
    ) -> Result<Vec<ClipId>, EditorError> {
        let mut sounds = Vec::new();
        for &clip in clips {
            let candidates = if self.audio_clip(clip).is_some() {
                vec![clip]
            } else {
                self.linked_with(clip)
            };
            for sound in candidates {
                if self.audio_clip(sound).is_some() && !sounds.contains(&sound) {
                    sounds.push(sound);
                }
            }
        }
        if sounds.is_empty() {
            return Err(EditorError::NoSoundToChange);
        }

        let depth = self.undo_depth();
        let result = sounds.iter().try_for_each(|&sound| {
            for property in [
                ClipProperty::Pitch(effect.pitch),
                ClipProperty::Robot(effect.robot),
                ClipProperty::Eq(effect.eq),
                ClipProperty::Space(effect.space),
            ] {
                self.set_clip_property(sound, property, false)?;
            }
            Ok(())
        });
        let steps = self.undo_depth().saturating_sub(depth);
        self.merge_last_steps(steps, &format!("Voice: {}", effect.name));
        result.map(|()| sounds)
    }

    /// The voice effect `clip` has, if it sounds exactly like one.
    pub fn voice_effect_of(&self, clip: ClipId) -> Option<&'static VoiceEffect> {
        let sound = self.audio_clip(clip)?;
        VOICE_EFFECTS.iter().find(|v| v.is_on(sound))
    }
}
