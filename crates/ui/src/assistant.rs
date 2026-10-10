//! Connecting an AI assistant: how to set one up, and whether one is here.
//!
//! The editing itself is `bettercut-mcp`'s (an MCP server installed beside
//! the app) and the live link the desktop shell serves. This window is for
//! the person who has never edited an MCP settings file: for each assistant
//! app — Claude, Cursor, VS Code, Windsurf, Codex, Gemini CLI, Zed and more —
//! the exact setup for this install, ready to copy, a button that does it
//! where the app's settings are a file this can safely add to, and a line
//! saying whether an assistant has been heard from.

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

impl Server {
    /// The same server listing only its common tools (`--compact`), for an
    /// app that takes only so many tools.
    fn compact(&self) -> Self {
        let mut compact = self.clone();
        compact.args.push("--compact".to_owned());
        compact
    }
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

/// An assistant app bettercut can be added to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    ClaudeCode,
    ClaudeDesktop,
    Cursor,
    VsCode,
    Windsurf,
    Codex,
    GeminiCli,
    Zed,
    Kiro,
    LmStudio,
    OpenCode,
    Other,
}

/// How an app keeps its MCP servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Settings {
    /// A command it is added with (`claude mcp add`).
    Command,
    /// A JSON file with the servers under this key, each `{command, args}`.
    Json(&'static str),
    /// VS Code: `servers`, each also saying it is `stdio`.
    VsCode,
    /// opencode: `mcp`, each a `local` server whose command is one list.
    OpenCode,
    /// Codex: a TOML file, a `[mcp_servers.<name>]` table each.
    CodexToml,
    /// Zed: JSON with comments under `context_servers` — not a file this
    /// edits, since comments would be lost; the block is shown to paste.
    Zed,
}

