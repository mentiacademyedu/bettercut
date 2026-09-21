//! Starting a bounce from the interface (`editor_core::bounce`).
//!
//! The same shape as `crate::render`: the shell owns the scheduler, so what
//! the interface can do is build the job — a copy of the project, the folder
//! made, the file named — and hand it over.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TrackId;
use bettercut_export::BounceJob;

/// A bounce of `track`, ready to submit.
///
/// The copy it mixes from is the project as it stands; the lane is silenced
/// down to itself inside the job, where the reason for it is written.
pub fn bounce_job(editor: &Editor, track: TrackId) -> Result<BounceJob, String> {
    if !editor.can_bounce(track) {
        return Err("That lane is locked, empty, or is not a sound lane".to_owned());
    }
    let (project, sequence) = editor.export_copy(None).map_err(|err| err.to_string())?;
    let path = editor.bounce_file(track);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|err| format!("Could not make the bounces folder: {err}"))?;
    }
    BounceJob::new(&project, sequence, track, path)
        .ok_or_else(|| "There is nothing on that lane to bounce".to_owned())
}
