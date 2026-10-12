//! Spline Font Database sources (L3): reading `.sfd` files and `.sfdir` folders into the model.
//!
//! Written only from the format's public description (the "Spline Font Database" and "Changes
//! to the sfd format" pages of the originating editor's technical reference) and from files we
//! write ourselves; no code that reads or writes the format was consulted.
//!
//! An SFD file is ASCII text, one `Keyword: value` per line: a font header, then one
//! `StartChar` … `EndChar` block per glyph, then optional bitmap strikes. A `.sfdir` folder holds
//! the same header in `font.props` and one `<name>.glyph` file per glyph.
//!
//! What is read:
//!
//! | SFD | Model |
//! |---|---|
//! | `FamilyName`, `FontName`, `FullName`, `Copyright`, `LangName`, `OS2Vendor` | family name, master name and properties (localised where the name table entry is) |
//! | `Version`, `Ascent`, `Descent`, `ItalicAngle`, `CreationTime`, `UComments` | version, units per em (ascent plus descent), metrics, date, note |
//! | OS/2, hhea and underline values with absolute values | the master's parameters |
//! | `Layer` (font) | layer 1 is the master layer, layer 0 its background, the others extra layers |
//! | `SplineSet` contours | paths; a quadratic layer's curves become quadratic segments |
//! | `Refer` | components, resolved by glyph index, then by code point |
//! | `AnchorPoint` | anchors, named as the reference names mark and ligature anchors |
//! | `Kerns2`, `KernClass2` in horizontal kerning lookups | pairs and kerning groups |
//! | `Encoding`, `AltUni2`, `Width`, `VWidth`, `GlyphClass`, `Comment`, `Colour` | code points, advance, vertical advance, OpenType category, note, colour |
//!
//! Everything else (lookups and their rules, hints, instructions, tables, images, bitmap strikes,
//! TeX and MATH data, unknown keywords) is skipped and listed in [`Import::skipped`], one entry
//! per kind of data with a count, so the user can see what did not come across.
//!
//! The input is untrusted: every count it states is capped by the limits below, numbers must be
//! finite and within the model's coordinate range, and the reader is a flat state machine over
//! lines, so it has no recursion. Malformed structure (a contour that does not start with a
//! move, a block that never ends, a bad number in geometry) fails with a message naming the line.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use fontcraft_model::{Font, ModelError};

mod build;
mod glyph;
mod header;
mod lines;
#[cfg(test)]
mod tests;

/// Largest `.sfd` file, or sum of a `.sfdir` folder's files, the reader accepts.
pub const MAX_FILE_BYTES: usize = 256 << 20;
/// Longest line, in bytes, including a quoted string continued over several lines.
pub const MAX_LINE_BYTES: usize = 4 << 20;
/// Most points in all contours of the file together.
pub const MAX_TOTAL_POINTS: usize = 10_000_000;
/// Most layers a font may declare, and the largest layer number a glyph may use.
pub const MAX_LAYERS: usize = 256;
/// Most classes on one side of a class kerning table.
pub const MAX_KERN_CLASSES: usize = 65_536;
/// Most values one class kerning table may hold (first classes × second classes).
pub const MAX_KERN_CELLS: usize = 4_000_000;
/// Most kerning pairs the import creates, glyph pairs and class pairs together.
pub const MAX_KERN_PAIRS: usize = 1_000_000;
/// Most anchors one glyph may have.
pub const MAX_ANCHORS_PER_GLYPH: usize = 4_096;
/// Most files a `.sfdir` folder may hold.
pub const MAX_SFDIR_FILES: usize = fontcraft_model::MAX_GLYPHS + 1_024;
/// Most distinct kinds of skipped data listed; the rest are counted under one entry.
pub const MAX_SKIPPED_KINDS: usize = 1_000;

/// A `.sfdir` folder's files: relative path (with `/`) → contents.
pub type Files = BTreeMap<String, Vec<u8>>;

#[derive(Debug, thiserror::Error)]
pub enum SfdError {
    #[error("not an SFD source: {0}")]
    NotSfd(String),
    #[error("the source is too large ({0} bytes; the limit is {MAX_FILE_BYTES})")]
    TooLarge(usize),
    #[error("{at}: {message}")]
    Syntax { at: String, message: String },
    #[error("not supported: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Model(#[from] ModelError),
}

pub type Result<T> = std::result::Result<T, SfdError>;

