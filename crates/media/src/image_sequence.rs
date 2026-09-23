//! Numbered stills as one clip: `frame_0001.png`, `frame_0002.png`, … read
//! as a video at a chosen rate.
//!
//! What a render, a time-lapse camera or a screen recorder hands over: one
//! file a frame. Importing them one at a time makes a thousand five-second
//! photos; importing them as a sequence makes the forty seconds of picture
//! they are. FFmpeg's `image2` demuxer reads a numbered run from a printf
//! pattern (`frame_%04d.png`) given where to start and how fast to count, so
//! the asset keeps the first frame's path — a real file, so the library's
//! missing/relink checks work as they do for anything else — and beside it
//! the three numbers the pattern needs.
//!
//! Which frames belong: the run of files without a gap around the one that
//! was picked. A gap ends the sequence, because a missing frame in the
//! middle is either a different render or a real hole, and either way is
//! the person's to notice rather than something to paper over.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The most frames one sequence is allowed to be: a hundred thousand is a
/// little over an hour at 25, and past that the directory scan is what is
/// slow, not the edit.
pub const MAX_SEQUENCE_FRAMES: u32 = 100_000;

/// How a numbered run of stills is read as one clip, kept on the asset
/// beside the first frame's path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSequence {
    /// The number in the first frame's name.
    pub start: u32,
    /// How many digits the number is written with, zero-padded.
    pub digits: usize,
    /// How many frames follow on from `start`, without a gap.
    pub count: u32,
}

/// The parts of a numbered file name: `frame_0007.png` is `frame_`, 7 in
/// four digits, and `.png`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Numbered {
    pub prefix: String,
    pub number: u32,
    pub digits: usize,
    /// With its dot; empty for a file without one.
    pub extension: String,
}

impl Numbered {
    /// The file name of frame `number` in this run.
    pub fn name_of(&self, number: u32) -> String {
        format!(
            "{}{:0width$}{}",
            self.prefix,
            number,
            self.extension,
            width = self.digits
        )
    }

    /// The printf pattern `image2` reads the run from: `frame_%04d.png`.
    pub fn pattern(&self) -> String {
        format!("{}%0{}d{}", self.prefix, self.digits, self.extension)
    }
}

/// The number at the end of `path`'s stem, if there is one it can count on
/// (up to nine digits: more is a hash or a timestamp, not a frame number).
pub fn numbered(path: &Path) -> Option<Numbered> {
    let stem = path.file_stem()?.to_str()?;
    let digits = stem.chars().rev().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let split = stem.len() - digits;
    let number = stem[split..].parse().ok()?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    Some(Numbered {
        prefix: stem[..split].to_owned(),
        number,
        digits,
        extension,
    })
}

/// The pattern path for the run `path` is one frame of: its own folder, with
/// the number replaced by `%0Nd`.
pub fn pattern_for(path: &Path, digits: usize) -> Option<PathBuf> {
    let mut parts = numbered(path)?;
    parts.digits = digits;
    Some(path.with_file_name(parts.pattern()))
}

/// The run of frames around `path`: back to the first without a gap, forward
/// to the last. `None` for a file that is not numbered or stands alone —
/// one still is a still.
pub fn sequence_at(path: &Path) -> Option<ImageSequence> {
    let parts = numbered(path)?;
    let folder = path.parent()?;
    let exists = |number: u32| folder.join(parts.name_of(number)).is_file();

    let mut start = parts.number;
    while start > 0 && exists(start - 1) {
        start -= 1;
    }
    let mut count = 0;
    while count < MAX_SEQUENCE_FRAMES && exists(start + count) {
        count += 1;
    }
    (count > 1).then_some(ImageSequence {
        start,
        digits: parts.digits,
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_numbered_name_splits_into_its_parts() {
        let parts = numbered(Path::new("C:/shots/frame_0007.png")).unwrap();
        assert_eq!(parts.prefix, "frame_");
        assert_eq!((parts.number, parts.digits), (7, 4));
        assert_eq!(parts.extension, ".png");
        assert_eq!(parts.name_of(12), "frame_0012.png");
        assert_eq!(parts.pattern(), "frame_%04d.png");
        assert_eq!(
            pattern_for(Path::new("C:/shots/frame_0007.png"), 4).unwrap(),
            Path::new("C:/shots/frame_%04d.png")
        );
    }

    #[test]
    fn a_name_without_a_number_is_not_a_frame() {
        assert!(numbered(Path::new("photo.png")).is_none());
        assert!(numbered(Path::new("IMG_20240101123045678.jpg")).is_none());
        let bare = numbered(Path::new("0042")).unwrap();
        assert_eq!(
            (bare.prefix.as_str(), bare.number, bare.extension.as_str()),
            ("", 42, "")
        );
    }
}
