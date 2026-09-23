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
/// Measure each of `clips` on its own and bring them all to the loudness
/// of their middle one: the overall level stays where it was, and the
/// differences between them go.
pub fn even_out(
    editor: &mut Editor,
    state: &mut UiState,
    clips: &[bettercut_editor_core::foundation::ClipId],
) {
    let threads = editor
        .hardware()
        .ffmpeg_threads_per_job(editor.project().settings.performance_mode);
    let Some(sequence) = editor.active_sequence() else {
        return;
    };
    let mut measured: Vec<(bettercut_editor_core::foundation::ClipId, f32)> = clips
        .iter()
        .filter(|clip| editor.audio_clip(**clip).is_some())
        .filter_map(|clip| {
            bettercut_playback::loudness_mix::measure_clip(
                editor.project(),
                sequence,
                *clip,
                threads,
            )
            .map(|lufs| (*clip, lufs))
        })
        .collect();
    if measured.len() < 2 {
        state.info("Select two or more sound clips with something to hear");
        return;
    }
    let mut levels: Vec<f32> = measured.iter().map(|(_, lufs)| *lufs).collect();
    levels.sort_by(f32::total_cmp);
    let target = levels[levels.len() / 2];
    measured.sort_by(|a, b| a.1.total_cmp(&b.1));
    let spread = levels[levels.len() - 1] - levels[0];
    match editor.match_clips_loudness(&measured, target) {
        Ok(0) => state.info("They are already as loud as each other"),
        Ok(n) => {
            state.needs_repaint = true;
            state.info(format!(
                "Evened out {n} clip(s) to {target:.1} LUFS (they were {spread:.1} dB apart)"
            ));
        }
        Err(err) => state.error(err.to_string()),
    }
}

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
