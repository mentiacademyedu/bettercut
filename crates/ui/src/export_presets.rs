//! Export settings of your own, kept by name.
//!
//! The built-in presets are what a *site* wants (`export_dialog::PLATFORMS`).
//! This is what *you* want: the codec, the size, the rate and the extra shapes
//! you send every week, which are not any site's recommendation and are not
//! worth setting up a second time.
//!
//! Beside the recent projects, the saved looks and the title styles, and for
//! the same reason (`crate::looks`): how you deliver is yours, not this
//! project's.
//!
//! # What is kept, and what is not
//!
//! The *format*: container, codec, size, rate, bitrate, rate control and the
//! extra shapes. Not the file name, not the folder, and not the range — those
//! are about one export of one edit, and a preset that quietly renamed the
//! file or wrote it somewhere else would be a trap rather than a shortcut.

use std::path::PathBuf;

/// How many may be kept, for the same reason looks have a limit.
pub const MAX_PRESETS: usize = 16;

/// The longest a preset's name may be.
pub const MAX_NAME: usize = 24;

/// One saved set of export settings. Plain numbers, so the file stays
/// readable and a preset written by an older build still loads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SavedExport {
    /// Index into the export dialog's container list.
    pub container: usize,
    /// The codec, by its own name.
    pub codec: String,
    /// Index into the dialog's height list, or `None` for a custom size.
    pub height: Option<usize>,
    /// The custom size, when there is one.
    pub custom: Option<(u32, u32)>,
    /// The frame rate as numerator and denominator, or `None` for the
    /// sequence's own.
    pub frame_rate: Option<(i64, i64)>,
    /// The bitrate choice, by name, and the typed rate when it is custom.
    pub bitrate: String,
    pub custom_kbps: Option<u32>,
    pub rate_control: String,
    /// Extra shapes to write alongside, as width:height.
    pub also: Vec<(u32, u32)>,
    /// The main file's shape, as width:height, or `None` for the sequence's.
    pub shape: Option<(u32, u32)>,
}

/// The export presets this user has saved, in the order they were saved.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserExports {
    presets: Vec<(String, SavedExport)>,
    file: Option<PathBuf>,
}

impl UserExports {
    pub fn stored_in(file: PathBuf) -> Self {
        let presets = std::fs::read_to_string(&file)
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self {
            presets,
            file: Some(file),
        }
    }

    /// Where the running editor keeps them: beside the recent projects.
    pub fn default_file() -> PathBuf {
        crate::recent::RecentProjects::default_file().with_file_name("export-presets.jsonl")
    }

    pub fn all(&self) -> &[(String, SavedExport)] {
        &self.presets
    }

    pub fn is_empty(&self) -> bool {
        self.presets.is_empty()
    }

    /// Save `preset` under `name`, replacing one of the same name.
    pub fn save(&mut self, name: &str, preset: SavedExport) -> Result<(), String> {
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        if name.is_empty() {
            return Err("Give the preset a name".to_owned());
        }
        match self.presets.iter_mut().find(|(saved, _)| *saved == name) {
            Some(entry) => entry.1 = preset,
            None => {
                if self.presets.len() >= MAX_PRESETS {
                    return Err(format!(
                        "Only {MAX_PRESETS} export presets can be kept — remove one first"
                    ));
                }
                self.presets.push((name, preset));
            }
        }
        self.write();
        Ok(())
    }

    /// Forget the preset called `name`. Returns whether there was one.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.presets.len();
        self.presets.retain(|(saved, _)| saved != name);
        let removed = self.presets.len() != before;
        if removed {
            self.write();
        }
        removed
    }

    pub fn get(&self, name: &str) -> Option<SavedExport> {
        self.presets
            .iter()
            .find(|(saved, _)| saved == name)
            .map(|(_, preset)| preset.clone())
    }

    fn write(&self) {
        let Some(file) = &self.file else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(file, render(&self.presets));
    }
}

