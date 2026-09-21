//! Your own keys for the shortcuts: any action in the Shortcuts window can be
//! moved to another key.
//!
//! A remapping rather than a second table of actions: the handler still speaks
//! in the keys the sheet was written for, and this says which key on the
//! keyboard now stands for each of them. So every action — and its modifier
//! variants, Shift and Ctrl and Alt — moves together, and nothing about what
//! a shortcut does changes.
//!
//! Kept as plain `default=key` lines beside the recent projects list, and like
//! that list, a file that is missing or garbled is simply no remapping.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::Key;

use crate::shortcuts::HANDLED_KEYS;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    /// Default key → the key that now does its job. Only keys that moved.
    moved: BTreeMap<Key, Key>,
    /// Where the map is kept; `None` keeps it in memory only.
    file: Option<PathBuf>,
}

impl Keymap {
    /// The map kept in `file`, reading whatever is there now.
    pub fn stored_in(file: PathBuf) -> Self {
        let moved = std::fs::read_to_string(&file)
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self {
            moved,
            file: Some(file),
        }
    }

    /// Where the running editor keeps its keys: beside the recent projects.
    pub fn default_file() -> PathBuf {
        crate::recent::RecentProjects::default_file().with_file_name("keys.txt")
    }

    /// The key on the keyboard that does `default`'s job now.
    pub fn key_for(&self, default: Key) -> Key {
        self.moved.get(&default).copied().unwrap_or(default)
    }

    /// Whether `default` has been moved to another key.
    pub fn is_moved(&self, default: Key) -> bool {
        self.moved.contains_key(&default)
    }

    /// Move `default`'s job to `to`. Refused, with the reason, when `to`
    /// already does another shortcut's job, or for Escape — always the way
    /// out, so it cannot be given away.
    pub fn rebind(&mut self, default: Key, to: Key) -> Result<(), String> {
        if !HANDLED_KEYS.contains(&default) {
            return Err(format!("{} is not a shortcut", default.name()));
        }
        if default == Key::Escape || to == Key::Escape {
            return Err("Escape is always the way out, and cannot be moved".to_owned());
        }
        if let Some(taken) = HANDLED_KEYS
            .iter()
            .copied()
            .find(|other| *other != default && self.key_for(*other) == to)
        {
            return Err(format!(
                "{} already does what {} does by default — move that one first",
                to.name(),
                taken.name()
            ));
        }
        if to == default {
            self.moved.remove(&default);
        } else {
            self.moved.insert(default, to);
        }
        self.store();
        Ok(())
    }

    /// Every shortcut back on its own key.
    pub fn reset_all(&mut self) {
        self.moved.clear();
        self.store();
    }

    fn store(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let text: String = self
            .moved
            .iter()
            .map(|(from, to)| format!("{}={}\n", from.name(), to.name()))
            .collect();
        let written = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(file, text));
        if let Err(err) = written {
            tracing::warn!(file = %file.display(), %err, "could not save the keyboard shortcuts");
        }
    }

    /// Where this map is kept, if anywhere.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }
}

/// `default=key` lines; anything unreadable, unknown or clashing is skipped.
fn parse(text: &str) -> BTreeMap<Key, Key> {
    let mut map = Keymap::default();
    for line in text.lines() {
        let Some((from, to)) = line.split_once('=') else {
            continue;
        };
        if let (Some(from), Some(to)) = (Key::from_name(from.trim()), Key::from_name(to.trim())) {
            // Through `rebind`, so a hand-edited file cannot give two jobs to
            // one key. `file` is `None` here, so nothing is written back.
            let _ = map.rebind(from, to);
        }
    }
    map.moved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moved_key_does_the_old_ones_job_and_the_old_one_is_free() {
        let mut map = Keymap::default();
        map.rebind(Key::S, Key::X).unwrap_err(); // X is Cut's
        map.rebind(Key::S, Key::B).unwrap();
        assert_eq!(map.key_for(Key::S), Key::B);
        assert!(map.is_moved(Key::S));
        // S itself now does nothing, so another job can take it.
        map.rebind(Key::M, Key::S).unwrap();
        assert_eq!(map.key_for(Key::M), Key::S);
        // Back to its own key clears the move.
        map.rebind(Key::S, Key::S).unwrap_err(); // M holds S now
        map.rebind(Key::M, Key::M).unwrap();
        map.rebind(Key::S, Key::S).unwrap();
        assert!(!map.is_moved(Key::S));
        assert!(map.rebind(Key::Escape, Key::B).is_err());
        assert!(map.rebind(Key::S, Key::Escape).is_err());
    }

    #[test]
    fn the_file_round_trips_and_nonsense_is_ignored() {
        let dir = std::env::temp_dir().join(format!("bettercut-keys-{}", std::process::id()));
        let file = dir.join("keys.txt");
        let _ = std::fs::remove_file(&file);
        let mut map = Keymap::stored_in(file.clone());
        map.rebind(Key::S, Key::B).unwrap();
        let again = Keymap::stored_in(file.clone());
        assert_eq!(again.key_for(Key::S), Key::B);

        std::fs::write(&file, "S=B\nnonsense\nM=B\nQ=Nope\n").unwrap();
        let odd = Keymap::stored_in(file.clone());
        assert_eq!(odd.key_for(Key::S), Key::B);
        assert_eq!(odd.key_for(Key::M), Key::M, "a clash was let in");
        let _ = std::fs::remove_dir_all(dir);
    }
}
