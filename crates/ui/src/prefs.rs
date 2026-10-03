//! What the interface remembers about itself between runs: today, which
//! theme. A project file says nothing about this, rightly — a light theme is
//! about the room the editor is in, not the edit.
//!
//! One line a setting, `name=value`, beside the recent-projects list. A line
//! nothing here understands is kept out rather than kept: a newer build's
//! setting is not this build's business, and an old file must never stop the
//! editor starting.

use std::path::{Path, PathBuf};

/// How small and how large the interface may be drawn.
pub const MIN_SCALE: f32 = 0.7;
pub const MAX_SCALE: f32 = 2.0;

#[derive(Debug, Clone, PartialEq)]
pub struct UserPrefs {
    pub light_theme: bool,
    /// The last colours picked, newest first (`crate::swatches`).
    pub recent_colours: Vec<[u8; 3]>,
    /// How large the interface is drawn, 1 being the system's own size:
    /// egui's zoom factor. Held to [`MIN_SCALE`]..=[`MAX_SCALE`].
    pub interface_scale: f32,
    /// Seconds between forced recovery snapshots (`Editor::set_autosave_seconds`).
    pub autosave_seconds: u64,
    /// The command palette's recent actions by name, newest first
    /// (`crate::palette`).
    pub palette_recent: Vec<String>,
    /// The welcome window was dismissed for good.
    pub seen_welcome: bool,
    /// The build that last ran here, for "what's new" (`crate::whats_new`).
    pub last_version: String,
    /// Ask GitHub, once a day, whether a newer version is out
    /// (`crate::updates`). On unless turned off.
    pub check_updates: bool,
    /// When that was last asked, in seconds since 1970.
    pub last_update_check: u64,
    file: Option<PathBuf>,
}

impl UserPrefs {
    /// Read the settings kept in `file`, or the defaults when there is no
    /// file yet.
    pub fn stored_in(file: PathBuf) -> Self {
        let mut prefs = std::fs::read_to_string(&file)
            .map(|text| parse(&text))
            .unwrap_or_default();
        prefs.file = Some(file);
        prefs
    }

    /// Where the running editor keeps them: beside the recent-projects list.
    pub fn default_file() -> PathBuf {
        bettercut_editor_core::templates::library::user_dir()
            .parent()
            .map_or_else(std::env::temp_dir, Path::to_path_buf)
            .join("interface.txt")
    }

    /// Write them out. Nothing to do for prefs made without a file.
    pub fn save(&self) -> Result<(), String> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(file, render(self)).map_err(|e| e.to_string())
    }
}

fn render(prefs: &UserPrefs) -> String {
    let colours: Vec<String> = prefs
        .recent_colours
        .iter()
        .map(|[r, g, b]| format!("{r:02x}{g:02x}{b:02x}"))
        .collect();
    format!(
        "# bettercut interface\nlight_theme={}\nrecent_colours={}\ninterface_scale={:.2}\nautosave_seconds={}\npalette_recent={}\nseen_welcome={}\nlast_version={}\ncheck_updates={}\nlast_update_check={}\n",
        prefs.light_theme,
        colours.join(","),
        prefs.interface_scale,
        prefs.autosave_seconds,
        // Names never hold a comma or a line break: they are the palette's own.
        prefs.palette_recent.join(","),
        prefs.seen_welcome,
        prefs.last_version,
        prefs.check_updates,
        prefs.last_update_check
    )
}

/// A scale held to what can be worked in: a garbled or absurd number is
/// the system's size.
pub fn sane_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    }
}

impl Default for UserPrefs {
    fn default() -> Self {
        Self {
            light_theme: false,
            recent_colours: Vec::new(),
            interface_scale: 1.0,
            autosave_seconds: 60,
            palette_recent: Vec::new(),
            seen_welcome: false,
            last_version: String::new(),
            check_updates: true,
            last_update_check: 0,
            file: None,
        }
    }
}

/// `rrggbb` to a colour, or nothing for anything else.
fn hex_colour(text: &str) -> Option<[u8; 3]> {
    let text = text.trim();
    if text.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn parse(text: &str) -> UserPrefs {
    let mut prefs = UserPrefs::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "light_theme" => prefs.light_theme = value.trim() == "true",
            "autosave_seconds" => {
                prefs.autosave_seconds = value.trim().parse().map_or(60, |s: u64| s.clamp(10, 600));
            }
            "interface_scale" => {
                prefs.interface_scale = value.trim().parse().map_or(1.0, sane_scale);
            }
            "seen_welcome" => prefs.seen_welcome = value.trim() == "true",
            "last_version" => prefs.last_version = value.trim().to_owned(),
            "check_updates" => prefs.check_updates = value.trim() != "false",
            "last_update_check" => prefs.last_update_check = value.trim().parse().unwrap_or(0),
            "palette_recent" => {
                prefs.palette_recent = value
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .take(crate::palette::RECENT)
                    .map(str::to_owned)
                    .collect();
            }
            "recent_colours" => {
                prefs.recent_colours = value
                    .split(',')
                    .filter_map(hex_colour)
                    .take(crate::swatches::MAX_SWATCHES)
                    .collect();
            }
            _ => {}
        }
    }
    prefs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_choice_round_trips() {
        let prefs = UserPrefs {
            light_theme: true,
            recent_colours: vec![[255, 136, 0], [0, 0, 16]],
            interface_scale: 1.25,
            autosave_seconds: 120,
            palette_recent: vec!["Undo".to_owned(), "Select All".to_owned()],
            seen_welcome: true,
            last_version: "0.0.9".to_owned(),
            check_updates: false,
            last_update_check: 1_700_000_000,
            file: None,
        };
        assert!(!parse(&render(&prefs)).check_updates);
        assert_eq!(parse(&render(&prefs)).last_update_check, 1_700_000_000);
        assert!(parse("").check_updates, "on unless turned off");
        assert_eq!(parse(&render(&prefs)).last_version, "0.0.9");
        assert!(parse(&render(&prefs)).seen_welcome);
        assert!(!parse("").seen_welcome, "a new install shows the welcome");
        assert_eq!(parse(&render(&prefs)).palette_recent, prefs.palette_recent);
        assert_eq!(parse(&render(&prefs)).autosave_seconds, 120);
        assert_eq!(parse("autosave_seconds=1\n").autosave_seconds, 10);
        assert!(parse(&render(&prefs)).light_theme);
        assert_eq!(parse(&render(&prefs)).interface_scale, 1.25);
        assert_eq!(parse("interface_scale=9\n").interface_scale, MAX_SCALE);
        assert_eq!(parse("interface_scale=nope\n").interface_scale, 1.0);
        assert_eq!(UserPrefs::default().interface_scale, 1.0);
        assert_eq!(parse(&render(&prefs)).recent_colours, prefs.recent_colours);
        assert!(
            parse("recent_colours=zz0000,ff00\n")
                .recent_colours
                .is_empty()
        );
        assert!(!parse(&render(&UserPrefs::default())).light_theme);
    }

    /// A line from another build, or a scribble, is passed over rather than
    /// stopping the file being read.
    #[test]
    fn unknown_lines_are_left_alone() {
        let prefs = parse("# hello\nsomething_new=7\nnot a setting\nlight_theme=true\n");
        assert!(prefs.light_theme);
    }

    #[test]
    fn no_file_is_the_defaults() {
        let prefs = UserPrefs::stored_in(std::env::temp_dir().join("bettercut-no-such-prefs.txt"));
        assert!(!prefs.light_theme);
    }
}
