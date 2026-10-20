//! rotatefont's per-glyph table (`-rtf`): one line per glyph, `source output advance x y`.
//!
//! - Blank lines and lines whose first non-blank character is `#` are skipped.
//! - `advance` is a number or `None` (the advance follows the widths policy).
//! - `x` and `y` replace the matrix's shift for that glyph.
//! - A source name that starts with a digit is a CID; every line names glyphs the same way.

use std::collections::BTreeMap;

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Most lines a glyph table may hold.
pub const MAX_GLYPH_TABLE_LINES: usize = 65_536;
/// Longest glyph table file, in bytes.
pub const MAX_GLYPH_TABLE_BYTES: usize = 8 << 20;
/// Longest glyph name or CID a line may give (rotatefont reads at most 127 characters).
pub const MAX_GLYPH_TABLE_NAME: usize = 127;
/// Largest offset a line may give, either way.
const MAX_OFFSET: f64 = 1.0e6;

/// One line of a glyph table.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphTableEntry {
    pub source: GlyphKey,
    pub output: GlyphKey,
    /// The advance width to write; `None` follows the widths policy.
    pub advance: Option<f64>,
    /// The shift that replaces the matrix's for this glyph.
    pub offset: (f64, f64),
}

/// A glyph by name, or by CID in a CID-keyed font.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GlyphKey {
    Name(String),
    Cid(u16),
}

/// What a glyph table sets for one glyph, by glyph id.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlyphSettings {
    pub offset: (f64, f64),
    pub advance: Option<f64>,
}

/// The lines of a glyph table. Refuses a line without five fields, numbers that are not finite
/// or too large, a negative or too large advance, a table mixing names and CIDs, a source listed
/// twice, and a table past [`MAX_GLYPH_TABLE_LINES`] lines or [`MAX_GLYPH_TABLE_BYTES`] bytes.
pub fn parse_glyph_table(text: &str) -> Result<Vec<GlyphTableEntry>> {
    if text.len() > MAX_GLYPH_TABLE_BYTES {
        return Err(TablesError::TooLarge { what: "a glyph table", limit: MAX_GLYPH_TABLE_BYTES });
    }
    let mut entries = Vec::new();
    let mut cids: Option<bool> = None;
    let mut seen = std::collections::BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let n = index + 1;
        if n > MAX_GLYPH_TABLE_LINES {
            return Err(TablesError::TooLarge { what: "the lines of a glyph table", limit: MAX_GLYPH_TABLE_LINES });
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = |why: &str| TablesError::BadOptions(format!("glyph table line {n}: {why}"));
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [source, output, advance, x, y] = fields.as_slice() else {
            return Err(bad("give five fields: source name, output name, advance width or None, x offset, y offset"));
        };
        let is_cid = source.starts_with(|c: char| c.is_ascii_digit());
        if *cids.get_or_insert(is_cid) != is_cid {
            return Err(bad("every line must name glyphs the same way, all by name or all by CID"));
        }
        let key = |text: &str| -> Result<GlyphKey> {
            if text.len() > MAX_GLYPH_TABLE_NAME {
                return Err(bad(&format!("a name is longer than {MAX_GLYPH_TABLE_NAME} characters")));
            }
            if is_cid {
                text.parse::<u16>().map(GlyphKey::Cid).map_err(|_| bad(&format!("`{text}` is not a CID from 0 to 65,535")))
            } else {
                Ok(GlyphKey::Name(text.to_string()))
            }
        };
        let number = |text: &str, what: &str, limit: f64| -> Result<f64> {
            text.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && v.abs() <= limit)
                .ok_or_else(|| bad(&format!("the {what} `{text}` is not a number within ±{limit}")))
        };
        let advance = match *advance {
            "None" => None,
            text => {
                let v = number(text, "advance width", 65_535.0)?;
                if v < 0.0 {
                    return Err(bad("the advance width is negative"));
                }
                Some(v)
            }
        };
        let source = key(source)?;
        if !seen.insert(source.clone()) {
            return Err(bad("this source glyph is listed already"));
        }
        entries.push(GlyphTableEntry {
            source,
            output: key(output)?,
            advance,
            offset: (number(x, "x offset", MAX_OFFSET)?, number(y, "y offset", MAX_OFFSET)?),
        });
    }
    Ok(entries)
}

/// The CID of every glyph of a CID-keyed `CFF ` font, by glyph id; `None` for any other font.
pub fn glyph_cids(font: &SfntFont) -> Result<Option<Vec<u16>>> {
    use read_fonts::{FontData, FontRead};
    let Some(data) = font.get(*b"CFF ") else { return Ok(None) };
    if crate::dehint::cff::Table::read(data, false)?.font_dicts.is_none() {
        return Ok(None);
    }
    let cff = read_fonts::tables::cff::Cff::read(FontData::new(data)).map_err(|e| TablesError::Invalid(format!("the CFF table: {e}")))?;
    let Some(charset) = cff.charset(0) else { return Ok(None) };
    let count = charset.num_glyphs().min(65_536);
    let mut out = Vec::with_capacity(count as usize);
    for gid in 0..count {
        let sid = charset
            .string_id(read_fonts::types::GlyphId::new(gid))
            .ok_or_else(|| TablesError::Invalid(format!("the CFF charset has no CID for glyph {gid}")))?;
        out.push(sid.to_u16());
    }
    Ok(Some(out))
}

/// The settings of a glyph table by glyph id, given how to find a glyph id for a key.
pub fn settings_by_glyph(entries: &[GlyphTableEntry], mut glyph: impl FnMut(&GlyphKey) -> Option<u16>) -> Result<BTreeMap<u16, GlyphSettings>> {
    let mut out = BTreeMap::new();
    for e in entries {
        let gid = glyph(&e.source).ok_or_else(|| {
            TablesError::BadOptions(match &e.source {
                GlyphKey::Name(n) => format!("the glyph table names `{n}`, which the font does not have"),
                GlyphKey::Cid(c) => format!("the glyph table names CID {c}, which the font does not have"),
            })
        })?;
        out.insert(gid, GlyphSettings { offset: e.offset, advance: e.advance });
    }
    Ok(out)
}
