//! Copy and paste a clip's animation.
//!
//! A move worked out on one shot — a slow push in, a fade and a drift — is
//! wanted on the next shots too. Copy Animation takes every keyframe off a
//! clip; Paste Animation puts the same movement on others, timed from each
//! clip's own start, as one undo step.
//!
//! Keys are kept in source time on a clip (`bettercut_timeline::keyframe`), so
//! they are carried between clips as a distance **along the timeline** from
//! the clip's start: a key a second into a shot lands a second into the shot
//! it is pasted on, whatever either one's in-point or speed. Keys past the end
//! of a shorter clip are left off rather than piled on its last frame. Pasting
//! replaces the animation of the parameters it carries and leaves the others.

use bettercut_foundation::{ClipId, MediaTime};
use bettercut_timeline::{AnimatedParameter, Interpolation, Keyframe, timeline_ticks_for};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

/// One copied key: its parameter, how far along the timeline from the clip's
/// start it sits, and what it holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CopiedKey {
    pub parameter: AnimatedParameter,
    pub offset_ticks: i64,
    pub value: f32,
    pub interpolation: Interpolation,
}

/// A clip's animation, as copied.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CopiedAnimation {
    pub keys: Vec<CopiedKey>,
}

impl CopiedAnimation {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The parameters it animates, each once.
    pub fn parameters(&self) -> Vec<AnimatedParameter> {
        let mut out: Vec<AnimatedParameter> = Vec::new();
        for key in &self.keys {
            if !out.contains(&key.parameter) {
                out.push(key.parameter);
            }
        }
        out
    }
}

impl Editor {
    /// Every keyframe on picture clip `clip`. `None` for a clip with none.
    pub fn copy_animation(&self, clip: ClipId) -> Option<CopiedAnimation> {
        let video = self.video_clip(clip)?;
        let keys: Vec<CopiedKey> = video
            .keyframes
            .iter()
            .flat_map(|track| {
                track.keys().iter().map(move |key| CopiedKey {
                    parameter: track.parameter,
                    offset_ticks: timeline_ticks_for(
                        MediaTime::from_ticks(key.time.ticks() - video.source.start.ticks()),
                        video.speed,
                    ),
                    value: key.value,
                    interpolation: key.interpolation,
                })
            })
            .collect();
        (!keys.is_empty()).then_some(CopiedAnimation { keys })
    }

    /// Put `animation` on each picture clip in `targets`, as one undo step.
    /// Returns how many clips took it.
    pub fn paste_animation(
        &mut self,
        animation: &CopiedAnimation,
        targets: impl IntoIterator<Item = ClipId>,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let parameters = animation.parameters();
        let mut commands = Vec::new();
        let mut taken = 0;
        for clip in targets {
            let Some(video) = self.video_clip(clip).cloned() else {
                continue;
            };
            let Some(track) = self.track_of(clip) else {
                continue;
            };
            taken += 1;
            // The pasted parameters' own keys go first.
            for parameter in &parameters {
                if let Some(existing) = video.keyframes.track(*parameter) {
                    for key in existing.keys() {
                        commands.push(Command::RemoveKeyframe {
                            sequence,
                            track,
                            clip,
                            parameter: *parameter,
                            time: key.time,
                        });
                    }
                }
            }
            let length = video.timeline.duration().ticks();
            for key in &animation.keys {
                if key.offset_ticks < 0 || key.offset_ticks > length {
                    continue;
                }
                // Back from timeline distance to this clip's source time.
                let into_source = video.speed.scale(key.offset_ticks);
                commands.push(Command::SetKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter: key.parameter,
                    key: Keyframe::new(
                        MediaTime::from_ticks(video.source.start.ticks() + into_source),
                        key.value,
                        key.interpolation,
                    ),
                });
            }
        }
        if commands.is_empty() {
            return Ok(0);
        }
        self.dispatch_group("Paste Animation".to_owned(), commands)?;
        Ok(taken)
    }
}