fn render(presets: &[(String, SavedExport)]) -> String {
    let mut text = String::from("# bettercut export presets: one JSON object a line\n");
    for (name, preset) in presets {
        let value = serde_json::json!({
            "name": name,
            "container": preset.container,
            "codec": preset.codec,
            "height": preset.height,
            "custom": preset.custom.map(|(w, h)| [w, h]),
            "frame_rate": preset.frame_rate.map(|(n, d)| [n, d]),
            "bitrate": preset.bitrate,
            "custom_kbps": preset.custom_kbps,
            "rate_control": preset.rate_control,
            "also": preset.also.iter().map(|(w, h)| [*w, *h]).collect::<Vec<_>>(),
            "shape": preset.shape.map(|(w, h)| [w, h]),
        });
        if let Ok(line) = serde_json::to_string(&value) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    text
}

fn parse(text: &str) -> Vec<(String, SavedExport)> {
    let mut presets = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(name) = value
            .get("name")
            .and_then(|name| name.as_str())
            .map(|name| name.trim().chars().take(MAX_NAME).collect::<String>())
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        // A pair of numbers, where one is written.
        let pair = |key: &str| -> Option<(u32, u32)> {
            let array = value.get(key)?.as_array()?;
            Some((
                array.first()?.as_u64()? as u32,
                array.get(1)?.as_u64()? as u32,
            ))
        };
        let ratio = |key: &str| -> Option<(i64, i64)> {
            let array = value.get(key)?.as_array()?;
            Some((array.first()?.as_i64()?, array.get(1)?.as_i64()?))
        };
        let text_of = |key: &str| {
            value
                .get(key)
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        presets.push((
            name,
            SavedExport {
                container: value
                    .get("container")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize,
                codec: text_of("codec"),
                height: value
                    .get("height")
                    .and_then(serde_json::Value::as_u64)
                    .map(|index| index as usize),
                custom: pair("custom"),
                frame_rate: ratio("frame_rate"),
                bitrate: text_of("bitrate"),
                custom_kbps: value
                    .get("custom_kbps")
                    .and_then(serde_json::Value::as_u64)
                    .map(|kbps| kbps as u32),
                rate_control: text_of("rate_control"),
                also: value
                    .get("also")
                    .and_then(serde_json::Value::as_array)
                    .map(|shapes| {
                        shapes
                            .iter()
                            .filter_map(|shape| {
                                let shape = shape.as_array()?;
                                Some((
                                    shape.first()?.as_u64()? as u32,
                                    shape.get(1)?.as_u64()? as u32,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                shape: pair("shape"),
            },
        ));
        if presets.len() >= MAX_PRESETS {
            break;
        }
    }
    presets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertical() -> SavedExport {
        SavedExport {
            container: 1,
            codec: "H264".to_owned(),
            height: Some(2),
            custom: None,
            frame_rate: Some((30, 1)),
            bitrate: "High".to_owned(),
            custom_kbps: Some(12_000),
            rate_control: "Variable".to_owned(),
            also: vec![(16, 9), (1, 1)],
            shape: Some((9, 16)),
        }
    }

    #[test]
    fn a_saved_preset_comes_back_by_name() {
        let mut presets = UserExports::default();
        presets.save("Client vertical", vertical()).expect("saved");
        assert_eq!(presets.get("Client vertical"), Some(vertical()));
        assert!(presets.remove("Client vertical"));
        assert!(presets.is_empty());
    }

    #[test]
    fn saving_the_same_name_twice_replaces_it() {
        let mut presets = UserExports::default();
        presets.save("Mine", vertical()).expect("saved");
        presets.save("Mine", SavedExport::default()).expect("saved");
        assert_eq!(presets.all().len(), 1);
        assert_eq!(presets.get("Mine"), Some(SavedExport::default()));
    }

    #[test]
    fn a_preset_needs_a_name_and_the_list_has_a_limit() {
        let mut presets = UserExports::default();
        assert!(presets.save("   ", vertical()).is_err());
        for n in 0..MAX_PRESETS {
            presets
                .save(&format!("Preset {n}"), vertical())
                .expect("saved");
        }
        assert!(presets.save("One more", vertical()).is_err());
        assert!(presets.save("Preset 0", SavedExport::default()).is_ok());
    }

    /// The extra shapes and the custom rate survive the file — the parts a
    /// flat list of numbers would have lost.
    #[test]
    fn presets_round_trip_through_the_file() {
        let mut presets = UserExports::default();
        presets.save("Client vertical", vertical()).expect("saved");
        presets
            .save("Plain", SavedExport::default())
            .expect("saved");

        let back = parse(&render(presets.all()));
        assert_eq!(back, presets.all().to_vec());
        assert_eq!(back[0].1.also, vec![(16, 9), (1, 1)]);
        assert_eq!(back[0].1.custom_kbps, Some(12_000));
    }

    #[test]
    fn a_garbled_line_is_skipped_rather_than_fatal() {
        let back = parse(
            "# a comment\nnot json\n{\"container\":2}\n{\"name\":\"Good\",\"codec\":\"H265\"}\n",
        );
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].0, "Good");
        assert_eq!(back[0].1.codec, "H265");
    }
}
