//! Recording a voiceover: the microphone runs while the edit plays, and what
//! was said lands on a sound lane where the playhead started.
//!
//! The recorder is held by the app shell, not by [`UiState`]: a microphone
//! stream is a device handle, not interface state. The Record button only
//! asks (`UiState::voiceover_toggle`); this answers once per frame.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;

use crate::state::UiState;

/// Shortest recording worth keeping: a click on Record then Stop.
const SHORTEST_SECONDS: f64 = 0.3;

#[derive(Default)]
pub struct VoiceoverRecorder {
    recording: Option<(bettercut_audio::Recorder, TimelineTime)>,
    /// Titles being read aloud (`crate::speech`), finished the same way.
    speech: crate::speech::SpeechJobs,
}

impl VoiceoverRecorder {
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Start or stop when asked, and keep the button's time and level current.
    pub fn sync(
        &mut self,
        editor: &mut Editor,
        state: &mut UiState,
        preview: Option<&mut crate::Preview>,
    ) {
        self.speech.sync(editor, state);
        if std::mem::take(&mut state.voiceover_toggle) {
            match self.recording.take() {
                None => match bettercut_audio::Recorder::start() {
                    Ok(recorder) => {
                        let at = editor.playhead();
                        state.info(format!(
                            "Recording from {} — press Stop when done",
                            recorder.device_name()
                        ));
                        self.recording = Some((recorder, at));
                        // The edit plays while you speak, so the words land
                        // on the pictures they are about.
                        if let Some(preview) = preview {
                            preview.set_playing(editor, true);
                        }
                    }
                    Err(err) => state.error(format!("Could not record: {err}")),
                },
                Some((recorder, at)) => {
                    if let Some(preview) = preview {
                        preview.set_playing(editor, false);
                    }
                    finish(editor, state, recorder.stop(), at);
                }
            }
        }

        state.voiceover_live = self
            .recording
            .as_ref()
            .map(|(recorder, _)| (recorder.seconds(), recorder.take_peak()));
        if state.voiceover_live.is_some() {
            state.needs_repaint = true;
        }
    }
}

/// Save a finished recording beside the project and put it on the timeline.
fn finish(
    editor: &mut Editor,
    state: &mut UiState,
    recording: bettercut_audio::Recording,
    at: TimelineTime,
) {
    if recording.seconds() < SHORTEST_SECONDS {
        state.info("Nothing recorded");
        return;
    }
    let folder = bettercut_editor_core::voiceover::voiceover_folder(editor.path());
    let file = bettercut_editor_core::voiceover::next_voiceover_file(&folder);
    if let Err(err) = recording.write_wav(&file) {
        state.error(format!("Could not save the recording: {err}"));
        return;
    }
    match editor.add_voiceover(&file, at) {
        Ok(_) => {
            state.info(format!(
                "Voiceover added ({:.1} s) — saved as {}",
                recording.seconds(),
                file.display()
            ));
            state.needs_repaint = true;
        }
        Err(err) => state.error(err.to_string()),
    }
}
