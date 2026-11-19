//! Patching table fields, for the audit's fixes on compiled fonts (M12.61; the `tables.*`
//! options `patchTable`, `cleanNameRecords`, `rebuildCmap`, `cleanInstances`, `dropAxis`,
//! `recompileLayout`).
//!
//! Every function takes the font and returns a changed copy; a function that fails returns an
//! error and the caller's font is untouched. Only the tables a function names are rewritten, the
//! others keep their bytes. The fixed-layout header tables (`head`, `hhea`, `vhea`, `OS/2`,
//! `post`, `maxp`) are patched in place, byte for byte, so fields no fix names keep their exact
//! values; the variable tables are written again.
//!
//! What to write is decided by the caller (the engine's `tables.recalculate` computes derived
//! values from the font); this module only knows where each field lives.

mod autohint_cff;
mod cff;
mod cmap;
mod fvar;
mod layout;
mod names;
mod recompile_cff;
#[cfg(test)]
mod tests;

pub use autohint_cff::{CffAutohint, CffAutohintOptions, CffGlyph, CffHintSetup, CffHinted, CffSkipped, autohint_cff, cff_hint_setup};
pub use cff::{CFF_FONT_BBOX, CFF_IS_FIXED_PITCH, cff_font_bbox, cff_top_ints, set_cff_top_ints};
pub use cmap::{CmapConflict, CmapRebuild, rebuild_cmap};
pub use fvar::{InstanceCleanup, clean_instances, drop_axis};
pub use layout::recompile_layout;
pub use names::{NameCleanup, NameOptions, clean_name_records};
pub use recompile_cff::{CffRecompile, recompile_cff};

use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

/// A new value for one field.
#[derive(Clone, Debug, PartialEq)]
pub enum PatchValue {
    Number(f64),
    Text(String),
}

/// What [`patch_table`] changes.
#[derive(Clone, Debug, PartialEq)]
pub struct PatchOptions {
    pub table: Tag,
    /// Field name (as `tables show` names it) → new value.
    pub fields: Vec<(String, PatchValue)>,
}

/// How a field is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    U16,
    I16,
    U32,
    /// 16.16 fixed point.
    Fixed,
    /// Four ASCII characters (`achVendID`).
    Tag,
    /// Ten bytes, written as ten numbers (`panose`).
    Panose,
}

impl Kind {
    fn size(self) -> usize {
        match self {
            Kind::U16 | Kind::I16 => 2,
            Kind::U32 | Kind::Fixed | Kind::Tag => 4,
            Kind::Panose => 10,
        }
    }
}

