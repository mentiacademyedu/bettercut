//! A lower third in one click: a coloured bar, a name beside it and a role
//! under the name, low on the left, sliding in and fading out together — the
//! caption that says who is talking.
//!
//! Three ordinary titles on three title lanes, so each part can be restyled,
//! moved or retimed afterwards with the tools every title already has.

use bettercut_foundation::{ClipId, TimelineTime, TrackId};
use bettercut_text::{Rgba, Shadow, Shape, ShapeKind, TextStyle};
use bettercut_timeline::{Motion, MotionKind, TextClip, Transform, Vec2};

use crate::command::{Command, TrackKindRepr};
use crate::editor::Editor;
use crate::error::EditorError;

/// How long a lower third stays up.
pub const LOWER_THIRD_DURATION: TimelineTime = TimelineTime::from_seconds(5);

/// The bar's colour when none is chosen: a warm orange that reads on most
/// footage.
pub const LOWER_THIRD_ACCENT: [u8; 3] = [255, 140, 40];

/// Where the block sits: the left edge of the name and role, and the middle of
/// the name's line, in the transform's units from the frame's centre.
const LEFT: f32 = -0.40;
const NAME_Y: f32 = 0.27;
const ROLE_Y: f32 = 0.335;

impl Editor {
    /// Put a lower third at the playhead: `name` large, `role` small beneath,
    /// a bar in `accent` to their left. One undo step; title lanes are added
    /// when there are not three free for its length. Returns the bar, the name
    /// and the role, in that order (the role is left out when empty).
    pub fn add_lower_third(
        &mut self,
        name: &str,
        role: &str,
        accent: [u8; 3],
    ) -> Result<Vec<ClipId>, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let name = name.trim();
        let role = role.trim();
        if name.is_empty() {
            return Err(EditorError::LowerThirdNeedsName);
        }
        let start = self.playhead();
        let end = start + LOWER_THIRD_DURATION;

        let [r, g, b] = accent;
        let shadow = Some(Shadow {
            offset_x: 0.0,
            offset_y: 3.0,
            blur: 10.0,
            color: Rgba::new(0, 0, 0, 200),
        });
        let enter = |kind, millis| bettercut_timeline::TextAnimation {
            scroll: None,
            intro: Some(Motion::new(kind, TimelineTime::from_millis(millis))),
            outro: Some(Motion::new(
                MotionKind::Fade,
                TimelineTime::from_millis(300),
            )),
            looping: None,
        };
        let placed = |x: f32, y: f32| Transform {
            position: Vec2::new(x, y),
            anchor: Vec2::new(0.0, 0.5),
            ..Transform::default()
        };

        let mut bar = TextClip::with_duration("Lower third bar", start, LOWER_THIRD_DURATION)?;
        bar.shape = Some(Shape {
            width: 12.0,
            height: if role.is_empty() { 70.0 } else { 120.0 },
            fill: Rgba::opaque(r, g, b),
            ..Shape::new(ShapeKind::Rectangle)
        });
        bar.transform = placed(
            LEFT - 0.018,
            if role.is_empty() {
                NAME_Y
            } else {
                (NAME_Y + ROLE_Y) / 2.0
            },
        );
        bar.animation = enter(MotionKind::SlideUp, 350);

        let mut title = TextClip::with_duration(name, start, LOWER_THIRD_DURATION)?;
        title.style = TextStyle {
            size: 58.0,
            weight: bettercut_text::FontWeight::Bold,
            align: bettercut_text::Alignment::Left,
            stroke: None,
            shadow,
            ..TextStyle::default()
        };
        title.transform = placed(LEFT, NAME_Y);
        title.animation = enter(MotionKind::SlideRight, 450);

        let mut parts = vec![bar, title];
        if !role.is_empty() {
            let mut subtitle = TextClip::with_duration(role, start, LOWER_THIRD_DURATION)?;
            subtitle.style = TextStyle {
                size: 34.0,
                weight: bettercut_text::FontWeight::Medium,
                align: bettercut_text::Alignment::Left,
                color: Rgba::new(230, 230, 230, 255),
                stroke: None,
                shadow,
                ..TextStyle::default()
            };
            subtitle.transform = placed(LEFT, ROLE_Y);
            subtitle.animation = enter(MotionKind::Fade, 700);
            parts.push(subtitle);
        }
        for part in &mut parts {
            self.fit_to_frame(part);
        }

        // A free title lane for each part: existing ones with room first, in
        // order, then new ones.
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let mut lanes: Vec<TrackId> = sequence
            .text_tracks
            .iter()
            .filter(|track| track.name != Self::CAPTION_TRACK)
            .filter(|track| {
                !track
                    .clips()
                    .iter()
                    .any(|c| c.timeline.start < end && c.timeline.end > start)
            })
            .map(|track| track.id)
            .take(parts.len())
            .collect();
        let existing = sequence.text_tracks.len();
        let mut new_lanes = Vec::new();
        while lanes.len() < parts.len() {
            let id = TrackId::new();
            new_lanes.push((id, format!("Title {}", existing + new_lanes.len() + 1)));
            lanes.push(id);
        }

        let ids: Vec<ClipId> = parts.iter().map(|p| p.id).collect();
        self.staged("Add Lower Third", |editor, stage| {
            for (id, name) in new_lanes {
                editor.stage(
                    stage,
                    Command::AddTrack {
                        sequence: sequence_id,
                        kind: TrackKindRepr::Text,
                        name,
                        id,
                    },
                )?;
            }
            for (clip, track) in parts.into_iter().zip(lanes) {
                editor.stage(
                    stage,
                    Command::AddText {
                        sequence: sequence_id,
                        track,
                        clip: Box::new(clip),
                    },
                )?;
            }
            Ok(())
        })?;
        Ok(ids)
    }
}