/// One kind of data the import did not keep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// What was skipped, in plain words (`TrueType instructions`, `unknown keyword "Foo"`).
    pub what: String,
    /// How many times it occurred.
    pub count: usize,
    /// Where it first occurred (`line 12`, `A.glyph, line 3`).
    pub first_at: String,
}

/// The result of an import: the font and what was left out of it.
#[derive(Clone, Debug)]
pub struct Import {
    pub font: Font,
    /// In the order each kind was first met.
    pub skipped: Vec<Skipped>,
}

/// Whether `bytes` start like an SFD file (after an optional byte-order mark).
pub fn is_sfd(bytes: &[u8]) -> bool {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF".as_slice()).unwrap_or(bytes);
    b.starts_with(b"SplineFontDB:")
}

/// Reads an `.sfd` file.
pub fn read_sfd(bytes: &[u8]) -> Result<Import> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(SfdError::TooLarge(bytes.len()));
    }
    let text = decode(bytes)?;
    let mut report = Report::default();
    let mut lines = lines::Lines::new("", text);
    let mut raw = header::RawFont::default();
    header::read_file(&mut lines, &mut raw, &mut report, true)?;
    build::build(raw, &mut report, false).map(|font| Import { font, skipped: report.into_list() })
}

/// Reads a `.sfdir` folder from its files: `font.props` and every top-level `<name>.glyph`.
/// Bitmap strikes (`<size>.strike/`) and CID subfonts (`<name>.subfont/`) are reported as skipped.
pub fn read_sfdir(files: &Files) -> Result<Import> {
    if files.len() > MAX_SFDIR_FILES {
        return Err(SfdError::Unsupported(format!("the folder holds more than {MAX_SFDIR_FILES} files")));
    }
    let total = files.values().try_fold(0usize, |sum, f| sum.checked_add(f.len())).unwrap_or(usize::MAX);
    if total > MAX_FILE_BYTES {
        return Err(SfdError::TooLarge(total));
    }
    let props = files.get("font.props").ok_or_else(|| SfdError::NotSfd("the folder has no font.props file".into()))?;
    let mut report = Report::default();
    let mut raw = header::RawFont::default();
    let text = decode(props)?;
    header::read_file(&mut lines::Lines::new("font.props", text), &mut raw, &mut report, true)?;
    for (path, bytes) in files {
        if path == "font.props" {
            continue;
        }
        if path.contains('/') {
            let what = if path.split('/').next().is_some_and(|d| d.ends_with(".strike")) {
                "bitmap strikes"
            } else if path.split('/').next().is_some_and(|d| d.ends_with(".subfont")) {
                "CID subfonts"
            } else {
                "files in subfolders"
            };
            report.skip(what, path);
            continue;
        }
        if !path.ends_with(".glyph") {
            report.skip("files other than glyphs", path);
            continue;
        }
        let text = decode(bytes)?;
        header::read_file(&mut lines::Lines::new(path, text), &mut raw, &mut report, false)?;
    }
    build::build(raw, &mut report, true).map(|font| Import { font, skipped: report.into_list() })
}

fn decode(bytes: &[u8]) -> Result<&str> {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF".as_slice()).unwrap_or(bytes);
    std::str::from_utf8(b).map_err(|e| SfdError::NotSfd(format!("the text is not UTF-8 (at byte {})", e.valid_up_to())))
}

/// The skipped-data list, merged by kind.
#[derive(Default)]
pub(crate) struct Report {
    kinds: Vec<Skipped>,
    index: BTreeMap<String, usize>,
}

impl Report {
    pub(crate) fn skip(&mut self, what: &str, at: &str) {
        self.skip_n(what, at, 1);
    }

    pub(crate) fn skip_n(&mut self, what: &str, at: &str, n: usize) {
        let key = if self.index.contains_key(what) || self.index.len() < MAX_SKIPPED_KINDS { what } else { "other data" };
        match self.index.get(key) {
            Some(&i) => {
                if let Some(k) = self.kinds.get_mut(i) {
                    k.count = k.count.saturating_add(n);
                }
            }
            None => {
                self.index.insert(key.to_string(), self.kinds.len());
                self.kinds.push(Skipped { what: key.to_string(), count: n, first_at: at.to_string() });
            }
        }
    }

    pub(crate) fn into_list(self) -> Vec<Skipped> {
        self.kinds
    }
}
