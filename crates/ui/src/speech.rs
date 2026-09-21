//! Text to speech: a title read aloud by one of the voices built into Windows,
//! saved as a WAV and put on a sound lane like a recorded voiceover.
//!
//! Through PowerShell's `System.Speech`, which every Windows install has, so
//! there is nothing to download and nothing to bundle. The words go in on
//! standard input rather than on the command line, so no quote or dollar sign
//! in a title can break out of the script. Speaking takes a second or two, so
//! it runs on its own thread and the editor carries on.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;

use crate::state::UiState;

/// The longest text read in one go, in characters.
pub const MAX_SPEECH_CHARS: usize = 5_000;

/// The voices installed on this machine, by name.
pub fn voices() -> Result<Vec<String>, String> {
    let output = powershell(
        "Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $s.GetInstalledVoices() | Where-Object { $_.Enabled } | ForEach-Object { $_.VoiceInfo.Name }",
        None,
    )?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Speak `text` into a WAV file at `path`, in `voice` (the default voice for
/// `None`) at `rate`, -10 (slow) to 10 (fast). Blocks until it is written.
pub fn synthesize(text: &str, voice: Option<&str>, rate: i32, path: &Path) -> Result<(), String> {
    let text: String = text.trim().chars().take(MAX_SPEECH_CHARS).collect();
    if text.is_empty() {
        return Err("There are no words to read".to_owned());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // The voice name and the path are not typed by the user as free text into
    // the script: the voice comes from `voices()`, and both are passed as
    // single-quoted PowerShell literals with any quote doubled.
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let choose = voice.map_or(String::new(), |v| format!("$s.SelectVoice({}); ", quote(v)));
    let script = format!(
        "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; \
         Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         {choose}$s.Rate = {rate}; \
         $s.SetOutputToWaveFile({path}); \
         $words = [Console]::In.ReadToEnd(); \
         $s.Speak($words); \
         $s.Dispose()",
        rate = rate.clamp(-10, 10),
        path = quote(&path.display().to_string()),
    );
    powershell(&script, Some(&text))?;
    if !path.exists() {
        return Err("The voice did not produce a file".to_owned());
    }
    Ok(())
}

fn powershell(script: &str, input: Option<&str>) -> Result<String, String> {
    if !cfg!(windows) {
        return Err("Reading aloud needs the voices built into Windows".to_owned());
    }
    let mut command = Command::new("powershell");
    // A windowed app starting a console program gets a console window flashed
    // up for it; this one has nothing to show.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start the speech engine: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // UTF-8 with no byte-order mark; PowerShell reads it as the console's
        // input. Dropping the handle closes it, which ends `ReadToEnd`.
        let _ = stdin.write_all(input.unwrap_or("").as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("The speech engine stopped: {e}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let first = message
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error");
        return Err(format!("The speech engine failed: {}", first.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Where a spoken take is saved: beside the project's voiceovers, as
/// `Speech N.wav`.
pub fn next_speech_file(editor: &Editor) -> PathBuf {
    let folder = bettercut_editor_core::voiceover::voiceover_folder(editor.path());
    (1..)
        .map(|n| folder.join(format!("Speech {n}.wav")))
        .find(|path| !path.exists())
        .unwrap_or_else(|| folder.join("Speech.wav"))
}

/// Speech being made in the background, and the machine's voices once known.
#[derive(Default)]
pub struct SpeechJobs {
    making: Option<Receiver<Result<(PathBuf, TimelineTime), String>>>,
    voices: Option<Receiver<Vec<String>>>,
}

impl SpeechJobs {
    /// Start what was asked for, and finish what is done. Once per frame.
    pub fn sync(&mut self, editor: &mut Editor, state: &mut UiState) {
        // The voice list, once, in the background, and only once someone has
        // opened the menu that shows it: listing starts PowerShell.
        if state.speech_voices_wanted && state.speech_voices.is_none() && self.voices.is_none() {
            let (send, receive) = channel();
            std::thread::spawn(move || {
                let _ = send.send(voices().unwrap_or_default());
            });
            self.voices = Some(receive);
        }
        if let Some(receive) = &self.voices {
            match receive.try_recv() {
                Ok(list) => {
                    state.speech_voices = Some(list);
                    self.voices = None;
                }
                Err(TryRecvError::Disconnected) => {
                    state.speech_voices = Some(Vec::new());
                    self.voices = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }

        if let Some(request) = state.speech_request.take() {
            if self.making.is_some() {
                state.info("Still reading the last one aloud — try again in a moment");
            } else {
                let path = next_speech_file(editor);
                let (send, receive) = channel();
                std::thread::spawn(move || {
                    let result =
                        synthesize(&request.text, request.voice.as_deref(), request.rate, &path)
                            .map(|()| (path, request.at));
                    let _ = send.send(result);
                });
                self.making = Some(receive);
                state.info("Reading aloud…");
            }
        }

        if let Some(receive) = &self.making {
            match receive.try_recv() {
                Ok(result) => {
                    self.making = None;
                    match result.and_then(|(path, at)| {
                        editor.add_voiceover(&path, at).map_err(|e| e.to_string())
                    }) {
                        Ok(_) => {
                            state.info("Spoken voiceover added");
                            state.needs_repaint = true;
                        }
                        Err(err) => state.error(err),
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.making = None;
                    state.error("The speech engine stopped without an answer");
                }
                Err(TryRecvError::Empty) => state.needs_repaint = true,
            }
        }
    }
}

/// Words to read aloud, asked for from the interface.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeechRequest {
    pub text: String,
    /// A name from [`voices`], or `None` for the default.
    pub voice: Option<String>,
    /// -10 slow to 10 fast.
    pub rate: i32,
    /// Where the spoken take starts on the timeline.
    pub at: TimelineTime,
}
