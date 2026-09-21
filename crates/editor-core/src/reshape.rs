//! One edit, several shapes: the copies an export writes alongside the main
//! file (a landscape video and its vertical and square cuts, in one go).
//!
//! # A copy, not an edit
//!
//! The extra shapes are built from a *clone* of the project with the sequence
//! reshaped, handed straight to the export. Nothing is dispatched, so nothing
//! lands in the history and the edit on screen does not change shape under the
//! user — which is what "also export as vertical" means. Doing it by reshaping
//! the real sequence, exporting, and reshaping back would put three steps in
//! the undo list and race the export job for the project.
//!
//! # How each shot is framed in the copy
//!
//! Exactly as the sequence panel's hint counts them
//! ([`Editor::clips_showing_bars`]): a clip still at the framing it was placed
//! with — full size, centred, upright, uncropped, not animated — gets §33's
//! auto crop to the new shape, so a landscape shot fills a vertical frame
//! without being enlarged. A clip the user framed by hand (shrunk into a
//! corner, cropped, moved, animated) is left exactly as it is: that framing
//! was a decision, and there is no way to guess what the same decision would
//! be at another shape.

use bettercut_foundation::{MediaId, SequenceId};
use bettercut_project_format::Project;
use bettercut_timeline::{Resolution, Sequence};

use crate::command::Command;

use crate::editor::{Editor, aspect_of, at_placed_framing};
use crate::error::EditorError;

/// A frame shape the interface offers, as width:height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// "16:9", as the shape buttons are labelled.
    pub label: &'static str,
    pub ratio: (u32, u32),
    /// What it is for, for the hover text.
    pub hint: &'static str,
    /// Safe in a file name, where a colon is not.
    pub file_suffix: &'static str,
}

/// Every shape offered, landscape first.
pub const SHAPES: [Shape; 5] = [
    Shape {
        label: "16:9",
        ratio: (16, 9),
        hint: "Landscape — YouTube, television, most cameras",
        file_suffix: "16x9",
    },
    Shape {
        label: "9:16",
        ratio: (9, 16),
        hint: "Vertical — Shorts, TikTok, Reels",
        file_suffix: "9x16",
    },
    Shape {
        label: "1:1",
        ratio: (1, 1),
        hint: "Square — feed posts",
        file_suffix: "1x1",
    },
    Shape {
        label: "4:5",
        ratio: (4, 5),
        hint: "Portrait — Instagram feed",
        file_suffix: "4x5",
    },
    Shape {
        label: "21:9",
        ratio: (21, 9),
        hint: "Ultrawide — cinematic",
        file_suffix: "21x9",
    },
];

impl Shape {
    /// Whether `size` is this shape.
    pub fn matches(self, size: Resolution) -> bool {
        matches_aspect(size, self.ratio)
    }

    /// `size` reshaped to this shape, keeping its short edge.
    pub fn applied_to(self, size: Resolution) -> Resolution {
        with_aspect(size, self.ratio)
    }
}

/// Whether `size` is `ratio` (width, height), to a percent.
///
/// Compared as a ratio rather than by exact dimensions: 1920×1080 and 1280×720
/// are both 16:9, and someone who typed 1918×1080 has still chosen landscape.
pub fn matches_aspect(size: Resolution, ratio: (u32, u32)) -> bool {
    if size.height == 0 || ratio.1 == 0 {
        return false;
    }
    let actual = f64::from(size.width) / f64::from(size.height);
    let wanted = f64::from(ratio.0) / f64::from(ratio.1);
    (actual - wanted).abs() < 0.01
}

