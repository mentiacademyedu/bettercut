//! Fade the selection in, out, or both, in one click: pictures fade through
//! their entrance and exit, sounds through their volume, and a shot's sound
//! with its picture (§12). One undo step for the lot.

use std::collections::BTreeSet;

use bettercut_foundation::{ClipId, TimelineTime};
use bettercut_timeline::{ClipMotion, Motion, MotionKind};

use crate::command::{ClipProperty, Command};
use crate::editor::Editor;
use crate::error::EditorError;

/// Which ends of each clip to fade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeEnds {
    In,
    Out,
    Both,
    /// Take the fades off both ends.
    Neither,
}

impl Editor {
    /// Fade every clip in `clips` — and whatever is linked to each — at the
    /// chosen ends over `length`, as one undo step. An end not chosen keeps
    /// what it had. Each fade is held to half its clip, so the two never
    /// overlap. Returns how many clips changed; titles and adjustments are
    /// left alone.
    pub fn fade_clips(
        &mut self,
        clips: &[ClipId],
        ends: FadeEnds,
        length: TimelineTime,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let every: BTreeSet<ClipId> = clips.iter().flat_map(|c| self.linked_with(*c)).collect();
        let (fade_in, fade_out) = match ends {
            FadeEnds::In => (true, false),
            FadeEnds::Out => (false, true),
            FadeEnds::Both | FadeEnds::Neither => (true, true),
        };
        let off = ends == FadeEnds::Neither;

        let mut commands = Vec::new();
        for clip in every {
            let Some(track) = self.track_of(clip) else {
                continue;
            };
            if let Some(video) = self.video_clip(clip) {
                let fade = (!off).then(|| length.min(half(video.timeline.duration())));
                let edge = |chosen: bool, existing: Option<Motion>| {
                    if !chosen {
                        return existing;
                    }
                    fade.filter(|f| *f > TimelineTime::ZERO)
                        .map(|f| Motion::new(MotionKind::Fade, f))
                };
                let motion = ClipMotion {
                    intro: edge(fade_in, video.motion.intro),
                    outro: edge(fade_out, video.motion.outro),
                };
                if motion != video.motion {
                    commands.push(Command::SetClipProperty {
                        sequence,
                        track,
                        clip,
                        property: ClipProperty::Motion(motion),
                    });
                }
            } else if let Some(audio) = self.audio_clip(clip) {
                let fade = if off {
                    TimelineTime::ZERO
                } else {
                    length.min(half(audio.timeline.duration()))
                };
                let new_in = if fade_in { fade } else { audio.fade_in };
                let new_out = if fade_out { fade } else { audio.fade_out };
                if (new_in, new_out) != (audio.fade_in, audio.fade_out) {
                    commands.push(Command::SetClipFades {
                        sequence,
                        track,
                        clip,
                        fade_in: new_in,
                        fade_out: new_out,
                    });
                }
            }
        }
        let count = commands.len();
        if count > 0 {
            let label = match ends {
                FadeEnds::In => "Fade In",
                FadeEnds::Out => "Fade Out",
                FadeEnds::Both => "Fade In and Out",
                FadeEnds::Neither => "Remove Fades",
            };
            self.dispatch_group(label, commands)?;
        }
        Ok(count)
    }
}

fn half(length: TimelineTime) -> TimelineTime {
    TimelineTime::from_ticks(length.ticks() / 2)
}
