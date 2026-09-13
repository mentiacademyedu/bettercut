//! Split screens: two, three or four shots on screen at once, each in its own
//! part of the frame.
//!
//! # Cells, filled
//!
//! A layout divides the frame into cells, and each selected clip fills one:
//! cropped to the cell's shape (§22's crop, via [`crop_to_aspect`]) so it
//! covers the cell with no bars, then scaled and moved into place. Nothing is
//! stretched. The clips stay ordinary clips — the crop, scale and position are
//! the same controls the Inspector shows, so a split screen can be adjusted
//! afterwards by hand, and undone in one step.
//!
//! # Which clip goes where
//!
//! Reading order: the clip on the highest lane takes the first cell (left, or
//! top), and so on down. Clips on one lane are taken in timeline order. That is
//! the order the timeline shows them in, top to bottom, so what the user sees
//! stacked is what they get side by side.

use bettercut_foundation::ClipId;
use bettercut_timeline::{AnimatedParameter, crop_to_aspect, fit_scale};

use crate::command::{ClipProperty, Command};
use crate::editor::Editor;
use crate::error::EditorError;

/// A way of dividing the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitLayout {
    /// Two halves, left and right.
    SideBySide,
    /// Two halves, top and bottom — the shape for a vertical video.
    Stacked,
    /// Three columns.
    Thirds,
    /// Four quarters.
    Grid,
}

/// One cell: its left, top, width and height, as fractions of the frame.
pub type Cell = [f32; 4];

impl SplitLayout {
    pub const ALL: [Self; 4] = [Self::SideBySide, Self::Stacked, Self::Thirds, Self::Grid];

    pub fn label(self) -> &'static str {
        match self {
            Self::SideBySide => "Side by side",
            Self::Stacked => "Top and bottom",
            Self::Thirds => "Three columns",
            Self::Grid => "Four-way grid",
        }
    }

    /// The cells, in reading order.
    pub fn cells(self) -> &'static [Cell] {
        const THIRD: f32 = 1.0 / 3.0;
        match self {
            Self::SideBySide => &[[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 1.0]],
            Self::Stacked => &[[0.0, 0.0, 1.0, 0.5], [0.0, 0.5, 1.0, 0.5]],
            Self::Thirds => &[
                [0.0, 0.0, THIRD, 1.0],
                [THIRD, 0.0, THIRD, 1.0],
                [2.0 * THIRD, 0.0, THIRD, 1.0],
            ],
            Self::Grid => &[
                [0.0, 0.0, 0.5, 0.5],
                [0.5, 0.0, 0.5, 0.5],
                [0.0, 0.5, 0.5, 0.5],
                [0.5, 0.5, 0.5, 0.5],
            ],
        }
    }

    /// How many clips it takes.
    pub fn clips(self) -> usize {
        self.cells().len()
    }
}

/// How a picture of `source_aspect` is framed to fill `cell` of a frame of
/// `output_aspect`: its crop, its uniform scale, and its position (offset from
/// the frame's centre, in frame fractions).
pub fn frame_in_cell(
    source_aspect: f32,
    output_aspect: f32,
    cell: Cell,
) -> (bettercut_timeline::Crop, f32, [f32; 2]) {
    let [left, top, width, height] = cell;
    let cell_aspect = width / height * output_aspect;
    let crop = crop_to_aspect(source_aspect, cell_aspect);
    // Cropped to the cell's shape, the picture is fitted to the frame like any
    // other; one factor then brings that fitted size down to the cell's.
    let (fit_x, _) = fit_scale(cell_aspect, output_aspect);
    let scale = width / fit_x;
    let position = [left + width / 2.0 - 0.5, top + height / 2.0 - 0.5];
    (crop, scale, position)
}

impl Editor {
    /// Arrange `clips` in `layout`, one per cell, as one undo step.
    ///
    /// Refused, changing nothing, unless exactly as many picture clips are
    /// given as the layout has cells, or when one of them has its position or
    /// scale animated — the keys would override the layout.
    pub fn apply_split_screen(
        &mut self,
        clips: &[ClipId],
        layout: SplitLayout,
    ) -> Result<(), EditorError> {
        let sequence_id = self.active_sequence_id()?;
        let sequence = self
            .project()
            .sequence(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        let output_aspect =
            sequence.resolution.width as f32 / sequence.resolution.height.max(1) as f32;

        // Picture clips only, in reading order: highest lane first.
        let mut ordered: Vec<(usize, i64, ClipId)> = Vec::new();
        for (lane, track) in sequence.video_tracks.iter().enumerate() {
            for clip in track.clips() {
                if clips.contains(&clip.id) {
                    ordered.push((lane, clip.timeline.start.ticks(), clip.id));
                }
            }
        }
        if ordered.len() != layout.clips() {
            return Err(EditorError::SplitScreenCount {
                wanted: layout.clips(),
                given: ordered.len(),
            });
        }
        // Tracks are stored bottom first; the top lane reads first.
        ordered.sort_by_key(|(lane, start, _)| (std::cmp::Reverse(*lane), *start));

        let mut commands = Vec::new();
        for ((_, _, clip_id), cell) in ordered.iter().zip(layout.cells()) {
            let clip = self
                .video_clip(*clip_id)
                .ok_or(EditorError::ClipNotFound(*clip_id))?;
            let animated = [
                AnimatedParameter::ScaleX,
                AnimatedParameter::ScaleY,
                AnimatedParameter::PositionX,
                AnimatedParameter::PositionY,
            ]
            .into_iter()
            .any(|parameter| clip.keyframes.is_animated(parameter));
            if animated {
                return Err(EditorError::AlreadyAnimated("position or scale"));
            }
            let source_aspect = self
                .project()
                .media_asset(clip.media_id)
                .filter(|asset| asset.width > 0 && asset.height > 0)
                .map_or(output_aspect, |asset| {
                    asset.width as f32 / asset.height as f32
                });
            let (crop, scale, [x, y]) = frame_in_cell(source_aspect, output_aspect, *cell);
            let track = self
                .track_of(*clip_id)
                .ok_or(EditorError::ClipNotFound(*clip_id))?;
            for property in [
                ClipProperty::Crop(crop),
                ClipProperty::Scale { x: scale, y: scale },
                ClipProperty::Position { x, y },
            ] {
                commands.push(Command::SetClipProperty {
                    sequence: sequence_id,
                    track,
                    clip: *clip_id,
                    property,
                });
            }
        }
        self.dispatch_group(format!("Split Screen: {}", layout.label()), commands)
    }
}