/// The fields [`patch_table`] can write: table, name, byte offset, kind. Offsets are those of
/// the OpenType specification; an `OS/2` field past the end of an older table version is
/// refused.
const FIELDS: &[(&Tag, &str, usize, Kind)] = &[
    (b"head", "fontRevision", 4, Kind::Fixed),
    (b"head", "flags", 16, Kind::U16),
    (b"head", "unitsPerEm", 18, Kind::U16),
    (b"head", "xMin", 36, Kind::I16),
    (b"head", "yMin", 38, Kind::I16),
    (b"head", "xMax", 40, Kind::I16),
    (b"head", "yMax", 42, Kind::I16),
    (b"head", "macStyle", 44, Kind::U16),
    (b"head", "lowestRecPPEM", 46, Kind::U16),
    (b"head", "fontDirectionHint", 48, Kind::I16),
    (b"hhea", "ascender", 4, Kind::I16),
    (b"hhea", "descender", 6, Kind::I16),
    (b"hhea", "lineGap", 8, Kind::I16),
    (b"hhea", "advanceWidthMax", 10, Kind::U16),
    (b"hhea", "minLeftSideBearing", 12, Kind::I16),
    (b"hhea", "minRightSideBearing", 14, Kind::I16),
    (b"hhea", "xMaxExtent", 16, Kind::I16),
    (b"hhea", "caretSlopeRise", 18, Kind::I16),
    (b"hhea", "caretSlopeRun", 20, Kind::I16),
    (b"hhea", "caretOffset", 22, Kind::I16),
    (b"vhea", "vertTypoAscender", 4, Kind::I16),
    (b"vhea", "vertTypoDescender", 6, Kind::I16),
    (b"vhea", "vertTypoLineGap", 8, Kind::I16),
    (b"vhea", "advanceHeightMax", 10, Kind::U16),
    (b"vhea", "minTopSideBearing", 12, Kind::I16),
    (b"vhea", "minBottomSideBearing", 14, Kind::I16),
    (b"vhea", "yMaxExtent", 16, Kind::I16),
    (b"vhea", "caretSlopeRise", 18, Kind::I16),
    (b"vhea", "caretSlopeRun", 20, Kind::I16),
    (b"vhea", "caretOffset", 22, Kind::I16),
    (b"OS/2", "xAvgCharWidth", 2, Kind::I16),
    (b"OS/2", "usWeightClass", 4, Kind::U16),
    (b"OS/2", "usWidthClass", 6, Kind::U16),
    (b"OS/2", "fsType", 8, Kind::U16),
    (b"OS/2", "ySubscriptXSize", 10, Kind::I16),
    (b"OS/2", "ySubscriptYSize", 12, Kind::I16),
    (b"OS/2", "ySubscriptXOffset", 14, Kind::I16),
    (b"OS/2", "ySubscriptYOffset", 16, Kind::I16),
    (b"OS/2", "ySuperscriptXSize", 18, Kind::I16),
    (b"OS/2", "ySuperscriptYSize", 20, Kind::I16),
    (b"OS/2", "ySuperscriptXOffset", 22, Kind::I16),
    (b"OS/2", "ySuperscriptYOffset", 24, Kind::I16),
    (b"OS/2", "yStrikeoutSize", 26, Kind::I16),
    (b"OS/2", "yStrikeoutPosition", 28, Kind::I16),
    (b"OS/2", "sFamilyClass", 30, Kind::I16),
    (b"OS/2", "panose", 32, Kind::Panose),
    (b"OS/2", "ulUnicodeRange1", 42, Kind::U32),
    (b"OS/2", "ulUnicodeRange2", 46, Kind::U32),
    (b"OS/2", "ulUnicodeRange3", 50, Kind::U32),
    (b"OS/2", "ulUnicodeRange4", 54, Kind::U32),
    (b"OS/2", "achVendID", 58, Kind::Tag),
    (b"OS/2", "fsSelection", 62, Kind::U16),
    (b"OS/2", "usFirstCharIndex", 64, Kind::U16),
    (b"OS/2", "usLastCharIndex", 66, Kind::U16),
    (b"OS/2", "sTypoAscender", 68, Kind::I16),
    (b"OS/2", "sTypoDescender", 70, Kind::I16),
    (b"OS/2", "sTypoLineGap", 72, Kind::I16),
    (b"OS/2", "usWinAscent", 74, Kind::U16),
    (b"OS/2", "usWinDescent", 76, Kind::U16),
    (b"OS/2", "ulCodePageRange1", 78, Kind::U32),
    (b"OS/2", "ulCodePageRange2", 82, Kind::U32),
    (b"OS/2", "sxHeight", 86, Kind::I16),
    (b"OS/2", "sCapHeight", 88, Kind::I16),
    (b"OS/2", "usDefaultChar", 90, Kind::U16),
    (b"OS/2", "usBreakChar", 92, Kind::U16),
    (b"OS/2", "usMaxContext", 94, Kind::U16),
    (b"OS/2", "usLowerOpticalPointSize", 96, Kind::U16),
    (b"OS/2", "usUpperOpticalPointSize", 98, Kind::U16),
    (b"post", "italicAngle", 4, Kind::Fixed),
    (b"post", "underlinePosition", 8, Kind::I16),
    (b"post", "underlineThickness", 10, Kind::I16),
    (b"post", "isFixedPitch", 12, Kind::U32),
    (b"maxp", "maxPoints", 6, Kind::U16),
    (b"maxp", "maxContours", 8, Kind::U16),
    (b"maxp", "maxCompositePoints", 10, Kind::U16),
    (b"maxp", "maxCompositeContours", 12, Kind::U16),
    (b"maxp", "maxStackElements", 24, Kind::U16),
    (b"maxp", "maxComponentElements", 28, Kind::U16),
    (b"maxp", "maxComponentDepth", 30, Kind::U16),
];

fn find_field(table: Tag, name: &str) -> Result<(usize, Kind)> {
    FIELDS
        .iter()
        .find(|(t, n, _, _)| **t == table && *n == name)
        .map(|(_, _, at, kind)| (*at, *kind))
        .ok_or_else(|| TablesError::BadOptions(format!("the field `{name}` of '{}' cannot be patched", tag_str(table).trim_end())))
}

