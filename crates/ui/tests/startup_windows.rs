//! The windows waiting at start-up come one at a time. Stacked, the welcome
//! window sat over the recovery prompt and neither could be read.

use bettercut_editor_core::{Editor, RecoverableSession, RecoveryPaths};
use bettercut_ui::{StartupWindow, UiState, startup_window};

fn pending_recovery() -> RecoverableSession {
    let (editor, _events) = Editor::new_project("Lost work");
    RecoverableSession {
        project: editor.project().clone(),
        paths: RecoveryPaths {
            dir: std::env::temp_dir().join("bettercut-startup-windows-test"),
        },
        replayed: 3,
        failed: 0,
    }
}

#[test]
fn the_most_urgent_window_goes_first_and_the_rest_wait() {
    let mut state = UiState::default();
    state.welcome_open = true;
    state.whats_new_open = true;
    state.crash_report = Some(("crash.txt".into(), "what happened".to_owned()));
    state.pending_recovery = Some(pending_recovery());
    assert_eq!(startup_window(&state), Some(StartupWindow::Recovery));

    // Each answered in turn lets the next one through.
    state.pending_recovery = None;
    assert_eq!(startup_window(&state), Some(StartupWindow::CrashReport));
    state.crash_report = None;
    assert_eq!(startup_window(&state), Some(StartupWindow::Welcome));
    state.welcome_open = false;
    assert_eq!(startup_window(&state), Some(StartupWindow::WhatsNew));
    state.whats_new_open = false;
    assert_eq!(startup_window(&state), None);
}

#[test]
fn a_window_waiting_is_still_shown_once_its_turn_comes() {
    // The welcome is not dismissed by having to wait: it stays asked for.
    let mut state = UiState::default();
    state.welcome_open = true;
    state.pending_recovery = Some(pending_recovery());
    assert_eq!(startup_window(&state), Some(StartupWindow::Recovery));
    assert!(state.welcome_open);
}
