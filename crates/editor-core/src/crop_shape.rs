//! Crop a clip to a shape in one click: a square out of a landscape shot, a
//! vertical strip for a phone frame — centred, at full size, as the crop tools
//! have it everywhere else (`bettercut_timeline::crop_to_aspect`).

use bettercut_foundation::ClipId;
use bettercut_timeline::{Crop, crop_to_aspect};

use crate::command::ClipProperty;
use crate::editor::Editor;
use crate::error::EditorError;

/// The shapes offered, as (label, width, height).
pub const CROP_SHAPES: [(&str, u32, u32); 4] = [
    ("1:1", 1, 1),
    ("4:5", 4, 5),
    ("9:16", 9, 16),
    ("16:9", 16, 9),
];

impl Editor {
    /// The crop that cuts picture clip `clip` to `shape` (width, height), or
    /// takes the crop off for `None`. `None` when the clip's picture size is
    /// not known.
    pub fn crop_for_shape(&self, clip: ClipId, shape: Option<(u32, u32)>) -> Option<Crop> {
        let video = self.video_clip(clip)?;
        let Some((w, h)) = shape else {
            return Some(Crop::NONE);
        };
        let asset = self.project().media_asset(video.media_id)?;
        if asset.width == 0 || asset.height == 0 || w == 0 || h == 0 {
            return None;
        }
        Some(crop_to_aspect(
            asset.width as f32 / asset.height as f32,
            w as f32 / h as f32,
        ))
    }

    /// Crop `clip` to `shape`, centred, as one undo step; `None` uncrops it.
    pub fn crop_to_shape(
        &mut self,
        clip: ClipId,
        shape: Option<(u32, u32)>,
    ) -> Result<(), EditorError> {
        if self.video_clip(clip).is_none() {
            return Err(EditorError::ClipKindMismatch);
        }
        let crop = self
            .crop_for_shape(clip, shape)
            .ok_or(EditorError::UnknownPictureSize)?;
        self.set_clip_property(clip, ClipProperty::Crop(crop), false)
    }

    /// Which of [`CROP_SHAPES`] the clip is cropped to right now, if any.
    pub fn crop_shape_of(&self, clip: ClipId) -> Option<(u32, u32)> {
        let current = self.video_clip(clip)?.crop;
        if current.is_none() {
            return None;
        }
        CROP_SHAPES.iter().find_map(|(_, w, h)| {
            let crop = self.crop_for_shape(clip, Some((*w, *h)))?;
            let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
            (near(crop.left, current.left)
                && near(crop.right, current.right)
                && near(crop.top, current.top)
                && near(crop.bottom, current.bottom))
            .then_some((*w, *h))
        })
    }
}