/// The names of the fields of `table` that [`patch_table`] can write.
pub fn patchable_fields(table: Tag) -> Vec<&'static str> {
    FIELDS.iter().filter(|(t, ..)| **t == table).map(|(_, n, ..)| *n).collect()
}

fn missing(table: Tag) -> TablesError {
    TablesError::MissingTable(tag_str(table).trim_end().to_string())
}

fn short(table: Tag, name: &str) -> TablesError {
    TablesError::invalid(format!("the '{}' table is too short to hold `{name}`", tag_str(table).trim_end()))
}

pub(crate) fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*b.first()?, *b.get(1)?]))
}

pub(crate) fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
}

/// Writes `value` at `at`; `None` when it does not fit.
pub(crate) fn put(bytes: &mut [u8], at: usize, value: &[u8]) -> Option<()> {
    bytes.get_mut(at..at.checked_add(value.len())?)?.copy_from_slice(value);
    Some(())
}

/// The current value of one field, in the form [`patch_table`] takes.
pub fn read_field(font: &SfntFont, table: Tag, name: &str) -> Result<PatchValue> {
    let (at, kind) = find_field(table, name)?;
    let data = font.get(table).ok_or_else(|| missing(table))?;
    let bytes = data.get(at..at.saturating_add(kind.size())).ok_or_else(|| short(table, name))?;
    let n = |b: &[u8]| -> u32 { b.iter().fold(0u32, |acc, x| (acc << 8) | u32::from(*x)) };
    Ok(match kind {
        Kind::U16 => PatchValue::Number(f64::from(n(bytes))),
        Kind::I16 => PatchValue::Number(f64::from(n(bytes) as u16 as i16)),
        Kind::U32 => PatchValue::Number(f64::from(n(bytes))),
        Kind::Fixed => PatchValue::Number(f64::from(n(bytes) as i32) / 65536.0),
        Kind::Tag => PatchValue::Text(String::from_utf8_lossy(bytes).into_owned()),
        Kind::Panose => PatchValue::Text(bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(" ")),
    })
}

fn integer(name: &str, v: f64, min: f64, max: f64) -> Result<i64> {
    if !v.is_finite() || v.fract() != 0.0 || v < min || v > max {
        return Err(TablesError::BadOptions(format!("`{name}` must be a whole number from {min} to {max}, not {v}")));
    }
    Ok(v as i64)
}

fn encode(name: &str, kind: Kind, value: &PatchValue) -> Result<Vec<u8>> {
    let number = || match value {
        PatchValue::Number(v) => Ok(*v),
        PatchValue::Text(_) => Err(TablesError::BadOptions(format!("`{name}` takes a number"))),
    };
    Ok(match kind {
        Kind::U16 => (integer(name, number()?, 0.0, 65_535.0)? as u16).to_be_bytes().to_vec(),
        Kind::I16 => (integer(name, number()?, -32_768.0, 32_767.0)? as i16).to_be_bytes().to_vec(),
        Kind::U32 => (integer(name, number()?, 0.0, f64::from(u32::MAX))? as u32).to_be_bytes().to_vec(),
        Kind::Fixed => {
            let v = number()?;
            let raw = (v * 65536.0).round();
            if !raw.is_finite() || raw < f64::from(i32::MIN) || raw > f64::from(i32::MAX) {
                return Err(TablesError::BadOptions(format!("`{name}` is out of range for a 16.16 number: {v}")));
            }
            (raw as i32).to_be_bytes().to_vec()
        }
        Kind::Tag => {
            let PatchValue::Text(t) = value else { return Err(TablesError::BadOptions(format!("`{name}` takes text"))) };
            if t.len() > 4 || !t.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
                return Err(TablesError::BadOptions(format!("`{name}` takes one to four printable ASCII characters")));
            }
            let mut out = t.as_bytes().to_vec();
            out.resize(4, b' ');
            out
        }
        Kind::Panose => {
            let PatchValue::Text(t) = value else { return Err(TablesError::BadOptions(format!("`{name}` takes ten numbers as text"))) };
            let parts: Vec<&str> = t.split([' ', ',']).filter(|s| !s.is_empty()).collect();
            if parts.len() != 10 {
                return Err(TablesError::BadOptions(format!("`{name}` takes ten numbers, not {}", parts.len())));
            }
            let mut out = Vec::with_capacity(10);
            for p in parts {
                out.push(p.parse::<u8>().map_err(|_| TablesError::BadOptions(format!("`{name}`: `{p}` is not a number from 0 to 255")))?);
            }
            out
        }
    })
}

