//! The templates that ship with the editor (Milestone 11).
//!
//! Compiled in rather than read from an install folder, so they cannot go
//! missing and a fresh install has something in the browser on first launch.
//! They go through [`crate::parse`] exactly like a downloaded template — a
//! starter gets no trust a stranger's file would not — and a test holds every
//! one of them to it, so a broken starter fails the build rather than
//! disappearing from the list on a user's machine.

use crate::{Template, parse};

/// File name and contents, in the order the browser lists them.
pub const FILES: [(&str, &str); 7] = [
    (
        "quick-intro.json",
        include_str!("../starters/quick-intro.json"),
    ),
    (
        "two-shot-promo.json",
        include_str!("../starters/two-shot-promo.json"),
    ),
    (
        "picture-in-picture.json",
        include_str!("../starters/picture-in-picture.json"),
    ),
    (
        "photo-montage.json",
        include_str!("../starters/photo-montage.json"),
    ),
    ("outro.json", include_str!("../starters/outro.json")),
    ("meme.json", include_str!("../starters/meme.json")),
    (
        "punchy-reel.json",
        include_str!("../starters/punchy-reel.json"),
    ),
];

/// Every starter that validates. All of them, unless a test has failed.
pub fn starters() -> Vec<Template> {
    FILES
        .iter()
        .filter_map(|(_, json)| parse(json).ok())
        .collect()
}
