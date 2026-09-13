//! Colour lookup tables: a grade someone else made, applied in one step.
//!
//! A `.cube` file is the format every grading tool writes — Resolve, Premiere,
//! the LUT packs sold for "the teal and orange look". It is a 3D grid: for a
//! sampling of input colours, the colour each becomes. Between the samples the
//! renderer interpolates.
//!
//! This module is the file format and the model, with no GPU in sight: the
//! parser the editor validates an import with and the renderer uploads from,
//! and a CPU lookup the tests hold the GPU's result against.
//!
//! ## What is accepted
//!
//! Adobe's `.cube` specification, 3D tables only: `LUT_3D_SIZE` from 2 to
//! [`MAX_LUT_SIZE`], an optional `DOMAIN_MIN`/`DOMAIN_MAX`, an optional
//! `TITLE`, `#` comments, and exactly size³ rows of three numbers with red
//! changing fastest. A 1D table (`LUT_1D_SIZE`) is refused with a message
//! saying so, rather than misread as a broken 3D one.
//!
//! ## Which colours it expects
//!
//! A creative LUT is built for display-encoded values — the numbers in an
//! ordinary video file — not for linear light. The renderer works in linear
//! light (§21a.1), so it encodes to sRGB before the lookup and decodes after,
//! and this module's [`CubeLut::apply`] works on encoded values to match.

use bettercut_foundation::LutId;
use serde::{Deserialize, Serialize};

/// The largest table accepted: 65³ is the finest any common tool writes, and a
/// quarter of a million entries is where a hand-edited mistake would start
/// costing real memory.
pub const MAX_LUT_SIZE: u32 = 65;

/// A LUT applied to a clip: which one, and how much of it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClipLut {
    pub lut: LutId,
    /// 0 is none of the grade, 1 is all of it. A way to ease a strong look
    /// back, which a LUT file on its own cannot do.
    #[serde(default = "full")]
    pub strength: f32,
}

fn full() -> f32 {
    1.0
}

impl ClipLut {
    pub fn new(lut: LutId) -> Self {
        Self { lut, strength: 1.0 }
    }

    /// Brought into range, as every value a project file can carry is.
    pub fn clamped(self) -> Self {
        Self {
            lut: self.lut,
            strength: if self.strength.is_finite() {
                self.strength.clamp(0.0, 1.0)
            } else {
                1.0
            },
        }
    }
}

/// A parsed 3D table.
#[derive(Debug, Clone, PartialEq)]
pub struct CubeLut {
    pub title: Option<String>,
    /// Samples along each axis.
    pub size: u32,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// `size³` output colours, red changing fastest, then green, then blue.
    pub table: Vec<[f32; 3]>,
}

/// Why a `.cube` file was not accepted, said in terms the user can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LutError {
    #[error("this is a 1D LUT; only 3D LUTs (LUT_3D_SIZE) are supported")]
    OneDimensional,
    #[error("the file does not say how big the table is (no LUT_3D_SIZE line)")]
    NoSize,
    #[error("LUT_3D_SIZE {0} is outside the supported 2 to 65")]
    BadSize(i64),
    #[error("line {line}: {problem}")]
    BadLine { line: usize, problem: String },
    #[error("the table has {found} entries; a size-{size} LUT needs {expected}")]
    WrongCount {
        size: u32,
        expected: usize,
        found: usize,
    },
    #[error("DOMAIN_MIN must be below DOMAIN_MAX on every channel")]
    BadDomain,
}

/// The largest `.cube` file read. A 65³ table written with generous precision
/// is about 8 MB; past this it is not a LUT.
pub const MAX_CUBE_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Read and parse a `.cube` file, size-capped before it is read (§64).
///
/// The error is a sentence naming the file, ready to show: the import and the
/// renderer's loaders both only ever report it.
pub fn load_cube_file(path: &std::path::Path) -> Result<CubeLut, String> {
    let name = path.display();
    let size = std::fs::metadata(path)
        .map_err(|err| format!("could not read {name}: {err}"))?
        .len();
    if size > MAX_CUBE_FILE_BYTES {
        return Err(format!(
            "{name} is {} MB; LUT files are limited to {} MB",
            size / (1024 * 1024),
            MAX_CUBE_FILE_BYTES / (1024 * 1024)
        ));
    }
    let text =
        std::fs::read_to_string(path).map_err(|err| format!("could not read {name}: {err}"))?;
    parse_cube(&text).map_err(|err| format!("{name}: {err}"))
}