/// The font with the fields changed; the table is rewritten, the others are kept byte for byte.
pub fn patch_table(font: &SfntFont, options: &PatchOptions) -> Result<SfntFont> {
    let table = options.table;
    let mut data = font.get(table).ok_or_else(|| missing(table))?.to_vec();
    for (name, value) in &options.fields {
        let (at, kind) = find_field(table, name)?;
        let bytes = encode(name, kind, value)?;
        put(&mut data, at, &bytes).ok_or_else(|| short(table, name))?;
    }
    let mut out = font.clone();
    out.insert(table, data);
    Ok(out)
}

/// The values the fields of a newer `OS/2` version get when [`raise_os2_version`] adds them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Os2Upgrade {
    pub code_page_range: [u32; 2],
    pub x_height: i16,
    pub cap_height: i16,
    pub default_char: u16,
    pub break_char: u16,
    pub max_context: u16,
}

/// The length of an `OS/2` table of each version.
fn os2_length(version: u16) -> usize {
    match version {
        0 => 78,
        1 => 86,
        2..=4 => 96,
        _ => 100,
    }
}

/// The font with its `OS/2` table raised to `version` (at most 5), the fields the older version
/// lacks filled from `fill`. A table already at that version or later is left as it is.
pub fn raise_os2_version(font: &SfntFont, version: u16, fill: &Os2Upgrade) -> Result<SfntFont> {
    const OS2: Tag = *b"OS/2";
    if version > 5 {
        return Err(TablesError::BadOptions(format!("OS/2 version {version} does not exist; the latest is 5")));
    }
    let data = font.get(OS2).ok_or_else(|| missing(OS2))?;
    let current = be_u16(data, 0).ok_or_else(|| short(OS2, "version"))?;
    if current >= version {
        return Ok(font.clone());
    }
    let have = os2_length(current);
    let mut out = data.get(..have).ok_or_else(|| TablesError::invalid(format!("the OS/2 table of version {current} is too short")))?.to_vec();
    if current < 1 {
        let [low, high] = fill.code_page_range;
        out.extend(low.to_be_bytes());
        out.extend(high.to_be_bytes());
    }
    if current < 2 && version >= 2 {
        out.extend(fill.x_height.to_be_bytes());
        out.extend(fill.cap_height.to_be_bytes());
        out.extend(fill.default_char.to_be_bytes());
        out.extend(fill.break_char.to_be_bytes());
        out.extend(fill.max_context.to_be_bytes());
    }
    if version >= 5 {
        out.extend(0u16.to_be_bytes());
        out.extend(0xFFFFu16.to_be_bytes());
    }
    put(&mut out, 0, &version.to_be_bytes()).ok_or_else(|| short(OS2, "version"))?;
    let mut font = font.clone();
    font.insert(OS2, out);
    Ok(font)
}

/// The byte ranges of each glyph in `glyf`, from `loca`.
pub(crate) fn glyph_ranges(font: &SfntFont) -> Result<Vec<(usize, usize)>> {
    let head = font.get(*b"head").ok_or_else(|| missing(*b"head"))?;
    let maxp = font.get(*b"maxp").ok_or_else(|| missing(*b"maxp"))?;
    let loca = font.get(*b"loca").ok_or_else(|| missing(*b"loca"))?;
    let long = be_u16(head, 50).ok_or_else(|| short(*b"head", "indexToLocFormat"))? != 0;
    let glyphs = usize::from(be_u16(maxp, 4).ok_or_else(|| short(*b"maxp", "numGlyphs"))?);
    let offset = |i: usize| -> Option<usize> {
        if long {
            be_u32(loca, i.checked_mul(4)?).and_then(|v| usize::try_from(v).ok())
        } else {
            be_u16(loca, i.checked_mul(2)?).map(|v| usize::from(v) * 2)
        }
    };
    let mut out = Vec::with_capacity(glyphs);
    for gid in 0..glyphs {
        let (Some(start), Some(end)) = (offset(gid), offset(gid.saturating_add(1))) else {
            return Err(TablesError::invalid("the loca table is shorter than maxp.numGlyphs says"));
        };
        out.push((start, end.max(start)));
    }
    Ok(out)
}

