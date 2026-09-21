//! Looks of your own: a grade you liked, kept by name and applied to anything
//! else.
//!
//! The eight built-in looks (`panels::LOOKS`) are a starting point, not an
//! answer — the grade that makes *your* camera look right is something you
//! arrive at once and then want on every clip you shoot with it.
//!
//! Kept beside the recent projects and the custom keys rather than in the
//! project, because that is what makes it yours rather than this edit's: a new
//! project opens with the same looks in it.

use std::path::PathBuf;

use bettercut_editor_core::timeline::ColorAdjust;

/// How many looks may be kept. A grid of names is only useful while it can be
/// read at a glance, and nobody is grading from a list of two hundred.
pub const MAX_LOOKS: usize = 24;

/// The longest a look's name may be, so the row of buttons stays readable.
pub const MAX_NAME: usize = 24;

/// A look worth keeping: the grade, and the picture treatment that goes with
/// it.
///
/// Five effects, not every effect. A look is how footage is *finished* —
/// softness, sparkle, edge, the falloff at the corners, the wear of old film —
/// and those travel from one shot to another. A glitch, a pixelation or a zoom
/// blur is a moment in a cut rather than a finish, and one arriving unasked on
/// a clip because a look was applied would be a surprise, not a shortcut.
///
/// Framing, opacity, masks and lookup tables are left out for a different
/// reason: the first two are about *this* clip's place in the frame, and a LUT
/// is a file this project happens to have imported — none of them mean
/// anything in the next project, where these looks still will.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedLook {
    pub colour: ColorAdjust,
    pub blur: f32,
    pub sharpen: f32,
    pub glow: f32,
    pub old_film: f32,
    pub vignette: f32,
}

impl Default for SavedLook {
    fn default() -> Self {
        Self {
            colour: ColorAdjust::IDENTITY,
            blur: 0.0,
            sharpen: 0.0,
            glow: 0.0,
            old_film: 0.0,
            vignette: 0.0,
        }
    }
}

impl SavedLook {
    /// Just a grade, with nothing done to the picture.
    pub fn graded(colour: ColorAdjust) -> Self {
        Self {
            colour,
            ..Self::default()
        }
    }

    /// Whether anything but the grade is in it, for a menu that says so.
    pub fn has_effects(self) -> bool {
        [
            self.blur,
            self.sharpen,
            self.glow,
            self.old_film,
            self.vignette,
        ]
        .iter()
        .any(|value| value.abs() > 0.001)
    }
}

/// The looks this user has saved, in the order they were saved.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserLooks {
    looks: Vec<(String, SavedLook)>,
    /// Where they are kept; `None` keeps them in memory only, which is what
    /// the tests use.
    file: Option<PathBuf>,
}

impl UserLooks {
    /// The looks kept in `file`, reading whatever is there now.
    ///
    /// A file that is missing or garbled is simply no saved looks: this is a
    /// convenience, and refusing to start the editor over it would not be.
    pub fn stored_in(file: PathBuf) -> Self {
        let looks = std::fs::read_to_string(&file)
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self {
            looks,
            file: Some(file),
        }
    }

    /// Where the running editor keeps them: beside the recent projects.
    pub fn default_file() -> PathBuf {
        crate::recent::RecentProjects::default_file().with_file_name("looks.txt")
    }

    pub fn all(&self) -> &[(String, SavedLook)] {
        &self.looks
    }

    pub fn is_empty(&self) -> bool {
        self.looks.is_empty()
    }

    /// Save `look` under `name`, replacing a look of the same name.
    ///
    /// Returns why not, if not: an empty name, or the list already full. The
    /// name is what the user will look for later, so it is trimmed but never
    /// invented.
    pub fn save(&mut self, name: &str, look: SavedLook) -> Result<(), String> {
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        if name.is_empty() {
            return Err("Give the look a name".to_owned());
        }
        match self.looks.iter_mut().find(|(saved, _)| *saved == name) {
            Some(entry) => entry.1 = look,
            None => {
                if self.looks.len() >= MAX_LOOKS {
                    return Err(format!(
                        "Only {MAX_LOOKS} looks can be kept — remove one first"
                    ));
                }
                self.looks.push((name, look));
            }
        }
        self.write();
        Ok(())
    }

    /// Forget the look called `name`. Returns whether there was one.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.looks.len();
        self.looks.retain(|(saved, _)| saved != name);
        let removed = self.looks.len() != before;
        if removed {
            self.write();
        }
        removed
    }

    /// The look called `name`, if it is saved.
    pub fn get(&self, name: &str) -> Option<SavedLook> {
        self.looks
            .iter()
            .find(|(saved, _)| saved == name)
            .map(|(_, look)| *look)
    }

    fn write(&self) {
        let Some(file) = &self.file else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(file, render(&self.looks));
    }
}

/// One look per line: the name, then its numbers, tab-separated.
///
/// A line rather than JSON for the same reason the keys file is: it is a
/// handful of numbers a person might want to read, copy to another machine, or
/// delete one line of.
fn render(looks: &[(String, SavedLook)]) -> String {
    let mut text = String::from(
        "# bettercut looks: name, brightness, contrast, saturation, temperature, tint, vibrance, \
         blur, sharpen, glow, old film, vignette\n",
    );
    for (name, look) in looks {
        let colour = look.colour;
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            name.replace('\t', " "),
            colour.brightness,
            colour.contrast,
            colour.saturation,
            colour.temperature,
            colour.tint,
            colour.vibrance,
            look.blur,
            look.sharpen,
            look.glow,
            look.old_film,
            look.vignette
        ));
    }
    text
}