/// Read a `.cube` file's text.
pub fn parse_cube(text: &str) -> Result<CubeLut, LutError> {
    let mut title = None;
    let mut size: Option<u32> = None;
    let mut domain_min = [0.0_f32; 3];
    let mut domain_max = [1.0_f32; 3];
    let mut table: Vec<[f32; 3]> = Vec::new();

    let three = |line: usize, words: &[&str]| -> Result<[f32; 3], LutError> {
        let bad = |problem: &str| LutError::BadLine {
            line,
            problem: problem.to_owned(),
        };
        if words.len() != 3 {
            return Err(bad("expected three numbers"));
        }
        let mut out = [0.0_f32; 3];
        for (slot, word) in out.iter_mut().zip(words) {
            let value: f32 = word.parse().map_err(|_| bad("not a number"))?;
            if !value.is_finite() {
                return Err(bad("not a finite number"));
            }
            *slot = value;
        }
        Ok(out)
    };

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let content = raw.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let mut words = content.split_whitespace();
        let Some(first) = words.next() else {
            continue;
        };
        match first {
            "TITLE" => {
                let rest = content["TITLE".len()..].trim().trim_matches('"');
                title = Some(rest.to_owned());
            }
            "LUT_1D_SIZE" => return Err(LutError::OneDimensional),
            "LUT_3D_SIZE" => {
                let value: i64 = words
                    .next()
                    .and_then(|w| w.parse().ok())
                    .ok_or(LutError::NoSize)?;
                if !(2..=i64::from(MAX_LUT_SIZE)).contains(&value) {
                    return Err(LutError::BadSize(value));
                }
                size = Some(value as u32);
            }
            "DOMAIN_MIN" => domain_min = three(line, &words.collect::<Vec<_>>())?,
            "DOMAIN_MAX" => domain_max = three(line, &words.collect::<Vec<_>>())?,
            // Keywords from other writers this table does not need.
            "LUT_3D_INPUT_RANGE" | "LUT_1D_INPUT_RANGE" => {}
            _ if first.starts_with(|c: char| c.is_ascii_alphabetic()) => {
                // An unknown keyword before the table starts is tolerated, as
                // the specification allows; one inside the table is a mistake.
                if !table.is_empty() {
                    return Err(LutError::BadLine {
                        line,
                        problem: format!("unexpected {first} inside the table"),
                    });
                }
            }
            _ => {
                let words: Vec<&str> = content.split_whitespace().collect();
                table.push(three(line, &words)?);
            }
        }
    }

    let size = size.ok_or(LutError::NoSize)?;
    let expected = (size as usize).pow(3);
    if table.len() != expected {
        return Err(LutError::WrongCount {
            size,
            expected,
            found: table.len(),
        });
    }
    if (0..3).any(|c| domain_min[c] >= domain_max[c]) {
        return Err(LutError::BadDomain);
    }
    Ok(CubeLut {
        title,
        size,
        domain_min,
        domain_max,
        table,
    })
}

