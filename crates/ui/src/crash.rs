//! Crash reports, for testers who cannot be watched over the shoulder.
//!
//! A panic writes a report beside the editor's other per-user files — which
//! build, which system, the message, where it happened and the backtrace —
//! before the process goes. The next launch finds it and says the editor
//! closed unexpectedly, with the report to copy and its folder to open, so
//! a tester can send it on. Nothing is sent anywhere by the editor itself.
//!
//! Unsaved work is a separate matter, already handled: the recovery journal
//! offers it back on the same launch.

use std::path::{Path, PathBuf};

use crate::state::UiState;
use crate::theme;

/// Where crash reports go: beside the templates and the interface prefs.
pub fn crash_dir() -> PathBuf {
    bettercut_editor_core::templates::library::user_dir()
        .parent()
        .map_or_else(std::env::temp_dir, Path::to_path_buf)
        .join("crashes")
}

/// This run's log file: beside the crash reports, so both are in the one
/// folder a tester is pointed at. The run before is kept as
/// `bettercut.previous.log`.
pub fn log_file() -> PathBuf {
    crash_dir()
        .parent()
        .map_or_else(std::env::temp_dir, Path::to_path_buf)
        .join("logs")
        .join("bettercut.log")
}

/// The last `lines` lines of the file at `path`, or nothing when it cannot
/// be read — for a crash report, where the last thing logged is the clue.
pub fn tail(path: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// The report for a panic: `message` at `location`, with `backtrace`.
pub fn report(message: &str, location: &str, backtrace: &str, when_unix: u64) -> String {
    format!(
        "bettercut {} crashed\nsystem: {} {}\ntime (unix seconds): {when_unix}\n\nmessage: {message}\nat: {location}\n\nbacktrace:\n{backtrace}\n\nWhat I was doing when it happened:\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

/// Write crash reports for panics into `dir`, then let the panic go on as
/// before. Called once, at start.
pub fn install_hook(dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_owned());
        let location = info.location().map_or_else(
            || "(unknown)".to_owned(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let when = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut text = report(&message, &location, &backtrace, when);
        // What the editor was doing just before: the end of this run's log.
        let log = tail(&log_file(), 40);
        if !log.is_empty() {
            text.push_str("\nlast lines of the log:\n");
            text.push_str(&log);
            text.push('\n');
        }
        // Best effort: a crash while reporting a crash must not hide the
        // first one.
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(format!("crash-{when}.txt")), text);
        previous(info);
    }));
}

/// The newest report in `dir` not yet shown, with its text.
pub fn pending(dir: &Path) -> Option<(PathBuf, String)> {
    let newest = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with("crash-") && n.ends_with(".txt") && !n.ends_with(".seen.txt")
            })
        })
        .max()?;
    let text = std::fs::read_to_string(&newest).ok()?;
    Some((newest, text))
}

/// Mark a report as shown, so the next launch does not show it again. The
/// file stays, renamed, for whoever wants it later.
pub fn mark_seen(path: &Path) {
    let seen = path.with_extension("seen.txt");
    let _ = std::fs::rename(path, seen);
}

/// The window, when a report from last time is waiting.
pub fn show(ctx: &egui::Context, state: &mut UiState) {
    let Some((path, text)) = state.crash_report.clone() else {
        return;
    };
    let mut close = false;
    egui::Window::new("bettercut closed unexpectedly")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(460.0, 0.0))
        .show(ctx, |ui| {
            ui.label("Sorry — the editor crashed last time. A report was saved.");
            ui.label(
                egui::RichText::new(
                    "Sending it to us helps fix it: copy it into your message, and say what you were doing.",
                )
                .small()
                .color(theme::disabled()),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Copy Report").clicked() {
                    state.copy_out = Some(text.clone());
                    state.info("Crash report copied");
                }
                if ui.button("Open Folder").clicked()
                    && let Err(err) = crate::reveal::show_in_folder(&path)
                {
                    state.error(err);
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
    if close {
        mark_seen(&path);
        state.crash_report = None;
        state.needs_repaint = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_is_the_last_lines() {
        let path = std::env::temp_dir().join(format!("bettercut-tail-{}.log", std::process::id()));
        std::fs::write(&path, "one\ntwo\nthree\nfour\n").unwrap();
        assert_eq!(tail(&path, 2), "three\nfour");
        assert_eq!(tail(&path, 10), "one\ntwo\nthree\nfour");
        let _ = std::fs::remove_file(&path);
        assert_eq!(tail(&path, 2), "", "a missing log is no tail");
    }

    #[test]
    fn a_report_is_written_found_once_and_then_set_aside() {
        let dir = std::env::temp_dir().join(format!("bettercut-crash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(pending(&dir).is_none(), "nothing yet");

        let text = report("index out of bounds", "src/x.rs:3", "0: frame", 100);
        std::fs::write(dir.join("crash-100.txt"), &text).unwrap();
        std::fs::write(
            dir.join("crash-200.txt"),
            report("newer", "src/y.rs:9", "", 200),
        )
        .unwrap();

        let (path, found) = pending(&dir).unwrap();
        assert!(found.contains("newer"), "the newest report first: {found}");
        assert!(found.starts_with("bettercut "), "{found}");
        mark_seen(&path);
        let (_, next) = pending(&dir).unwrap();
        assert!(next.contains("index out of bounds"), "then the older one");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
