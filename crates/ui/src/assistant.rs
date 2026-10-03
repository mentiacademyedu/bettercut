//! Connecting an AI assistant: how to set one up, and whether one is here.
//!
//! The editing itself is `bettercut-mcp`'s (an MCP server installed beside
//! the app) and the live link the desktop shell serves. This window is for
//! the person who has never typed `claude mcp add`: the exact command for
//! this install, ready to copy, and a line saying whether an assistant has
//! been heard from.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::state::UiState;
use crate::theme;

/// How recently an assistant must have called to count as connected.
const CONNECTED_FOR: Duration = Duration::from_secs(120);

/// The program an assistant runs, and what to pass it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// Where the MCP server is: `bettercut-mcp` beside the app — or, inside an
/// AppImage, whose own files are only there while it runs, the AppImage
/// itself with `--mcp`.
pub fn server() -> Option<Server> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Some(Server {
            program: PathBuf::from(appimage),
            args: vec!["--mcp".to_owned()],
        });
    }
    let app = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "bettercut-mcp.exe"
    } else {
        "bettercut-mcp"
    };
    Some(Server {
        program: app.parent()?.join(name),
        args: Vec::new(),
    })
}

/// The Claude Code command that adds `server`.
pub fn claude_code_command(server: &Server) -> String {
    let mut command = format!(
        "claude mcp add bettercut -- \"{}\"",
        server.program.display()
    );
    for arg in &server.args {
        command.push(' ');
        command.push_str(arg);
    }
    command
}

/// The block Claude Desktop (and most other clients) take in their settings.
pub fn desktop_config(server: &Server) -> String {
    let mut entry = serde_json::json!({ "command": server.program.display().to_string() });
    if !server.args.is_empty() {
        entry["args"] = serde_json::json!(server.args);
    }
    let config = serde_json::json!({ "mcpServers": { "bettercut": entry } });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

/// What to say about the connection, given when an assistant last called.
pub fn connection(last_heard: Option<Instant>, now: Instant) -> &'static str {
    match last_heard {
        Some(at) if now.saturating_duration_since(at) <= CONNECTED_FOR => {
            "An assistant is connected to this window."
        }
        Some(_) => "An assistant was connected earlier; it has been quiet for a while.",
        None => "No assistant has connected yet. Once one is set up, ask it to attach to the app.",
    }
}

/// The window, while it is open.
pub fn show(ctx: &egui::Context, state: &mut UiState) {
    if !state.assistant_open {
        return;
    }
    let mut open = true;
    let server = server();
    egui::Window::new("Connect an AI Assistant")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(520.0, 0.0))
        .show(ctx, |ui| {
            ui.label(
                "An assistant such as Claude can edit with you: cut, title, colour, caption and \
                 export, in this window as you watch, every change undoable like your own.",
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(connection(state.assistant_seen, Instant::now())).strong(),
            );
            ui.add_space(8.0);
            let Some(server) = server else {
                ui.label("The assistant program could not be found beside the app.");
                return;
            };
            if !server.program.exists() {
                ui.label(
                    egui::RichText::new(format!(
                        "bettercut-mcp is not beside the app ({}). Install bettercut from its \
                         installer to get it.",
                        server.program.display()
                    ))
                    .color(theme::disabled()),
                );
            }
            theme::section(ui, "Claude Code");
            let command = claude_code_command(&server);
            ui.label("Run this once in a terminal:");
            copyable(ui, state, &command);
            ui.add_space(6.0);
            theme::section(ui, "Claude Desktop and other apps");
            ui.label("Add this to the app's MCP server settings (claude_desktop_config.json):");
            copyable(ui, state, &desktop_config(&server));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "Then ask it to \"attach to the bettercut app\". It connects only on this \
                     computer, with a key this window makes each time it starts.",
                )
                .color(theme::disabled()),
            );
        });
    if !open {
        state.assistant_open = false;
        state.needs_repaint = true;
    }
}

/// Text in a monospace box with a Copy button beside it.
fn copyable(ui: &mut egui::Ui, state: &mut UiState, text: &str) {
    ui.horizontal(|ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_max_width(420.0);
            ui.add(egui::Label::new(egui::RichText::new(text).monospace()).wrap());
        });
        if ui.button("Copy").clicked() {
            state.copy_out = Some(text.to_owned());
            state.info("Copied");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_name_the_server_beside_the_app() {
        let server = Server {
            program: PathBuf::from("C:\\Program Files\\bettercut\\bettercut-mcp.exe"),
            args: Vec::new(),
        };
        let server = &server;
        assert_eq!(
            claude_code_command(server),
            "claude mcp add bettercut -- \"C:\\Program Files\\bettercut\\bettercut-mcp.exe\""
        );
        let config: serde_json::Value =
            serde_json::from_str(&desktop_config(server)).unwrap_or_default();
        assert_eq!(
            config["mcpServers"]["bettercut"]["command"],
            "C:\\Program Files\\bettercut\\bettercut-mcp.exe"
        );
        assert!(config["mcpServers"]["bettercut"]["args"].is_null());
    }

    #[test]
    fn an_appimage_is_run_with_the_flag() {
        let server = Server {
            program: PathBuf::from("/home/me/bettercut-0.4.0-x86_64.AppImage"),
            args: vec!["--mcp".to_owned()],
        };
        assert_eq!(
            claude_code_command(&server),
            "claude mcp add bettercut -- \"/home/me/bettercut-0.4.0-x86_64.AppImage\" --mcp"
        );
        let config: serde_json::Value =
            serde_json::from_str(&desktop_config(&server)).unwrap_or_default();
        assert_eq!(
            config["mcpServers"]["bettercut"]["args"],
            serde_json::json!(["--mcp"])
        );
    }

    #[test]
    fn connected_means_heard_from_lately() {
        let now = Instant::now();
        assert!(connection(None, now).starts_with("No assistant"));
        assert!(connection(Some(now), now).contains("is connected"));
        if let Some(long_ago) = now.checked_sub(Duration::from_secs(600)) {
            assert!(connection(Some(long_ago), now).contains("earlier"));
        }
    }
}
