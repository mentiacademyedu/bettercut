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

/// Where Claude Desktop keeps its settings, if it is installed here.
pub fn claude_desktop_config_file() -> Option<PathBuf> {
    let folder = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Claude")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?)
            .join("Library")
            .join("Application Support")
            .join("Claude")
    } else {
        return None;
    };
    folder
        .is_dir()
        .then(|| folder.join("claude_desktop_config.json"))
}

/// `existing` settings (or none yet) with bettercut added to their MCP
/// servers, everything else kept as it was. Refuses settings that are not a
/// JSON object rather than guess at them.
pub fn merge_desktop_config(existing: Option<&str>, server: &Server) -> Result<String, String> {
    let mut config = match existing.map(str::trim) {
        None | Some("") => serde_json::json!({}),
        Some(text) => serde_json::from_str::<serde_json::Value>(text).map_err(|err| {
            format!("Claude Desktop's settings are not valid JSON ({err}); add bettercut by hand")
        })?,
    };
    let Some(settings) = config.as_object_mut() else {
        return Err(
            "Claude Desktop's settings are not what was expected; add bettercut by hand".to_owned(),
        );
    };
    let servers = settings
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err(
            "Claude Desktop's mcpServers is not a list of servers; add bettercut by hand"
                .to_owned(),
        );
    };
    let added: serde_json::Value =
        serde_json::from_str(&desktop_config(server)).map_err(|e| e.to_string())?;
    servers.insert(
        "bettercut".to_owned(),
        added["mcpServers"]["bettercut"].clone(),
    );
    serde_json::to_string_pretty(&config).map_err(|e| e.to_string())
}

/// Add bettercut to Claude Desktop's settings, keeping a copy of them first.
pub fn add_to_claude_desktop(server: &Server) -> Result<String, String> {
    let file = claude_desktop_config_file().ok_or("Claude Desktop is not installed here")?;
    let existing = std::fs::read_to_string(&file).ok();
    let merged = merge_desktop_config(existing.as_deref(), server)?;
    if existing.is_some() {
        let backup = file.with_extension("json.before-bettercut");
        std::fs::copy(&file, &backup).map_err(|e| format!("could not keep a copy: {e}"))?;
    }
    std::fs::write(&file, merged).map_err(|e| format!("could not save: {e}"))?;
    Ok("Added to Claude Desktop. Quit and reopen Claude Desktop to use it.".to_owned())
}

/// Run `claude mcp add` for every project of this user, and say how it went.
pub fn add_to_claude_code(server: &Server) -> String {
    // npm installs `claude.cmd` on Windows, which is not found as `claude`.
    let names: &[&str] = if cfg!(windows) {
        &["claude", "claude.cmd"]
    } else {
        &["claude"]
    };
    for name in names {
        let mut command = std::process::Command::new(name);
        command
            .args(["mcp", "add", "--scope", "user", "bettercut", "--"])
            .arg(&server.program)
            .args(&server.args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window flashing up over the editor.
            command.creation_flags(0x0800_0000);
        }
        let Ok(output) = command.output() else {
            continue;
        };
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return if output.status.success() {
            "Added to Claude Code, for every folder you use it in.".to_owned()
        } else if said.contains("already exists") {
            "Claude Code already has bettercut.".to_owned()
        } else {
            format!("Claude Code said: {}", said.trim())
        };
    }
    "Claude Code was not found. Install it, or run the command below in a terminal.".to_owned()
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
            if let Some(note) = state.assistant_note.lock().ok().and_then(|n| n.clone()) {
                ui.label(egui::RichText::new(note).color(theme::accent_text()));
                ui.add_space(4.0);
            }
            theme::section(ui, "Claude Code");
            if ui
                .button("Add to Claude Code")
                .on_hover_text("Runs the command below for you")
                .clicked()
            {
                let note = std::sync::Arc::clone(&state.assistant_note);
                let ctx = ui.ctx().clone();
                let server = server.clone();
                if let Ok(mut n) = note.lock() {
                    *n = Some("Adding to Claude Code…".to_owned());
                }
                std::thread::spawn(move || {
                    let said = add_to_claude_code(&server);
                    if let Ok(mut n) = note.lock() {
                        *n = Some(said);
                    }
                    ctx.request_repaint();
                });
            }
            let command = claude_code_command(&server);
            ui.label("Or run this once in a terminal:");
            copyable(ui, state, &command);
            ui.add_space(6.0);
            theme::section(ui, "Claude Desktop and other apps");
            if claude_desktop_config_file().is_some()
                && ui
                    .button("Add to Claude Desktop")
                    .on_hover_text(
                        "Adds bettercut to Claude Desktop's settings, keeping a copy of them",
                    )
                    .clicked()
            {
                let said = add_to_claude_desktop(&server).unwrap_or_else(|err| err);
                if let Ok(mut n) = state.assistant_note.lock() {
                    *n = Some(said);
                }
            }
            ui.label("Or add this to the app's MCP server settings (claude_desktop_config.json):");
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
    fn claude_desktop_settings_keep_what_was_there() {
        let server = Server {
            program: PathBuf::from("/Applications/bettercut.app/Contents/MacOS/bettercut-mcp"),
            args: Vec::new(),
        };
        // Nothing yet: made from scratch.
        let made: serde_json::Value =
            serde_json::from_str(&merge_desktop_config(None, &server).unwrap_or_default())
                .unwrap_or_default();
        assert!(made["mcpServers"]["bettercut"]["command"].is_string());

        // Other servers and settings stay; an old bettercut entry is replaced.
        let before = r#"{ "theme": "dark", "mcpServers": {
            "files": { "command": "files-server" },
            "bettercut": { "command": "/old/place" } } }"#;
        let merged: serde_json::Value =
            serde_json::from_str(&merge_desktop_config(Some(before), &server).unwrap_or_default())
                .unwrap_or_default();
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["mcpServers"]["files"]["command"], "files-server");
        assert_eq!(
            merged["mcpServers"]["bettercut"]["command"],
            "/Applications/bettercut.app/Contents/MacOS/bettercut-mcp"
        );

        // Not JSON, or not shaped like settings: refused, never overwritten.
        assert!(merge_desktop_config(Some("{ not json"), &server).is_err());
        assert!(merge_desktop_config(Some("[1, 2]"), &server).is_err());
        assert!(merge_desktop_config(Some(r#"{"mcpServers": 3}"#), &server).is_err());
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
