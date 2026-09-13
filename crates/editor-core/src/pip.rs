//! Picture in picture: one shot shrunk into a corner, over another.
//!
//! The reaction face over the gameplay, the presenter over the slides. The
//! inset keeps its whole picture — nothing is cropped — at a quarter, a third
//! or two fifths of the frame, a fixed margin in from the corner. Like
//! a split screen it is made of the clip's own scale and position, so it can be
//! nudged by hand afterwards, and "Full Frame" puts the clip back.
//!
//! The inset only shows over something when it is on a higher track than the
//! shot it sits on — lanes composite bottom up (§22). That is the usual setup
//! for an overlay, so the clip is left on its lane rather than moved.

use bettercut_foundation::ClipId;
use bettercut_timeline::{AnimatedParameter, Crop, fit_scale};

use crate::command::{ClipProperty, Command};
use crate::editor::Editor;
use crate::error::EditorError;

/// The corner an inset sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl PipCorner {
    /// In reading order, which is also how the menu lays them out: two by two.
    pub const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top left",
            Self::TopRight => "Top right",
            Self::BottomLeft => "Bottom left",
            Self::BottomRight => "Bottom right",
        }
    }
}

/// How big an inset is: the share of the frame its longer side takes, along
/// that side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PipSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl PipSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }

    /// The inset's width as a fraction of the frame's — or its height as a
    /// fraction of the frame's, whichever is the larger share.
    pub fn share(self) -> f32 {
        match self {
            Self::Small => 0.25,
            Self::Medium => 1.0 / 3.0,
            Self::Large => 0.4,
        }
    }
}

/// The gap between an inset and the frame's edges, as a fraction of the
/// frame's height — the same number of pixels on both sides of the corner.
pub const PIP_MARGIN: f32 = 0.04;

/// The scale and position that put a picture of `source_aspect` in `corner` of
/// a frame of `output_aspect`, at `size`. Position is the offset of the
/// picture's centre from the frame's, in frame fractions, down positive — the
/// clip's own position control.
pub fn inset_frame(
    source_aspect: f32,
    output_aspect: f32,
    corner: PipCorner,
    size: PipSize,
) -> (f32, [f32; 2]) {
    // At scale 1 the picture is fitted to the frame, filling it along one
    // side; one factor takes that side to the inset's share. Sizing by the
    // width alone would make a portrait shot in a landscape frame taller than
    // the frame.
    let (fit_x, fit_y) = fit_scale(source_aspect, output_aspect);
    let scale = size.share() / fit_x.max(fit_y);
    let (width, height) = (fit_x * scale, fit_y * scale);
    let margin_x = PIP_MARGIN / output_aspect;
    let x = 0.5 - margin_x - width / 2.0;
    let y = 0.5 - PIP_MARGIN - height / 2.0;
    let (x, y) = match corner {
        PipCorner::TopLeft => (-x, -y),
        PipCorner::TopRight => (x, -y),
        PipCorner::BottomLeft => (-x, y),
        PipCorner::BottomRight => (x, y),
    };
    (scale, [x, y])
}

impl Editor {
    /// Shrink `clip` into `corner` at `size`, as one undo step.
    ///
    /// Any crop is cleared, so the whole shot shows. Refused, changing nothing,
    /// for a clip whose position or scale is animated — the keys would move it
    /// straight back out.
    pub fn apply_picture_in_picture(
        &mut self,
        clip: ClipId,
        corner: PipCorner,
        size: PipSize,
    ) -> Result<(), EditorError> {
        let output_aspect = self.output_aspect()?;
        let source_aspect = self.source_aspect(clip, output_aspect)?;
        let (scale, [x, y]) = inset_frame(source_aspect, output_aspect, corner, size);
        self.set_framing(
            clip,
            Crop::NONE,
            scale,
            [x, y],
            format!("Picture in Picture: {}", corner.label()),
        )
    }

    /// Back to filling the frame: no crop, normal size, centred. One undo step.
    pub fn reset_to_full_frame(&mut self, clip: ClipId) -> Result<(), EditorError> {
        self.set_framing(clip, Crop::NONE, 1.0, [0.0, 0.0], "Full Frame".to_owned())
    }

    /// The corner and size `clip` is inset at, if it is exactly one of them.
    pub fn picture_in_picture_of(&self, clip: ClipId) -> Option<(PipCorner, PipSize)> {
        let output_aspect = self.output_aspect().ok()?;
        let source_aspect = self.source_aspect(clip, output_aspect).ok()?;
        let video = self.video_clip(clip)?;
        if !video.crop.is_none() {
            return None;
        }
        let t = video.transform;
        let near = |a: f32, b: f32| (a - b).abs() < 0.001;
        PipSize::ALL.into_iter().find_map(|size| {
            PipCorner::ALL.into_iter().find_map(|corner| {
                let (scale, [x, y]) = inset_frame(source_aspect, output_aspect, corner, size);
                (near(t.scale.x, scale)
                    && near(t.scale.y, scale)
                    && near(t.position.x, x)
                    && near(t.position.y, y))
                .then_some((corner, size))
            })
        })
    }

    fn output_aspect(&self) -> Result<f32, EditorError> {
        let sequence = self.active_sequence_id()?;
        let resolution = self
            .project()
            .sequence(sequence)
            .ok_or(EditorError::SequenceNotFound(sequence))?
            .resolution;
        Ok(resolution.width as f32 / resolution.height.max(1) as f32)
    }

    fn source_aspect(&self, clip: ClipId, output_aspect: f32) -> Result<f32, EditorError> {
        let video = self.video_clip(clip).ok_or(EditorError::ClipKindMismatch)?;
        Ok(self
            .project()
            .media_asset(video.media_id)
            .filter(|asset| asset.width > 0 && asset.height > 0)
            .map_or(output_aspect, |asset| {
                asset.width as f32 / asset.height as f32
            }))
    }

    fn set_framing(
        &mut self,
        clip: ClipId,
        crop: Crop,
        scale: f32,
        [x, y]: [f32; 2],
        label: String,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let video = self.video_clip(clip).ok_or(EditorError::ClipKindMismatch)?;
        let animated = [
            AnimatedParameter::ScaleX,
            AnimatedParameter::ScaleY,
            AnimatedParameter::PositionX,
            AnimatedParameter::PositionY,
        ]
        .into_iter()
        .any(|parameter| video.keyframes.is_animated(parameter));
        if animated {
            return Err(EditorError::AlreadyAnimated("position or scale"));
        }
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;
        let commands = [
            ClipProperty::Crop(crop),
            ClipProperty::Scale { x: scale, y: scale },
            ClipProperty::Position { x, y },
        ]
        .into_iter()
        .map(|property| Command::SetClipProperty {
            sequence,
            track,
            clip,
            property,
        })
        .collect();
        self.dispatch_group(label, commands)
    }
}
