//! Starting a render in place (`editor_core::render_in_place`).
//!
//! The bake is an ordinary export over a range with no sound, so the only
//! thing that needs writing is the setting-up: where the file goes, what the
//! edit under the stretch hashes to, and a copy of the project to render from.
//! It lives here rather than in the shell because the hash comes from
//! `bettercut_playback`, which the interface already has and the shell does
//! not.

use std::path::PathBuf;

use bettercut_editor_core::Editor;
use bettercut_editor_core::timeline::TimelineRange;
use bettercut_export::{ExportJob, ExportSettings};

/// A bake of `range`, ready to submit: the job, the file it will write, and
/// the hash of the edit it is a bake of.
///
/// The copy it renders from has no bakes of its own, so what is written is the
/// edit composited — not an older bake of the same stretch read and written
/// straight back out.
pub fn render_job(
    editor: &Editor,
    range: TimelineRange,
) -> Result<(ExportJob, PathBuf, u64), String> {
    let (mut project, sequence) = editor.export_copy(None).map_err(|err| err.to_string())?;
    if let Some(active) = project.sequence_mut(sequence) {
        active.renders.clear();
    }
    let path = editor.render_file(range);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|err| format!("Could not make the renders folder: {err}"))?;
    }

    let active = project
        .sequence(sequence)
        .ok_or_else(|| "That sequence is not there any more".to_owned())?;
    let fingerprint = bettercut_playback::rendered::fingerprint(&project, active, range);
    let mut settings = ExportSettings::for_sequence(path.clone(), active);
    settings.range = range;
    settings.picture_only = true;

    Ok((
        ExportJob::new(&project, sequence, settings),
        path,
        fingerprint,
    ))
}