impl App {
    pub const ALL: [App; 12] = [
        App::ClaudeCode,
        App::ClaudeDesktop,
        App::Cursor,
        App::VsCode,
        App::Windsurf,
        App::Codex,
        App::GeminiCli,
        App::Zed,
        App::Kiro,
        App::LmStudio,
        App::OpenCode,
        App::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            App::ClaudeCode => "Claude Code",
            App::ClaudeDesktop => "Claude Desktop",
            App::Cursor => "Cursor",
            App::VsCode => "VS Code (Copilot)",
            App::Windsurf => "Windsurf",
            App::Codex => "Codex",
            App::GeminiCli => "Gemini CLI",
            App::Zed => "Zed",
            App::Kiro => "Kiro",
            App::LmStudio => "LM Studio",
            App::OpenCode => "opencode",
            App::Other => "Another app",
        }
    }

    fn settings(self) -> Settings {
        match self {
            App::ClaudeCode => Settings::Command,
            App::VsCode => Settings::VsCode,
            App::Codex => Settings::CodexToml,
            App::Zed => Settings::Zed,
            App::OpenCode => Settings::OpenCode,
            App::ClaudeDesktop
            | App::Cursor
            | App::Windsurf
            | App::GeminiCli
            | App::Kiro
            | App::LmStudio
            | App::Other => Settings::Json("mcpServers"),
        }
    }

    /// Whether the app takes only so many tools — Cursor about 40 and
    /// Windsurf 100, for all its servers together — so is given the compact
    /// server, whose few tools reach the rest.
    pub fn wants_compact(self) -> bool {
        matches!(self, App::Cursor | App::Windsurf)
    }

    /// The server as this app should run it.
    pub fn server(self, server: &Server) -> Server {
        if self.wants_compact() {
            server.compact()
        } else {
            server.clone()
        }
    }

    /// The settings file this app keeps its servers in, from the user's home
    /// folder and (on Windows) roaming application data — whether or not the
    /// app is installed.
    fn settings_file_in(self, home: &std::path::Path, roaming: &std::path::Path) -> Option<PathBuf> {
        let code_user = || {
            if cfg!(windows) {
                roaming.join("Code").join("User")
            } else if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Code/User")
            } else {
                home.join(".config/Code/User")
            }
        };
        Some(match self {
            App::ClaudeDesktop => {
                let folder = if cfg!(windows) {
                    roaming.join("Claude")
                } else if cfg!(target_os = "macos") {
                    home.join("Library/Application Support/Claude")
                } else {
                    return None;
                };
                folder.join("claude_desktop_config.json")
            }
            App::Cursor => home.join(".cursor").join("mcp.json"),
            App::VsCode => code_user().join("mcp.json"),
            App::Windsurf => home.join(".codeium").join("windsurf").join("mcp_config.json"),
            App::Codex => home.join(".codex").join("config.toml"),
            App::GeminiCli => home.join(".gemini").join("settings.json"),
            App::Kiro => home.join(".kiro").join("settings").join("mcp.json"),
            App::LmStudio => home.join(".lmstudio").join("mcp.json"),
            App::OpenCode => home.join(".config").join("opencode").join("opencode.json"),
            App::Zed => {
                if cfg!(windows) {
                    roaming.join("Zed").join("settings.json")
                } else {
                    home.join(".config/zed/settings.json")
                }
            }
            App::ClaudeCode | App::Other => return None,
        })
    }

    /// The settings file on this computer.
    pub fn settings_file(self) -> Option<PathBuf> {
        let home = PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?);
        let roaming = std::env::var_os("APPDATA").map_or_else(|| home.clone(), PathBuf::from);
        self.settings_file_in(&home, &roaming)
    }

    /// The settings file, if the app looks installed here: the folder it
    /// keeps its settings in exists (the file itself may not, yet). Never
    /// for Zed, whose settings this does not edit.
    pub fn installed_settings_file(self) -> Option<PathBuf> {
        if self.settings() == Settings::Zed {
            return None;
        }
        let file = self.settings_file()?;
        file.parent()?.is_dir().then_some(file)
    }

    /// What to paste, or run, to add `server` to this app by hand.
    pub fn setup_text(self, server: &Server) -> String {
        let server = self.server(server);
        match self.settings() {
            Settings::Command => claude_code_command(&server),
            Settings::CodexToml => codex_table(&server),
            settings => {
                let (key, entry) = json_entry(settings, &server);
                let mut block = serde_json::Map::new();
                block.insert(key.to_owned(), serde_json::json!({ "bettercut": entry }));
                serde_json::to_string_pretty(&block).unwrap_or_default()
            }
        }
    }

    /// Where `setup_text` goes, said for a person.
    pub fn where_it_goes(self) -> String {
        let file = self
            .settings_file()
            .map(|f| f.display().to_string())
            .unwrap_or_default();
        match self {
            App::ClaudeCode => "Or run this once in a terminal:".to_owned(),
            App::ClaudeDesktop if file.is_empty() => {
                "Add this to claude_desktop_config.json:".to_owned()
            }
            App::Other => {
                "Most apps take this in their MCP server settings (often a file named mcp.json):"
                    .to_owned()
            }
            App::Zed => format!("Add this to Zed's settings ({file}):"),
            _ => format!("Or add this to {file}:"),
        }
    }

    /// Add bettercut to this app's settings file, keeping a copy of it first.
    pub fn add_bettercut(self, server: &Server) -> Result<String, String> {
        let file = self
            .installed_settings_file()
            .ok_or_else(|| format!("{} is not installed here", self.name()))?;
        let existing = std::fs::read_to_string(&file).ok();
        let merged = self.merge(existing.as_deref(), server)?;
        if existing.is_some() {
            let mut backup = file.clone().into_os_string();
            backup.push(".before-bettercut");
            std::fs::copy(&file, &backup).map_err(|e| format!("could not keep a copy: {e}"))?;
        }
        std::fs::write(&file, merged).map_err(|e| format!("could not save: {e}"))?;
        Ok(format!(
            "Added to {}. Quit and reopen {} to use it.",
            self.name(),
            self.name()
        ))
    }

    /// `existing` settings (or none yet) with bettercut added, everything
    /// else kept as it was. Refuses settings it cannot read for sure rather
    /// than guess at them.
    pub fn merge(self, existing: Option<&str>, server: &Server) -> Result<String, String> {
        let server = self.server(server);
        match self.settings() {
            Settings::CodexToml => Ok(merge_codex(existing.unwrap_or(""), &server)),
            Settings::Command | Settings::Zed => {
                Err(format!("{} is not set up through a settings file", self.name()))
            }
            settings => merge_json(self.name(), settings, existing, &server),
        }
    }
}