/// `size` reshaped to `ratio`, keeping its short edge — 1920×1080 as 9:16 is
/// 1080×1920, not something smaller both ways. Always even (§36): 4:2:0 chroma
/// has no representation for an odd dimension.
pub fn with_aspect(size: Resolution, ratio: (u32, u32)) -> Resolution {
    let short = u64::from(size.width.min(size.height).max(2));
    let (num, den) = (u64::from(ratio.0.max(1)), u64::from(ratio.1.max(1)));
    let (w, h) = if num >= den {
        (short * num / den, short)
    } else {
        (short, short * den / num)
    };
    Resolution::new((w as u32).max(2) & !1, (h as u32).max(2) & !1)
}

impl Editor {
    /// A copy of the project for an export to read, with the active sequence
    /// reshaped to `shape` — or as it stands, for `None`. The editor itself is
    /// not changed.
    ///
    /// Returns the copy and the sequence in it to export.
    pub fn export_copy(&self, shape: Option<Shape>) -> Result<(Project, SequenceId), EditorError> {
        self.export_copy_of(None, shape)
    }

    /// [`Self::export_copy`] for any sequence in the project: `None` is the
    /// one on screen.
    pub fn export_copy_of(
        &self,
        sequence: Option<SequenceId>,
        shape: Option<Shape>,
    ) -> Result<(Project, SequenceId), EditorError> {
        let sequence_id = match sequence {
            Some(id) => {
                self.project()
                    .sequence(id)
                    .ok_or(EditorError::SequenceNotFound(id))?;
                id
            }
            None => self.active_sequence_id()?,
        };
        let mut project = self.project().clone();
        let Some(shape) = shape else {
            return Ok((project, sequence_id));
        };
        let media = media_aspects(&project);
        let sequence = project
            .sequence_mut(sequence_id)
            .ok_or(EditorError::SequenceNotFound(sequence_id))?;
        reshape(sequence, &media, shape);
        Ok((project, sequence_id))
    }

    /// Copy `sequence` at another shape — a vertical cut of a landscape edit —
    /// right after it, and switch to the copy. One undo step.
    ///
    /// Framed the way an export's extra shapes are: shots still at their
    /// placed framing are cropped to fill the new frame, anything framed by
    /// hand is left as it was. The original is untouched.
    pub fn copy_sequence_as(
        &mut self,
        sequence: SequenceId,
        shape: Shape,
    ) -> Result<SequenceId, EditorError> {
        let (mut copy, index) = self.sequence_copy(sequence)?;
        let name = &self.project().sequences[index].name;
        copy.name = format!("{name} {}", shape.label);
        reshape(&mut copy, &media_aspects(self.project()), shape);
        let id = copy.id;
        self.dispatch(Command::AddSequence {
            sequence: Box::new(copy),
            index: index + 1,
        })?;
        self.switch_sequence(id);
        Ok(id)
    }
}

/// Every file's width over height, where it is known.
fn media_aspects(project: &Project) -> Vec<(MediaId, Option<f32>)> {
    project
        .media
        .iter()
        .map(|asset| (asset.id, aspect_of(asset.width, asset.height)))
        .collect()
}

/// Give `sequence` the shape `shape`, cropping every shot still at its placed
/// framing to fill it.
fn reshape(sequence: &mut Sequence, media: &[(MediaId, Option<f32>)], shape: Shape) {
    sequence.resolution = shape.applied_to(sequence.resolution);
    let Some(output) = aspect_of(sequence.resolution.width, sequence.resolution.height) else {
        return;
    };

    for track in &mut sequence.video_tracks {
        // By id: a track hands out its clips mutably one at a time, so its
        // ordering cannot be broken from outside. A crop does not move one.
        let ids: Vec<_> = track.clips().iter().map(|clip| clip.id).collect();
        for id in ids {
            let Some(clip) = track.get_mut(id) else {
                continue;
            };
            if !at_placed_framing(clip) {
                continue;
            }
            let Some(source) = media
                .iter()
                .find(|(id, _)| *id == clip.media_id)
                .and_then(|(_, aspect)| *aspect)
            else {
                continue; // missing media, or a size never learned
            };
            clip.crop = bettercut_timeline::crop_to_aspect(source, output);
        }
    }
}