fn parse(text: &str) -> Vec<(String, SavedLook)> {
    let mut looks = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let Some(name) = fields.next().map(str::trim).filter(|n| !n.is_empty()) else {
            continue;
        };
        let numbers: Vec<f32> = fields
            .filter_map(|field| field.trim().parse().ok())
            .collect();
        // Older files stop at the tint; a look written before vibrance existed
        // simply has none, which is what zero means.
        if numbers.len() < 5 || numbers.iter().any(|value| !value.is_finite()) {
            continue;
        }
        // A file written before a column existed simply has none, and what
        // it does not say is nothing — which is what zero means for every one
        // of them.
        let at = |index: usize| numbers.get(index).copied().unwrap_or(0.0);
        looks.push((
            name.chars().take(MAX_NAME).collect(),
            SavedLook {
                colour: ColorAdjust {
                    brightness: numbers[0],
                    contrast: numbers[1],
                    saturation: numbers[2],
                    temperature: numbers[3],
                    tint: numbers[4],
                    vibrance: at(5),
                },
                blur: at(6),
                sharpen: at(7),
                glow: at(8),
                old_film: at(9),
                vignette: at(10),
            },
        ));
        if looks.len() >= MAX_LOOKS {
            break;
        }
    }
    looks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warm() -> ColorAdjust {
        ColorAdjust {
            brightness: 1.05,
            contrast: 1.1,
            saturation: 1.2,
            temperature: 0.3,
            tint: -0.05,
            vibrance: 0.25,
        }
    }

    /// A grade *and* a finish: soft, glowing, worn at the corners.
    fn dreamy() -> SavedLook {
        SavedLook {
            colour: warm(),
            blur: 0.2,
            sharpen: 0.0,
            glow: 0.6,
            old_film: 0.1,
            vignette: 0.35,
        }
    }

    #[test]
    fn a_saved_look_comes_back_by_name() {
        let mut looks = UserLooks::default();
        looks.save("My camera", dreamy()).expect("saved");
        assert_eq!(looks.get("My camera"), Some(dreamy()));
        assert_eq!(looks.all().len(), 1);
        assert!(looks.remove("My camera"));
        assert!(looks.is_empty());
    }

    #[test]
    fn saving_the_same_name_twice_replaces_it() {
        let mut looks = UserLooks::default();
        looks.save("Mine", dreamy()).expect("saved");
        looks
            .save("Mine", SavedLook::graded(ColorAdjust::IDENTITY))
            .expect("saved");
        assert_eq!(looks.all().len(), 1);
        assert_eq!(
            looks.get("Mine"),
            Some(SavedLook::graded(ColorAdjust::IDENTITY))
        );
    }

    #[test]
    fn a_look_needs_a_name() {
        let mut looks = UserLooks::default();
        assert!(looks.save("   ", dreamy()).is_err());
    }

    #[test]
    fn the_list_has_a_limit() {
        let mut looks = UserLooks::default();
        for n in 0..MAX_LOOKS {
            looks.save(&format!("Look {n}"), dreamy()).expect("saved");
        }
        assert!(looks.save("One more", dreamy()).is_err());
        // Replacing one that is already there still works.
        assert!(
            looks
                .save("Look 0", SavedLook::graded(ColorAdjust::IDENTITY))
                .is_ok()
        );
    }

    #[test]
    fn looks_round_trip_through_the_file_they_are_written_as() {
        let mut looks = UserLooks::default();
        looks.save("My camera", dreamy()).expect("saved");
        looks
            .save("Flat", SavedLook::graded(ColorAdjust::IDENTITY))
            .expect("saved");

        let back = parse(&render(looks.all()));
        assert_eq!(back, looks.all().to_vec());
    }

    /// A file from before vibrance existed still reads, with none of it.
    #[test]
    fn an_older_file_without_vibrance_still_reads() {
        let back = parse("Old\t1.1\t1.2\t0.9\t0.1\t0\n");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].1.colour.vibrance, 0.0);
        assert!((back[0].1.colour.brightness - 1.1).abs() < 1e-6);
    }

    /// And one from before a look carried its finish: the grade reads, and the
    /// finish is nothing, which is what a grade-only look was.
    #[test]
    fn an_older_file_without_the_finish_still_reads() {
        let back = parse("Old\t1.1\t1.2\t0.9\t0.1\t0\t0.2\n");
        assert_eq!(back.len(), 1);
        assert!((back[0].1.colour.vibrance - 0.2).abs() < 1e-6);
        assert!(!back[0].1.has_effects(), "{:?}", back[0].1);
    }

    #[test]
    fn a_look_knows_whether_it_is_more_than_a_grade() {
        assert!(dreamy().has_effects());
        assert!(!SavedLook::graded(warm()).has_effects());
    }

    #[test]
    fn a_garbled_line_is_skipped_rather_than_fatal() {
        let back = parse("# a comment\nbroken\nAlso broken\t1\t2\nGood\t1\t1\t1\t0\t0\t0\n");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].0, "Good");
    }
}