/// The Claude Code command that adds `server`.
pub fn claude_code_command(server: &Server) -> String {
    let mut command = format!(
        "claude mcp add --scope user bettercut -- \"{}\"",
        server.program.display()
    );
    for arg in &server.args {
        command.push(' ');
        command.push_str(arg);
    }
    command
}

/// The key a JSON settings file keeps servers under, and bettercut's entry.
fn json_entry(settings: Settings, server: &Server) -> (&'static str, serde_json::Value) {
    let program = server.program.display().to_string();
    match settings {
        Settings::VsCode => (
            "servers",
            serde_json::json!({ "type": "stdio", "command": program, "args": server.args }),
        ),
        Settings::OpenCode => {
            let mut command = vec![program];
            command.extend(server.args.iter().cloned());
            (
                "mcp",
                serde_json::json!({ "type": "local", "command": command, "enabled": true }),
            )
        }
        Settings::Zed => (
            "context_servers",
            serde_json::json!({ "command": program, "args": server.args }),
        ),
        Settings::Json(key) => {
            let mut entry = serde_json::json!({ "command": program });
            if !server.args.is_empty() {
                entry["args"] = serde_json::json!(server.args);
            }
            (key, entry)
        }
        Settings::Command | Settings::CodexToml => ("", serde_json::Value::Null),
    }
}

fn merge_json(
    app: &str,
    settings: Settings,
    existing: Option<&str>,
    server: &Server,
) -> Result<String, String> {
    let mut config = match existing.map(str::trim) {
        None | Some("") => serde_json::json!({}),
        Some(text) => serde_json::from_str::<serde_json::Value>(text).map_err(|err| {
            format!("{app}'s settings are not plain JSON ({err}); add bettercut by hand")
        })?,
    };
    let Some(top) = config.as_object_mut() else {
        return Err(format!(
            "{app}'s settings are not what was expected; add bettercut by hand"
        ));
    };
    let (key, entry) = json_entry(settings, server);
    let servers = top.entry(key).or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err(format!(
            "{app}'s {key} is not a list of servers; add bettercut by hand"
        ));
    };
    servers.insert("bettercut".to_owned(), entry);
    serde_json::to_string_pretty(&config).map_err(|e| e.to_string())
}

/// Codex's table for `server`. A JSON string is a valid TOML basic string,
/// escapes and all, so paths with backslashes come out right.
fn codex_table(server: &Server) -> String {
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let args: Vec<String> = server.args.iter().map(|a| quote(a)).collect();
    format!(
        "[mcp_servers.bettercut]\ncommand = {}\nargs = [{}]\n",
        quote(&server.program.display().to_string()),
        args.join(", ")
    )
}

