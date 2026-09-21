//! Matching the mix to a platform's loudness target.
//!
//! The measurement is [`bettercut_playback::loudness_mix::measure`], which
//! mixes the whole sequence through the same mixer the preview and the export
//! use — so what is measured is what will be delivered (§46).
//!
//! Done here and now rather than as a background job, for the reason the
//! motion tracker gives: it is seconds of work on a normal edit, once, when
//! asked for, and a job queue for it would be more machinery than the wait.

use bettercut_editor_core::Editor;

use crate::state::UiState;

/// Measure the mix and set the master volume so it lands on `target` LUFS.
pub fn match_to(editor: &mut Editor, state: &mut UiState, target: f32) {
    let threads = editor
        .hardware()
        .ffmpeg_threads_per_job(editor.project().settings.performance_mode);
    let measured = editor.active_sequence().and_then(|sequence| {
        bettercut_playback::loudness_mix::measure(editor.project(), sequence, threads)
    });
    let Some(lufs) = measured else {
        state.loudness_measured = None;
        state.error("There is nothing to hear in this edit yet");
        return;
    };
    state.loudness_measured = Some(lufs);
    match editor.match_loudness(lufs, target) {
        Ok(volume) => state.info(format!(
            "Measured {lufs:.1} LUFS; master volume now {:.0}%",
            volume * 100.0
        )),
        Err(err) => state.error(err.to_string()),
    }
    state.needs_repaint = true;
}
