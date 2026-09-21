//! Title styles of your own: the way a caption looks, kept by name and put on
//! any other title.
//!
//! A title is a dozen decisions — font, size, weight, colour, outline, shadow,
//! background, line height, letter spacing — and a video has twenty titles
//! that should all look the same. Making the twentieth match the first by hand
//! is the work this saves, and *matching* is the point: a caption that is a
//! pixel bigger than the one before it reads as a mistake.
//!
//! Kept beside the recent projects and the saved looks rather than in the
//! project, and for the same reason (`crate::looks`): a style is how *you*
//! title a video, so the next project opens with it already there.
//!
//! # Why a line of JSON
//!
//! The looks file is tab-separated numbers, which a person can read and edit.
//! A style is not numbers: it has an optional outline with its own colour and
//! width, an optional shadow with an offset, an optional background. Flattened
//! into columns it would be unreadable *and* fragile. One JSON object a line
//! keeps the property that mattered — a line can be deleted, copied to another
//! machine, or hand-edited — without pretending a nested thing is flat.

use std::path::PathBuf;

use bettercut_editor_core::text::TextStyle;

/// How many styles may be kept, for the same reason looks have a limit: a row
/// of names is only useful while it can be read at a glance.
pub const MAX_STYLES: usize = 24;

/// The longest a style's name may be.
pub const MAX_NAME: usize = 24;

/// The title styles this user has saved, in the order they were saved.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserTitleStyles {
    styles: Vec<(String, TextStyle)>,
    /// Where they are kept; `None` keeps them in memory only, which is what
    /// the tests use.
    file: Option<PathBuf>,
}

impl UserTitleStyles {
    /// The styles kept in `file`, reading whatever is there now.
    ///
    /// A file that is missing or garbled is simply no saved styles: this is a
    /// convenience, and refusing to start the editor over it would not be.
    pub fn stored_in(file: PathBuf) -> Self {
        let styles = std::fs::read_to_string(&file)
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self {
            styles,
            file: Some(file),
        }
    }

    /// Where the running editor keeps them: beside the recent projects.
    pub fn default_file() -> PathBuf {
        crate::recent::RecentProjects::default_file().with_file_name("title-styles.jsonl")
    }

    pub fn all(&self) -> &[(String, TextStyle)] {
        &self.styles
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }

    /// Save `style` under `name`, replacing a style of the same name.
    ///
    /// Returns why not, if not: an empty name, or the list already full.
    pub fn save(&mut self, name: &str, style: TextStyle) -> Result<(), String> {
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        if name.is_empty() {
            return Err("Give the style a name".to_owned());
        }
        match self.styles.iter_mut().find(|(saved, _)| *saved == name) {
            Some(entry) => entry.1 = style,
            None => {
                if self.styles.len() >= MAX_STYLES {
                    return Err(format!(
                        "Only {MAX_STYLES} title styles can be kept — remove one first"
                    ));
                }
                self.styles.push((name, style));
            }
        }
        self.write();
        Ok(())
    }

    /// Forget the style called `name`. Returns whether there was one.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.styles.len();
        self.styles.retain(|(saved, _)| saved != name);
        let removed = self.styles.len() != before;
        if removed {
            self.write();
        }
        removed
    }

    /// The style called `name`, if it is saved.
    pub fn get(&self, name: &str) -> Option<TextStyle> {
        self.styles
            .iter()
            .find(|(saved, _)| saved == name)
            .map(|(_, style)| style.clone())
    }

    fn write(&self) {
        let Some(file) = &self.file else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(file, render(&self.styles));
    }
}

/// One style a line: its name, then the style itself.
fn render(styles: &[(String, TextStyle)]) -> String {
    let mut text =
        String::from("# bettercut title styles: one {\"name\": …, \"style\": …} a line\n");
    for (name, style) in styles {
        let line = serde_json::json!({ "name": name, "style": style });
        if let Ok(line) = serde_json::to_string(&line) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    text
}

fn parse(text: &str) -> Vec<(String, TextStyle)> {
    let mut styles = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // A line that will not read is one style lost, not a file lost: the
        // rest are still perfectly good.
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let name = value
            .get("name")
            .and_then(|name| name.as_str())
            .map(|name| name.trim().chars().take(MAX_NAME).collect::<String>())
            .filter(|name| !name.is_empty());
        let style = value
            .get("style")
            .and_then(|style| serde_json::from_value::<TextStyle>(style.clone()).ok());
        let (Some(name), Some(style)) = (name, style) else {
            continue;
        };
        styles.push((name, style));
        if styles.len() >= MAX_STYLES {
            break;
        }
    }
    styles
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_editor_core::text::{FontWeight, Rgba};

    fn caption() -> TextStyle {
        TextStyle {
            size: 72.0,
            weight: FontWeight::Bold,
            color: Rgba::WHITE,
            stroke: Some(bettercut_editor_core::text::Stroke {
                color: Rgba::BLACK,
                width: 4.0,
            }),
            ..TextStyle::default()
        }
    }

    #[test]
    fn a_saved_style_comes_back_by_name() {
        let mut styles = UserTitleStyles::default();
        styles.save("Caption", caption()).expect("saved");
        assert_eq!(styles.get("Caption"), Some(caption()));
        assert_eq!(styles.all().len(), 1);
        assert!(styles.remove("Caption"));
        assert!(styles.is_empty());
    }

    #[test]
    fn saving_the_same_name_twice_replaces_it() {
        let mut styles = UserTitleStyles::default();
        styles.save("Mine", caption()).expect("saved");
        styles.save("Mine", TextStyle::default()).expect("saved");
        assert_eq!(styles.all().len(), 1);
        assert_eq!(styles.get("Mine"), Some(TextStyle::default()));
    }

    #[test]
    fn a_style_needs_a_name() {
        let mut styles = UserTitleStyles::default();
        assert!(styles.save("  ", caption()).is_err());
    }

    #[test]
    fn the_list_has_a_limit() {
        let mut styles = UserTitleStyles::default();
        for n in 0..MAX_STYLES {
            styles
                .save(&format!("Style {n}"), caption())
                .expect("saved");
        }
        assert!(styles.save("One more", caption()).is_err());
        // Replacing one that is already there still works.
        assert!(styles.save("Style 0", TextStyle::default()).is_ok());
    }

    /// The outline, the shadow and the rest survive the file — which is the
    /// whole reason this one is JSON.
    #[test]
    fn styles_round_trip_through_the_file_they_are_written_as() {
        let mut styles = UserTitleStyles::default();
        styles.save("Caption", caption()).expect("saved");
        styles.save("Plain", TextStyle::default()).expect("saved");

        let back = parse(&render(styles.all()));
        assert_eq!(back, styles.all().to_vec());
        assert!(back[0].1.stroke.is_some(), "the outline was lost");
    }

    #[test]
    fn a_garbled_line_is_skipped_rather_than_fatal() {
        let mut styles = UserTitleStyles::default();
        styles.save("Good", caption()).expect("saved");
        let text = format!("# a comment\nnot json\n{{\"name\":\"no style\"}}\n{}", {
            let rendered = render(styles.all());
            rendered.lines().nth(1).unwrap_or_default().to_owned()
        });

        let back = parse(&text);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].0, "Good");
    }
}