/// `existing` Codex settings with bettercut's table in place of any earlier
/// one (and its sub-tables), everything else line for line as it was.
fn merge_codex(existing: &str, server: &Server) -> String {
    let ours = |header: &str| {
        let name = header.trim().trim_start_matches('[').trim_end_matches(']').trim();
        name == "mcp_servers.bettercut" || name.starts_with("mcp_servers.bettercut.")
    };
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in existing.lines() {
        if line.trim_start().starts_with('[') {
            skipping = ours(line);
        }
        if !skipping {
            kept.push(line);
        }
    }
    let mut text = kept.join("\n");
    let trimmed = text.trim_end().len();
    text.truncate(trimmed);
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(&codex_table(server));
    text
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

/// Whether an assistant has called lately enough to count as connected.
pub fn is_connected(last_heard: Option<Instant>, now: Instant) -> bool {
    last_heard.is_some_and(|at| now.saturating_duration_since(at) <= CONNECTED_FOR)
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
                "An assistant — Claude, Copilot, Cursor, Codex, Gemini or any other that speaks \
                 MCP — can edit with you: cut, title, colour, caption and export, in this \
                 window as you watch, every change undoable like your own.",
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
            theme::section(ui, "Assistant app");
            let before = state.assistant_app;
            egui::ComboBox::from_id_salt("assistant app")
                .width(220.0)
                .selected_text(state.assistant_app.name())
                .show_ui(ui, |ui| {
                    for app in App::ALL {
                        ui.selectable_value(&mut state.assistant_app, app, app.name());
                    }
                });
            if state.assistant_app != before
                && let Ok(mut n) = state.assistant_note.lock()
            {
                *n = None;
            }
            let app = state.assistant_app;
            ui.add_space(6.0);
            if app == App::ClaudeCode {
                if ui
                    .add(theme::primary_button("Add to Claude Code"))
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
            } else if app.installed_settings_file().is_some()
                && ui
                    .add(theme::primary_button(&format!("Add to {}", app.name())))
                    .on_hover_text(format!(
                        "Adds bettercut to {}'s settings, keeping a copy of them",
                        app.name()
                    ))
                    .clicked()
            {
                let said = app.add_bettercut(&server).unwrap_or_else(|err| err);
                if let Ok(mut n) = state.assistant_note.lock() {
                    *n = Some(said);
                }
            }
            ui.label(app.where_it_goes());
            copyable(ui, state, &app.setup_text(&server));
            if app.wants_compact() {
                ui.label(
                    egui::RichText::new(format!(
                        "{} takes only so many tools, so bettercut lists its common ones \
                         (--compact); the assistant finds the rest when it needs them.",
                        app.name()
                    ))
                    .color(theme::disabled()),
                );
            }
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

    fn windows_install() -> Server {
        Server {
            program: PathBuf::from("C:\\Program Files\\bettercut\\bettercut-mcp.exe"),
            args: Vec::new(),
        }
    }

    #[test]
    fn the_commands_name_the_server_beside_the_app() {
        let server = &windows_install();
        assert_eq!(
            App::ClaudeCode.setup_text(server),
            "claude mcp add --scope user bettercut -- \"C:\\Program Files\\bettercut\\bettercut-mcp.exe\""
        );
        let config: serde_json::Value =
            serde_json::from_str(&App::ClaudeDesktop.setup_text(server)).unwrap_or_default();
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
            "claude mcp add --scope user bettercut -- \"/home/me/bettercut-0.4.0-x86_64.AppImage\" --mcp"
        );
        let config: serde_json::Value =
            serde_json::from_str(&App::GeminiCli.setup_text(&server)).unwrap_or_default();
        assert_eq!(
            config["mcpServers"]["bettercut"]["args"],
            serde_json::json!(["--mcp"])
        );
        // Cursor gets the compact server, after the AppImage's own flag.
        let config: serde_json::Value =
            serde_json::from_str(&App::Cursor.setup_text(&server)).unwrap_or_default();
        assert_eq!(
            config["mcpServers"]["bettercut"]["args"],
            serde_json::json!(["--mcp", "--compact"])
        );
    }

    #[test]
    fn each_app_gets_its_own_shape() {
        let server = &windows_install();
        let vscode: serde_json::Value =
            serde_json::from_str(&App::VsCode.setup_text(server)).unwrap_or_default();
        assert_eq!(vscode["servers"]["bettercut"]["type"], "stdio");
        let opencode: serde_json::Value =
            serde_json::from_str(&App::OpenCode.setup_text(server)).unwrap_or_default();
        assert_eq!(opencode["mcp"]["bettercut"]["type"], "local");
        assert_eq!(
            opencode["mcp"]["bettercut"]["command"][0],
            "C:\\Program Files\\bettercut\\bettercut-mcp.exe"
        );
        let zed: serde_json::Value =
            serde_json::from_str(&App::Zed.setup_text(server)).unwrap_or_default();
        assert!(zed["context_servers"]["bettercut"]["command"].is_string());
        assert_eq!(
            App::Codex.setup_text(server),
            "[mcp_servers.bettercut]\ncommand = \"C:\\\\Program Files\\\\bettercut\\\\bettercut-mcp.exe\"\nargs = []\n"
        );
        // Every app shows something to paste or run, and a place for it.
        for app in App::ALL {
            assert!(!app.setup_text(server).is_empty(), "{}", app.name());
            assert!(!app.where_it_goes().is_empty(), "{}", app.name());
        }
    }

    #[test]
    fn settings_files_are_where_each_app_looks() {
        let home = PathBuf::from("/home/me");
        let roaming = PathBuf::from("/roaming");
        let at = |app: App| {
            app.settings_file_in(&home, &roaming)
                .map(|f| f.display().to_string().replace('\\', "/"))
                .unwrap_or_default()
        };
        assert!(at(App::Cursor).ends_with("/home/me/.cursor/mcp.json"));
        assert!(at(App::Windsurf).ends_with(".codeium/windsurf/mcp_config.json"));
        assert!(at(App::Codex).ends_with(".codex/config.toml"));
        assert!(at(App::GeminiCli).ends_with(".gemini/settings.json"));
        assert!(at(App::VsCode).ends_with("Code/User/mcp.json"));
        assert!(at(App::ClaudeCode).is_empty() && at(App::Other).is_empty());
    }

    #[test]
    fn json_settings_keep_what_was_there() {
        let server = Server {
            program: PathBuf::from("/Applications/bettercut.app/Contents/MacOS/bettercut-mcp"),
            args: Vec::new(),
        };
        // Nothing yet: made from scratch.
        let made: serde_json::Value = serde_json::from_str(
            &App::ClaudeDesktop
                .merge(None, &server)
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        assert!(made["mcpServers"]["bettercut"]["command"].is_string());

        // Other servers and settings stay; an old bettercut entry is replaced.
        let before = r#"{ "theme": "dark", "mcpServers": {
            "files": { "command": "files-server" },
            "bettercut": { "command": "/old/place" } } }"#;
        let merged: serde_json::Value = serde_json::from_str(
            &App::Cursor
                .merge(Some(before), &server)
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["mcpServers"]["files"]["command"], "files-server");
        assert_eq!(
            merged["mcpServers"]["bettercut"]["command"],
            "/Applications/bettercut.app/Contents/MacOS/bettercut-mcp"
        );
        assert_eq!(merged["mcpServers"]["bettercut"]["args"], serde_json::json!(["--compact"]));

        // VS Code's own key, beside its inputs.
        let merged: serde_json::Value = serde_json::from_str(
            &App::VsCode
                .merge(Some(r#"{ "inputs": [], "servers": {} }"#), &server)
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        assert_eq!(merged["inputs"], serde_json::json!([]));
        assert_eq!(merged["servers"]["bettercut"]["type"], "stdio");

        // Not JSON (comments too), or not shaped like settings: refused,
        // never overwritten.
        let app = App::GeminiCli;
        assert!(app.merge(Some("{ not json"), &server).is_err());
        assert!(app.merge(Some("// a comment\n{}"), &server).is_err());
        assert!(app.merge(Some("[1, 2]"), &server).is_err());
        assert!(app.merge(Some(r#"{"mcpServers": 3}"#), &server).is_err());
        // Zed's settings, with comments, are never edited.
        assert!(App::Zed.merge(Some("{}"), &server).is_err());
        assert!(App::Zed.installed_settings_file().is_none());
    }

    #[test]
    fn codex_settings_keep_every_other_line() {
        let server = windows_install();
        let before = "model = \"o4\"\n\n[mcp_servers.files]\ncommand = \"files\"\n\n\
                      [mcp_servers.bettercut]\ncommand = \"/old\"\n\n\
                      [mcp_servers.bettercut.env]\nX = \"1\"\n\n[profiles.fast]\nmodel = \"mini\"\n";
        let merged = App::Codex.merge(Some(before), &server).unwrap_or_default();
        assert!(merged.starts_with("model = \"o4\"\n\n[mcp_servers.files]\ncommand = \"files\""));
        assert!(merged.contains("[profiles.fast]\nmodel = \"mini\""), "{merged}");
        assert!(!merged.contains("/old") && !merged.contains("X = \"1\""), "{merged}");
        assert_eq!(merged.matches("[mcp_servers.bettercut]").count(), 1, "{merged}");
        assert!(merged.ends_with("args = []\n"), "{merged}");
        // Nothing there yet: just the table.
        assert_eq!(
            App::Codex.merge(None, &server).unwrap_or_default(),
            App::Codex.setup_text(&server)
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
