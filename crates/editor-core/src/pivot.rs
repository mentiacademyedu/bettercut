//! The pivot: the point a picture turns and scales about, moved without the
//! picture moving with it.
//!
//! A pivot is a place *in the picture* — 0..1 across and down it — and the
//! renderer puts that place at the clip's position. Move the pivot on its
//! own and the picture jumps, because the position now names a different
//! part of it. So moving the pivot means moving the position by the same
//! amount, worked out through the same matrix the renderer uses: the picture
//! stays exactly where it was, and only the point it turns about has changed.

use bettercut_foundation::ClipId;
use bettercut_timeline::{Transform, Vec2, fit_scale};

use crate::command::{ClipProperty, Command};
use crate::editor::Editor;
use crate::error::EditorError;

/// The position that keeps a picture where it is once its pivot moves to
/// `anchor`, in the sequence's frame units.
///
/// The renderer maps a point `p` of the picture to
/// `M·(p − anchor) + position`, with `M` the fit, scale and rotation. For the
/// picture to stay put as `anchor` becomes `anchor'`, the position has to
/// gain `M·(anchor' − anchor)` — the same term, so the two cancel.
pub fn position_keeping_place(
    transform: Transform,
    source_aspect: f32,
    output_aspect: f32,
    anchor: Vec2,
) -> Vec2 {
    let (fit_x, fit_y) = fit_scale(source_aspect, output_aspect);
    let flip_x = if transform.flip_h { -1.0 } else { 1.0 };
    let flip_y = if transform.flip_v { -1.0 } else { 1.0 };
    let scale_x = fit_x * transform.scale.x * flip_x;
    let scale_y = fit_y * transform.scale.y * flip_y;
    let (sin, cos) = transform.rotation_degrees.to_radians().sin_cos();
    let aspect = output_aspect.max(0.01);

    // The renderer's own matrix, in frame units rather than clip space
    // (halved), and with y down as positions are.
    let a = scale_x * cos;
    let b = -scale_x * aspect * sin;
    let c = -scale_y * sin / aspect;
    let d = -scale_y * cos;

    let (dx, dy) = (anchor.x - transform.anchor.x, anchor.y - transform.anchor.y);
    Vec2::new(
        transform.position.x + (a * dx + c * dy),
        transform.position.y - (b * dx + d * dy),
    )
}

impl Editor {
    /// Move a picture's pivot to `anchor` (0..1 across and down it) and its
    /// position with it, so the picture does not move: one step, as a drag
    /// on the preview's pivot handle is one gesture.
    ///
    /// A clip whose position is keyed keeps its keys and only its pivot
    /// moves: there is no one position to correct, and rewriting every key
    /// would be a bigger edit than was asked for.
    pub fn set_pivot_keeping_place(
        &mut self,
        clip: ClipId,
        anchor: Vec2,
        continuing: bool,
    ) -> Result<(), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let video = self.video_clip(clip).ok_or(EditorError::ClipKindMismatch)?;
        let held = |v: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.5
            }
        };
        let anchor = Vec2::new(held(anchor.x), held(anchor.y));

        let keyed = video
            .keyframes
            .is_animated(bettercut_timeline::AnimatedParameter::PositionX)
            || video
                .keyframes
                .is_animated(bettercut_timeline::AnimatedParameter::PositionY);
        let mut commands = vec![Command::SetClipProperty {
            sequence: sequence_id,
            track,
            clip,
            property: ClipProperty::Anchor {
                x: anchor.x,
                y: anchor.y,
            },
        }];
        if !keyed {
            let sequence = self
                .active_sequence()
                .ok_or(EditorError::SequenceNotFound(sequence_id))?;
            let output_aspect =
                sequence.resolution.width.max(1) as f32 / sequence.resolution.height.max(1) as f32;
            let source_aspect = self
                .project()
                .media_asset(video.media_id)
                .filter(|asset| asset.height > 0)
                .map_or(output_aspect, |asset| {
                    asset.width as f32 / asset.height as f32
                });
            let position =
                position_keeping_place(video.transform, source_aspect, output_aspect, anchor);
            commands.push(Command::SetClipProperty {
                sequence: sequence_id,
                track,
                clip,
                property: ClipProperty::Position {
                    x: position.x,
                    y: position.y,
                },
            });
        }
        self.dispatch_gesture("Move Pivot".to_owned(), commands, continuing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5
    }

    /// Unscaled and unturned in a frame it fills: moving the pivot to the
    /// left edge moves the position half a frame left, so the picture's left
    /// edge is where its middle was — which is where it already was.
    #[test]
    fn moving_the_pivot_moves_the_position_by_the_same_span() {
        let transform = Transform {
            position: Vec2::new(0.1, 0.0),
            ..Transform::default()
        };
        let moved = position_keeping_place(transform, 16.0 / 9.0, 16.0 / 9.0, Vec2::new(0.0, 0.5));
        assert!(close(moved, Vec2::new(0.1 - 0.5, 0.0)), "{moved:?}");

        let down = position_keeping_place(transform, 16.0 / 9.0, 16.0 / 9.0, Vec2::new(0.5, 1.0));
        assert!(close(down, Vec2::new(0.1, 0.5)), "{down:?}");
    }

    /// The pivot stays at the same place on screen: the point of the whole
    /// thing. Checked by mapping the *new* pivot through the renderer's
    /// matrix before and after and finding it where the old one was — for a
    /// picture that is scaled and turned, where a naive shift would miss.
    #[test]
    fn the_picture_stays_where_it_was_when_scaled_and_turned() {
        let transform = Transform {
            position: Vec2::new(-0.2, 0.15),
            scale: Vec2::new(0.7, 0.7),
            rotation_degrees: 30.0,
            ..Transform::default()
        };
        let (source, output) = (4.0 / 3.0, 16.0 / 9.0);

        let place = |t: Transform, p: Vec2| -> Vec2 {
            let (fit_x, fit_y) = fit_scale(source, output);
            let (sin, cos) = t.rotation_degrees.to_radians().sin_cos();
            let sx = fit_x * t.scale.x;
            let sy = fit_y * t.scale.y;
            let (a, b, c, d) = (sx * cos, -sx * output * sin, -sy * sin / output, -sy * cos);
            let (dx, dy) = (p.x - t.anchor.x, p.y - t.anchor.y);
            Vec2::new(
                t.position.x + a * dx + c * dy,
                t.position.y - (b * dx + d * dy),
            )
        };
        let corner = Vec2::new(1.0, 1.0);
        let before = place(transform, corner);

        let anchor = Vec2::new(0.2, 0.8);
        let mut after = transform;
        after.anchor = anchor;
        after.position = position_keeping_place(transform, source, output, anchor);
        assert!(
            close(place(after, corner), before),
            "{:?} vs {before:?}",
            place(after, corner)
        );
    }

    /// A pivot already where it is asked to go changes nothing.
    #[test]
    fn the_same_pivot_is_the_same_position() {
        let transform = Transform {
            position: Vec2::new(0.3, -0.1),
            rotation_degrees: 12.0,
            ..Transform::default()
        };
        let same = position_keeping_place(transform, 1.0, 1.0, transform.anchor);
        assert!(close(same, transform.position));
    }
}
