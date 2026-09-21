//! Taking the shake out of a hand-held shot.
//!
//! The measuring is the motion tracker's ([`bettercut_playback::tracker`]):
//! a patch in the middle of the frame, followed across the clip, is where the
//! camera was pointing frame by frame. What that path does is two things at
//! once — the pan the operator meant, and the wobble they did not — and the
//! difference between the path and a smoothed version of it is the wobble.
//!
//! Cancelling it means moving the picture the other way, which uncovers the
//! edge of the frame; so the clip is scaled up by just enough to keep the edge
//! out of sight, and no more. A stabilised shot is always a slightly tighter
//! shot, which is the trade every stabiliser makes.

use bettercut_foundation::ClipId;
use bettercut_timeline::{AnimatedParameter, Interpolation, Keyframe};

use crate::command::{ClipProperty, Command};
use crate::editor::Editor;
use crate::error::EditorError;
use crate::track_motion::TrackedPoint;

/// How much of the path either side is averaged into the smoothed one, as a
/// share of the path's length.
///
/// A fifth: wide enough to flatten a hand's wobble, narrow enough that a real
/// pan is still followed rather than fought.
const WINDOW: f32 = 0.2;

/// The most the picture may be scaled up to hide the edges. Past this the shot
/// is cropped so hard that stabilising has cost more than the shake did.
pub const MAX_ZOOM: f32 = 1.4;

/// What stabilising did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stabilised {
    /// Keyframes written.
    pub keys: usize,
    /// How far the picture had to move at its worst, as a share of the frame.
    pub worst: f32,
    /// What the clip was scaled to, to keep the edges out of the frame.
    pub zoom: f32,
}

impl Editor {
    /// Steady `clip` against `path` — where the picture was, frame by frame.
    ///
    /// Writes position keyframes that cancel the wobble and scales the clip up
    /// enough to keep its edges out of the frame. One undo step.
    ///
    /// Refused when the path is too short to tell a wobble from a pan, or when
    /// keeping the edges hidden would mean cropping past [`MAX_ZOOM`] — a shot
    /// that shaky is better re-shot than salvaged, and silently cropping a
    /// third of it away is not a decision to make for somebody.
    pub fn stabilise(
        &mut self,
        clip: ClipId,
        path: &[TrackedPoint],
    ) -> Result<Stabilised, EditorError> {
        if path.len() < 4 {
            return Err(EditorError::NothingTracked);
        }
        let sequence = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self
            .video_clip(clip)
            .ok_or(EditorError::ClipKindMismatch)?
            .clone();

        // The path the camera *meant*, and what is left over.
        let smoothed = smooth(path);
        let corrections: Vec<(f32, f32)> = path
            .iter()
            .zip(&smoothed)
            .map(|(point, (x, y))| (x - point.center[0], y - point.center[1]))
            .collect();
        let worst = corrections
            .iter()
            .map(|(x, y)| x.abs().max(y.abs()))
            .fold(0.0_f32, f32::max);
        // Moving the picture by `worst` uncovers that much of the far edge, so
        // the scale has to cover twice it.
        let zoom = (1.0 + 2.0 * worst).max(1.0);
        if zoom > MAX_ZOOM {
            return Err(EditorError::TooShakyToSteady);
        }

        let mut commands: Vec<Command> = Vec::new();
        for parameter in [AnimatedParameter::PositionX, AnimatedParameter::PositionY] {
            for key in self.keyframes_of(clip, parameter) {
                commands.push(Command::RemoveKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter,
                    time: key.time,
                });
            }
        }

        // Scaled first, so the keys that follow are about where the picture
        // sits rather than about how big it is.
        let scale = video.transform.scale;
        commands.push(Command::SetClipProperty {
            sequence,
            track,
            clip,
            property: ClipProperty::Scale {
                x: scale.x * zoom,
                y: scale.y * zoom,
            },
        });

        let mut keys = 0;
        for (point, (dx, dy)) in path.iter().zip(&corrections) {
            let at = video.progress_time_at(point.at);
            if at < video.source.start || at > video.source.end {
                continue;
            }
            // Down the frame for the tracker is down the frame on screen,
            // which is the other way round in the clip's own units.
            for (parameter, value) in [
                (
                    AnimatedParameter::PositionX,
                    video.transform.position.x + dx,
                ),
                (
                    AnimatedParameter::PositionY,
                    video.transform.position.y - dy,
                ),
            ] {
                commands.push(Command::SetKeyframe {
                    sequence,
                    track,
                    clip,
                    parameter,
                    key: Keyframe::new(at, parameter.clamp(value), Interpolation::Linear),
                });
            }
            keys += 1;
        }
        if keys == 0 {
            return Err(EditorError::NothingTracked);
        }
        self.dispatch_group("Stabilise".to_owned(), commands)?;
        Ok(Stabilised { keys, worst, zoom })
    }
}

/// The path with the wobble taken out: each point averaged with its
/// neighbours across a window of [`WINDOW`].
fn smooth(path: &[TrackedPoint]) -> Vec<(f32, f32)> {
    let reach = ((path.len() as f32 * WINDOW).round() as usize).max(1);
    (0..path.len())
        .map(|index| {
            let from = index.saturating_sub(reach);
            let to = (index + reach + 1).min(path.len());
            let window = &path[from..to];
            let count = window.len() as f32;
            (
                window.iter().map(|p| p.center[0]).sum::<f32>() / count,
                window.iter().map(|p| p.center[1]).sum::<f32>() / count,
            )
        })
        .collect()
}