/// The font with the stored bounding box of each listed glyph replaced. Glyphs without an
/// outline (no bytes in `glyf`) are left out; a glyph id past the end is refused.
pub fn set_glyph_bounds(font: &SfntFont, bounds: &[(u16, [i16; 4])]) -> Result<SfntFont> {
    const GLYF: Tag = *b"glyf";
    let ranges = glyph_ranges(font)?;
    let mut glyf = font.get(GLYF).ok_or_else(|| missing(GLYF))?.to_vec();
    for (gid, b) in bounds {
        let Some(&(start, end)) = ranges.get(usize::from(*gid)) else {
            return Err(TablesError::BadOptions(format!("the font has no glyph {gid}")));
        };
        if end.saturating_sub(start) < 10 {
            continue;
        }
        let mut header = Vec::with_capacity(8);
        for v in b {
            header.extend(v.to_be_bytes());
        }
        put(&mut glyf, start.saturating_add(2), &header).ok_or_else(|| TablesError::invalid(format!("glyph {gid} lies past the end of glyf")))?;
    }
    let mut out = font.clone();
    out.insert(GLYF, glyf);
    Ok(out)
}

/// A `gasp` table (version 1) with these `(rangeMaxPPEM, rangeGaspBehavior)` ranges.
pub fn gasp_table(ranges: &[(u16, u16)]) -> Result<Vec<u8>> {
    let count = u16::try_from(ranges.len()).map_err(|_| TablesError::TooLarge { what: "the number of gasp ranges", limit: 65_535 })?;
    let mut out = Vec::with_capacity(4 + 4 * ranges.len());
    out.extend(1u16.to_be_bytes());
    out.extend(count.to_be_bytes());
    for (ppem, behaviour) in ranges {
        out.extend(ppem.to_be_bytes());
        out.extend(behaviour.to_be_bytes());
    }
    Ok(out)
}

/// Whether `needle` occurs in `hay`, in time linear in both (Knuth, Morris and Pratt).
fn contains_bytes(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    // fail[i]: the length of the longest proper prefix of needle[..=i] that also ends it.
    let mut fail = vec![0usize; needle.len()];
    let mut k = 0usize;
    for (i, &c) in needle.iter().enumerate().skip(1) {
        while k > 0 && needle.get(k) != Some(&c) {
            k = fail.get(k - 1).copied().unwrap_or(0);
        }
        if needle.get(k) == Some(&c) {
            k += 1;
        }
        if let Some(slot) = fail.get_mut(i) {
            *slot = k;
        }
    }
    let mut k = 0usize;
    for &c in hay {
        while k > 0 && needle.get(k) != Some(&c) {
            k = fail.get(k - 1).copied().unwrap_or(0);
        }
        if needle.get(k) == Some(&c) {
            k += 1;
        }
        if k == needle.len() {
            return true;
        }
    }
    false
}

/// The font with `code` appended to its `prep` program (a new `prep` when it has none) and
/// `maxp.maxStackElements` raised to `stack` when it is lower. `false` and the font unchanged
/// when the program already holds `code`.
pub fn append_prep(font: &SfntFont, code: &[u8], stack: u16) -> Result<(SfntFont, bool)> {
    const PREP: Tag = *b"prep";
    let mut prep = font.get(PREP).map(<[u8]>::to_vec).unwrap_or_default();
    if !code.is_empty() && contains_bytes(&prep, code) {
        return Ok((font.clone(), false));
    }
    if prep.len().saturating_add(code.len()) > usize::from(u16::MAX) * 4 {
        return Err(TablesError::TooLarge { what: "the prep program", limit: usize::from(u16::MAX) * 4 });
    }
    prep.extend_from_slice(code);
    let mut out = font.clone();
    out.insert(PREP, prep);
    if let Some(maxp) = font.get(*b"maxp")
        && be_u32(maxp, 0) == Some(0x0001_0000)
        && be_u16(maxp, 24).is_some_and(|v| v < stack)
    {
        let mut maxp = maxp.to_vec();
        put(&mut maxp, 24, &stack.to_be_bytes()).ok_or_else(|| short(*b"maxp", "maxStackElements"))?;
        out.insert(*b"maxp", maxp);
    }
    Ok((out, true))
}
