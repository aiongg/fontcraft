//! The native `.fontcraft` format (L3).
//!
//! A `.fontcraft` file is UTF-8 JSON:
//! ```json
//! { "format": "fontcraft", "version": 1, "generator": "FontCraft 0.0.1",
//!   "font": { …fontcraft_model::Font… } }
//! ```
//! It is lossless for everything in the model: keys this version does not know are kept in each
//! struct's `extra` map and written back. Readers reject a `version` newer than they support and
//! files larger than [`MAX_BYTES`]. Keys are written in a stable order with two-space indentation,
//! so files diff well under version control (`docs/spec/30-source-formats.md` › round-trip rules).
//!
//! Planned (see ROADMAP.md, M1): optional gzip, an embedded preview, package (directory) form.
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod atomic;

use fontcraft_model::Font;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
pub use atomic::write_atomic;

/// The version this build writes.
pub const VERSION: u32 = 1;
pub const FORMAT: &str = "fontcraft";
pub const EXTENSION: &str = "fontcraft";
/// Largest file [`load`] accepts (a guard against hostile input).
pub const MAX_BYTES: usize = 512 << 20;

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("not a FontCraft file: {0}")]
    NotFontcraft(String),
    #[error("this file was written by a newer FontCraft (format version {found}; this build reads up to {supported})")]
    TooNew { found: u32, supported: u32 },
    #[error("the file is too large ({0} bytes)")]
    TooLarge(usize),
    #[error("the font in the file is not valid: {0}")]
    Invalid(#[from] fontcraft_model::ModelError),
    #[error("cannot write the file: {0}")]
    Encode(String),
    #[error("{0}")]
    Io(String),
}

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'a str,
    version: u32,
    generator: String,
    font: &'a Font,
}

#[derive(Deserialize)]
struct Header {
    #[serde(default)]
    format: String,
    #[serde(default)]
    version: u32,
}

#[derive(Deserialize)]
struct FileIn {
    font: Font,
}

/// The bytes of a `.fontcraft` file holding `font`.
pub fn save(font: &Font) -> Result<Vec<u8>, FormatError> {
    let file = FileOut { format: FORMAT, version: VERSION, generator: format!("FontCraft {}", env!("CARGO_PKG_VERSION")), font };
    let mut bytes = serde_json::to_vec_pretty(&file).map_err(|e| FormatError::Encode(e.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Reads a `.fontcraft` file and checks the font it holds.
pub fn load(bytes: &[u8]) -> Result<Font, FormatError> {
    if bytes.len() > MAX_BYTES {
        return Err(FormatError::TooLarge(bytes.len()));
    }
    let header: Header = serde_json::from_slice(bytes).map_err(|e| FormatError::NotFontcraft(e.to_string()))?;
    if header.format != FORMAT {
        return Err(FormatError::NotFontcraft(format!("the `format` key is `{}`, expected `{FORMAT}`", header.format)));
    }
    if header.version > VERSION {
        return Err(FormatError::TooNew { found: header.version, supported: VERSION });
    }
    let file: FileIn = serde_json::from_slice(bytes).map_err(|e| FormatError::NotFontcraft(e.to_string()))?;
    file.font.check()?;
    Ok(file.font)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_round_trips_exactly() {
        let font = Font::new("Round Trip");
        let bytes = save(&font).unwrap();
        assert_eq!(load(&bytes).unwrap(), font);
        // Saving what was loaded gives the same bytes: the format is stable.
        assert_eq!(save(&load(&bytes).unwrap()).unwrap(), bytes);
    }

    #[test]
    fn newer_and_foreign_files_are_rejected_with_a_reason() {
        assert!(matches!(load(br#"{"format":"fontcraft","version":99,"font":{}}"#), Err(FormatError::TooNew { found: 99, .. })));
        assert!(matches!(load(br#"{"format":"other","version":1}"#), Err(FormatError::NotFontcraft(_))));
        assert!(matches!(load(b"\x00\x01\x00\x00"), Err(FormatError::NotFontcraft(_))));
        assert!(matches!(load(b"[1,2,3]"), Err(FormatError::NotFontcraft(_))));
        assert!(matches!(load(br#"{"format":"fontcraft","version":1}"#), Err(FormatError::NotFontcraft(_))));
    }

    #[test]
    fn an_invalid_font_is_rejected_on_load() {
        let mut font = Font::new("Bad");
        font.units_per_em = 1;
        let bytes = save(&font).unwrap();
        assert!(matches!(load(&bytes), Err(FormatError::Invalid(_))));
    }

    #[test]
    fn garbage_never_panics() {
        let good = save(&Font::new("Fuzz")).unwrap();
        // Truncations and single-byte corruptions of a valid file.
        for cut in (0..good.len()).step_by(97) {
            let _ = load(good.get(..cut).unwrap_or_default());
        }
        for i in (0..good.len()).step_by(53) {
            let mut bad = good.clone();
            if let Some(b) = bad.get_mut(i) {
                *b = b.wrapping_add(13);
            }
            let _ = load(&bad);
        }
    }
}
