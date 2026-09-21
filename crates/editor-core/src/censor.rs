//! Censor: covering part of a shot — a face, a number plate — with blocks.
//!
//! Built from what already exists rather than a new kind of layer: a copy of
//! the shot on the lane above, pixelated and cut down to an ellipse by a mask.
//! Because the copy is the same shot at the same time with the same framing
//! and movement, it lines up with the original exactly, and the blocks sit
//! over whatever the mask covers. Moving the mask moves the censored patch;
//! the pixelate slider sets how coarse it is; deleting the copy uncensors.
//!
//! One undo step, including the lane made for it when the lanes above are
//! busy.

use bettercut_foundation::ClipId;
use bettercut_timeline::{Clip, Mask, MaskShape, VideoTrack};

use crate::command::{ClipPayload, Command, TrackPayload};
use crate::editor::Editor;
use crate::error::EditorError;

/// How coarse a new censor's blocks are, 0–100.
pub const CENSOR_PIXELATE: f32 = 60.0;

/// How soft a new blurred censor is, on the blur control's 0–100 scale:
/// enough that a face or a plate cannot be made out.
pub const CENSOR_BLUR: f32 = 80.0;

/// How the covered part is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CensorStyle {
    /// Coarse square blocks.
    Blocks,
    /// A heavy blur: softer on the eye, as hidden.
    Blur,
}

impl CensorStyle {
    pub const ALL: [Self; 2] = [Self::Blocks, Self::Blur];

    pub fn label(self) -> &'static str {
        match self {
            Self::Blocks => "With Blocks",
            Self::Blur => "With Blur",
        }
    }
}

/// Where a new censor's patch sits and how big it is: an upright oval a
/// little above the middle, about the size of a face in a medium shot.
pub fn censor_mask() -> Mask {
    Mask {
        shape: MaskShape::Ellipse,
        center: [0.5, 0.42],
        size: [0.14, 0.2],
        feather: 0.08,
        rotation_degrees: 0.0,
        invert: false,
    }
}

impl Editor {
    /// Cover part of picture clip `clip` with blocks: a pixelated, masked copy
    /// on the lane above — a free one, or a new one made just above. One undo
    /// step. Returns the copy, which is what to select to place the patch.
    pub fn censor_clip(&mut self, clip: ClipId) -> Result<ClipId, EditorError> {
        self.censor_clip_with(clip, CensorStyle::Blocks)
    }

    /// [`Self::censor_clip`], hiding the part with `style`.
    pub fn censor_clip_with(
        &mut self,
        clip: ClipId,
        style: CensorStyle,
    ) -> Result<ClipId, EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let (index, original) = sequence
            .video_tracks
            .iter()
            .enumerate()
            .find_map(|(i, t)| t.get(clip).map(|c| (i, c.clone())))
            .ok_or(EditorError::ClipKindMismatch)?;

        let mut copy = original;
        copy.id = ClipId::new();
        copy.set_link(None);
        match style {
            CensorStyle::Blocks => copy.pixelate = CENSOR_PIXELATE,
            CensorStyle::Blur => {
                copy.pixelate = 0.0;
                copy.blur = CENSOR_BLUR;
            }
        }
        copy.mask = Some(censor_mask());
        let id = copy.id;
        let range = copy.timeline;

        // Tracks are stored bottom first: above is a higher index.
        let free = sequence.video_tracks[index + 1..]
            .iter()
            .find(|t| !t.locked && t.clips().iter().all(|c| !c.timeline.overlaps(range)))
            .map(|t| t.id);

        let command = match free {
            Some(track) => Command::AddClip {
                sequence: sequence_id,
                track,
                clip: ClipPayload::Video(Box::new(copy)),
            },
            None => {
                let mut lane = VideoTrack::new("Censor");
                lane.insert(copy)?;
                Command::InsertTrack {
                    sequence: sequence_id,
                    index: index + 1,
                    track: TrackPayload::Video(Box::new(lane)),
                }
            }
        };
        self.dispatch_group("Censor".to_owned(), vec![command])?;
        Ok(id)
    }
}