impl CubeLut {
    /// A table that changes nothing, for tests and as a reference.
    pub fn identity(size: u32) -> Self {
        let size = size.clamp(2, MAX_LUT_SIZE);
        let step = 1.0 / (size - 1) as f32;
        let mut table = Vec::with_capacity((size as usize).pow(3));
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    table.push([r as f32 * step, g as f32 * step, b as f32 * step]);
                }
            }
        }
        Self {
            title: None,
            size,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            table,
        }
    }

    /// The entry at grid position (r, g, b).
    pub fn at(&self, r: u32, g: u32, b: u32) -> [f32; 3] {
        let n = self.size as usize;
        self.table[r as usize + g as usize * n + b as usize * n * n]
    }

    /// Look up one display-encoded colour, trilinearly — what the GPU does, on
    /// the CPU, for tests to compare against.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let last = (self.size - 1) as f32;
        let mut base = [0_u32; 3];
        let mut frac = [0.0_f32; 3];
        for c in 0..3 {
            let span = self.domain_max[c] - self.domain_min[c];
            let at = ((rgb[c] - self.domain_min[c]) / span).clamp(0.0, 1.0) * last;
            let floor = at.floor().min(last - 1.0);
            base[c] = floor as u32;
            frac[c] = at - floor;
        }
        let mut out = [0.0_f32; 3];
        for (dr, wr) in [(0, 1.0 - frac[0]), (1, frac[0])] {
            for (dg, wg) in [(0, 1.0 - frac[1]), (1, frac[1])] {
                for (db, wb) in [(0, 1.0 - frac[2]), (1, frac[2])] {
                    let v = self.at(base[0] + dr, base[1] + dg, base[2] + db);
                    let w = wr * wg * wb;
                    for c in 0..3 {
                        out[c] += v[c] * w;
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(size: u32, rows: &[[f32; 3]], header: &str) -> String {
        let mut text = format!("{header}\nLUT_3D_SIZE {size}\n");
        for row in rows {
            text.push_str(&format!("{} {} {}\n", row[0], row[1], row[2]));
        }
        text
    }

    #[test]
    fn a_table_is_read_with_its_title_domain_and_comments() {
        let identity = CubeLut::identity(2);
        let text = cube(
            2,
            &identity.table,
            "# made by hand\nTITLE \"Plain\"\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1",
        );
        let lut = parse_cube(&text).unwrap();
        assert_eq!(lut.title.as_deref(), Some("Plain"));
        assert_eq!(
            lut,
            CubeLut {
                title: Some("Plain".to_owned()),
                ..identity
            }
        );
    }

    #[test]
    fn red_changes_fastest() {
        let lut = CubeLut::identity(3);
        assert_eq!(lut.at(2, 0, 0), [1.0, 0.0, 0.0]);
        assert_eq!(
            lut.table[1],
            [0.5, 0.0, 0.0],
            "the second row is red's next step"
        );
    }

    #[test]
    fn a_1d_table_is_named_as_one() {
        assert_eq!(
            parse_cube("LUT_1D_SIZE 1024\n0 0 0\n"),
            Err(LutError::OneDimensional)
        );
    }

    #[test]
    fn broken_files_say_what_is_wrong() {
        assert_eq!(parse_cube("0 0 0\n"), Err(LutError::NoSize));
        assert_eq!(parse_cube("LUT_3D_SIZE 1\n"), Err(LutError::BadSize(1)));
        assert_eq!(parse_cube("LUT_3D_SIZE 300\n"), Err(LutError::BadSize(300)));
        assert!(matches!(
            parse_cube("LUT_3D_SIZE 2\n0 0 0\n"),
            Err(LutError::WrongCount {
                expected: 8,
                found: 1,
                ..
            })
        ));
        assert!(matches!(
            parse_cube("LUT_3D_SIZE 2\n0 zero 0\n"),
            Err(LutError::BadLine { line: 2, .. })
        ));
        assert!(matches!(
            parse_cube("LUT_3D_SIZE 2\n0 NaN 0\n"),
            Err(LutError::BadLine { line: 2, .. })
        ));
        let identity = CubeLut::identity(2);
        let text = cube(2, &identity.table, "DOMAIN_MIN 1 0 0\nDOMAIN_MAX 1 1 1");
        assert_eq!(parse_cube(&text), Err(LutError::BadDomain));
    }

    #[test]
    fn the_identity_table_changes_nothing_between_its_samples() {
        let lut = CubeLut::identity(5);
        for rgb in [
            [0.0, 0.0, 0.0],
            [0.13, 0.5, 0.97],
            [1.0, 1.0, 1.0],
            [0.33, 0.66, 0.2],
        ] {
            let out = lut.apply(rgb);
            for c in 0..3 {
                assert!((out[c] - rgb[c]).abs() < 1e-5, "{rgb:?} became {out:?}");
            }
        }
    }

    #[test]
    fn a_strength_out_of_range_is_brought_back() {
        let id = LutId::new();
        assert_eq!(
            ClipLut {
                lut: id,
                strength: 4.0
            }
            .clamped()
            .strength,
            1.0
        );
        assert_eq!(
            ClipLut {
                lut: id,
                strength: -1.0
            }
            .clamped()
            .strength,
            0.0
        );
        assert_eq!(
            ClipLut {
                lut: id,
                strength: f32::NAN
            }
            .clamped()
            .strength,
            1.0
        );
    }
}
