//! Render in place: remembering which stretches of the edit are already baked
//! into a file (`bettercut_timeline::render`).
//!
//! The baking itself is an export — the same code, over a range, with no sound
//! ([`bettercut_export::ExportSettings::picture_only`]) — because a bake that
//! was made by a second renderer would be a second answer to "what does this
//! edit look like" (§46). What lives here is the bookkeeping either side of
//! it: where the file goes, that it is media now, and which stretch it stands
//! for.
//!
//! # Not a command
//!
//! A bake changes nothing about the edit; it is a faster way of playing the
//! edit that is already there. So it makes no undo step — pressing Ctrl+Z
//! after a render must undo the *edit* before it, not the render — and, like
//! importing media, it snapshots the journal so a recovered session keeps it.

use std::path::{Path, PathBuf};

use bettercut_foundation::{ClipId, MediaId};
use bettercut_timeline::{RenderedRange, TimelineRange};

use crate::editor::Editor;
use crate::error::EditorError;

/// Where bakes for the project at `project` are kept: a folder beside it, as
/// its voiceovers and its versions are, so moving the project folder takes its
/// renders with it and deleting the folder costs only rendering time.
///
/// Not the media cache: that is keyed by imported file and capped by the
/// setting in §67, and a bake belongs to one project's edit rather than to any
/// file in it. An unsaved project renders into the user's application data.
pub fn render_folder(project: Option<&Path>) -> PathBuf {
    match project {
        Some(project) => {
            let stem = project.file_stem().map_or_else(
                || "project".to_owned(),
                |s| s.to_string_lossy().into_owned(),
            );
            project.with_file_name(format!("{stem} renders"))
        }
        None => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(std::env::temp_dir)
            .join("bettercut")
            .join("renders"),
    }
}

impl Editor {
    /// Every baked stretch of the active sequence, whether or not still
    /// current.
    pub fn renders(&self) -> &[RenderedRange] {
        self.active_sequence()
            .map(|sequence| sequence.renders.as_slice())
            .unwrap_or_default()
    }

    /// Where a bake of `range` of the active sequence is written.
    ///
    /// The sequence and the range are both in the name, so two bakes cannot
    /// land on one another and re-baking a stretch writes over the file it
    /// replaces rather than leaving it behind.
    pub fn render_file(&self, range: TimelineRange) -> PathBuf {
        let sequence = self
            .active_sequence()
            .map(|sequence| sequence.id.as_uuid().simple().to_string())
            .unwrap_or_else(|| "sequence".to_owned());
        render_folder(self.path()).join(format!(
            "{sequence}-{:012}-{:012}.mp4",
            range.start.ticks().max(0),
            range.end.ticks().max(0)
        ))
    }

    /// Read a finished bake back in as media.
    ///
    /// Flagged [`MediaAsset::baked`], which is what keeps it out of the media
    /// browser and out of the proxy queue — it is a file this program wrote
    /// from the edit, not one the user brought to it.
    pub fn import_baked(&mut self, path: &Path) -> Result<MediaId, EditorError> {
        use bettercut_media::MediaProber;

        let mut asset = bettercut_media::FfmpegProber.probe(path)?;
        asset.baked = true;
        Ok(self.import_media(asset))
    }

    /// Remember that `range` of the active sequence is baked into `media`,
    /// which hashed to `fingerprint` when it was written.
    ///
    /// Replaces any bake of the same stretch: a second file for one stretch is
    /// two answers to the same question, and the older one is the wrong one.
    /// Not undoable — see the module note.
    pub fn record_render(
        &mut self,
        range: TimelineRange,
        media: MediaId,
        fingerprint: u64,
    ) -> Result<(), EditorError> {
        let sequence = self.active_sequence_id()?;
        let record = RenderedRange {
            range,
            media,
            clip: ClipId::new(),
            fingerprint,
        };
        let replaced = {
            let active = self
                .project_mut()
                .sequence_mut(sequence)
                .ok_or(EditorError::SequenceNotFound(sequence))?;
            let replaced = active
                .renders
                .iter()
                .position(|render| render.range == range)
                .map(|at| active.renders.remove(at));
            active.renders.push(record);
            replaced
        };
        // The file the old bake used is ours and nothing reads it now.
        if let Some(old) = replaced.filter(|old| old.media != media) {
            self.forget_baked_media(old.media);
        }
        self.after_render_change();
        Ok(())
    }

    /// Drop every bake of the active sequence, deleting the files.
    ///
    /// Only files this program wrote: an asset without [`MediaAsset::baked`]
    /// is something the user imported, and nothing here deletes those.
    /// Returns how many were dropped.
    pub fn clear_renders(&mut self) -> usize {
        let Ok(sequence) = self.active_sequence_id() else {
            return 0;
        };
        let dropped: Vec<RenderedRange> = self
            .project_mut()
            .sequence_mut(sequence)
            .map(|active| std::mem::take(&mut active.renders))
            .unwrap_or_default();
        for render in &dropped {
            self.forget_baked_media(render.media);
        }
        if !dropped.is_empty() {
            self.after_render_change();
        }
        dropped.len()
    }

    /// Drop bakes whose file has gone — the cache cleared, the machine
    /// changed, the folder tidied. Returns how many were dropped.
    ///
    /// Asked once when a project is opened rather than per frame: a stat call
    /// for every frame drawn would cost more than the compositing a bake
    /// saves.
    pub fn prune_renders(&mut self) -> usize {
        // Which baked files are no longer there, asked once for the project.
        let missing: Vec<MediaId> = self
            .project()
            .media
            .iter()
            .filter(|asset| asset.baked && !asset.path.exists())
            .map(|asset| asset.id)
            .collect();
        if missing.is_empty() {
            return 0;
        }

        let sequences: Vec<bettercut_foundation::SequenceId> =
            self.project().sequences.iter().map(|s| s.id).collect();
        let mut gone = 0;
        for id in sequences {
            let Some(sequence) = self.project_mut().sequence_mut(id) else {
                continue;
            };
            let before = sequence.renders.len();
            sequence
                .renders
                .retain(|render| !missing.contains(&render.media));
            gone += before - sequence.renders.len();
        }
        for media in &missing {
            self.forget_baked_media(*media);
        }
        self.after_render_change();
        gone
    }

    /// Take a baked file out of the project and off the disk.
    fn forget_baked_media(&mut self, media: MediaId) {
        let path = self
            .project()
            .media_asset(media)
            .filter(|asset| asset.baked)
            .map(|asset| asset.path.clone());
        let Some(path) = path else {
            return; // not ours, or already gone
        };
        if self.project_mut().remove_media(media).is_err() {
            return; // still used by a clip: leave both alone
        }
        if path.exists()
            && let Err(err) = std::fs::remove_file(&path)
        {
            tracing::warn!(%err, path = %path.display(), "could not delete a baked file");
        }
    }

    /// A bake is project state changed outside the command path, so it has to
    /// be folded into a fresh snapshot or a recovered session loses it — the
    /// same rule importing media follows.
    fn after_render_change(&mut self) {
        self.recorded_outside_a_command("recording a render");
    }
}
