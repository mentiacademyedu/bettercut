//! Cached thumbnails (§19).
//!
//! # Why raw pixels and not PNG
//!
//! A thumbnail is written once and read on every launch, by a UI that needs it
//! as an RGBA buffer to hand to the GPU. Storing PNG would mean pulling in an
//! image decoder — or routing every read back through FFmpeg — to arrive at
//! exactly the bytes we started with. §74 asks for a justification before any
//! large dependency, and "so the cache is smaller" is not one when the whole
//! file is 57 KB and lives in a directory that is disposable by design.
//!
//! The format is deliberately dull: a short header, then tightly packed RGBA.

use std::io::{Read, Write};
use std::path::Path;

use crate::error::CacheError;

/// `BCT1` — magic plus version, so a future format change is detectable rather
/// than being read as garbage pixels.
const MAGIC: [u8; 4] = *b"BCT1";
const HEADER_BYTES: usize = 12;

/// A decoded thumbnail, ready to upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl Thumbnail {
    /// Build one, rejecting anything whose buffer does not match its size.
    ///
    /// A mismatch here would become an out-of-bounds read in the UI, which is
    /// a much harder failure to trace than a rejected write.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, CacheError> {
        let expected = expected_len(width, height)?;
        if rgba.len() != expected {
            return Err(CacheError::MalformedThumbnail {
                detail: format!(
                    "{}x{} needs {expected} bytes, got {}",
                    width,
                    height,
                    rgba.len()
                ),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn write(&self, path: &Path) -> Result<(), CacheError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| io(parent, source))?;
        }

        // Write beside the target and rename, so an interrupted write cannot
        // leave a half-file that later reads as a valid header (§38.1's rule,
        // applied to the cache).
        let temp = path.with_extension("part");
        {
            let mut file = std::fs::File::create(&temp).map_err(|s| io(&temp, s))?;
            let mut put = |bytes: &[u8]| file.write_all(bytes).map_err(|s| io(&temp, s));
            put(&MAGIC)?;
            put(&self.width.to_le_bytes())?;
            put(&self.height.to_le_bytes())?;
            put(&self.rgba)?;
            file.flush().map_err(|s| io(&temp, s))?;
        }
        std::fs::rename(&temp, path).map_err(|s| io(path, s))?;
        Ok(())
    }

    pub fn read(path: &Path) -> Result<Self, CacheError> {
        let mut file = std::fs::File::open(path).map_err(|s| io(path, s))?;

        let mut header = [0_u8; HEADER_BYTES];
        file.read_exact(&mut header).map_err(|s| io(path, s))?;
        if header[..4] != MAGIC {
            return Err(CacheError::MalformedThumbnail {
                detail: "wrong magic; not a bettercut thumbnail".to_owned(),
            });
        }

        let width = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        let height = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        let expected = expected_len(width, height)?;

        // Read exactly what the header claims. Trusting the file's length
        // instead would let a truncated file through as a shorter image.
        let mut rgba = vec![0_u8; expected];
        file.read_exact(&mut rgba).map_err(|s| io(path, s))?;

        Ok(Self {
            width,
            height,
            rgba,
        })
    }
}

fn io(path: &Path, source: std::io::Error) -> CacheError {
    CacheError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Byte count for an image, refusing sizes that cannot be one.
fn expected_len(width: u32, height: u32) -> Result<usize, CacheError> {
    if width == 0 || height == 0 {
        return Err(CacheError::MalformedThumbnail {
            detail: format!("{width}x{height} has a zero dimension"),
        });
    }
    // A corrupted header must not turn into a huge allocation.
    if width > 8192 || height > 8192 {
        return Err(CacheError::MalformedThumbnail {
            detail: format!("{width}x{height} is implausibly large for a thumbnail"),
        });
    }
    Ok(width as usize * height as usize * 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Thumbnail {
        Thumbnail::new(4, 2, vec![7_u8; 4 * 2 * 4]).expect("valid")
    }

    #[test]
    fn a_thumbnail_round_trips_through_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("t.bct");
        let original = sample();

        original.write(&path).expect("write");
        assert_eq!(Thumbnail::read(&path).expect("read"), original);
    }

    #[test]
    fn a_buffer_that_does_not_match_its_size_is_refused() {
        assert!(Thumbnail::new(4, 2, vec![0; 10]).is_err());
    }

    #[test]
    fn zero_dimensions_are_refused() {
        assert!(Thumbnail::new(0, 4, Vec::new()).is_err());
        assert!(Thumbnail::new(4, 0, Vec::new()).is_err());
    }

    /// A stale or foreign file must be reported, not decoded as pixels.
    #[test]
    fn a_file_without_the_magic_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("t.bct");
        std::fs::write(&path, b"not a thumbnail at all").expect("write");

        let err = Thumbnail::read(&path).expect_err("should refuse");
        assert!(format!("{err}").contains("magic"), "{err}");
    }

    /// Truncation is the realistic corruption: a crash mid-write, or a cache
    /// copied while being written.
    #[test]
    fn a_truncated_file_is_refused_rather_than_read_short() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("t.bct");
        sample().write(&path).expect("write");

        let full = std::fs::read(&path).expect("read");
        std::fs::write(&path, &full[..full.len() - 5]).expect("truncate");

        assert!(Thumbnail::read(&path).is_err(), "a short file was accepted");
    }

    /// A corrupted header must not become a multi-gigabyte allocation.
    #[test]
    fn an_implausible_size_in_the_header_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("t.bct");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, &bytes).expect("write");

        assert!(Thumbnail::read(&path).is_err());
    }
}
