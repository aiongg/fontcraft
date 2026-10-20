//! Removing hints (M12.27; rows `dehint`, `hints.remove`). Oracles: `tx -n`, pyftsubset
//! `--no-hinting`. CFF: stems, masks and flex; TrueType: glyph instructions, `fpgm`, `prep`,
//! `cvt `, `hdmx`, `LTSH`, `VDMX`.
//!
//! What a full run does (no glyph selection):
//!
//! | Font | Change |
//! |---|---|
//! | TrueType | instructions of every glyph removed (`glyf`, `loca`); the hinting tables deleted; `LTSH` deleted; the `maxp` hinting limits reset as pyftsubset resets them; `head` flags 2 and 4 cleared |
//! | CFF, CFF2 | stems, hint masks, counter masks and (unless kept) flex removed from every charstring, which is written without subroutines; the Private DICT hint values (zones, stems, `BlueScale` …) deleted as pyftsubset deletes them |
//! | both | `GPOS` device tables (not variation indexes) and contour-point anchors removed; `gasp` kept, replaced or deleted as asked |
//!
//! With a glyph selection only those glyphs lose their hints and every font-wide table stays.
//! `keepFontHints` keeps `fpgm`, `prep`, `cvt `, `cvar`, the `maxp` limits, the `head` flags and
//! the CFF Private DICT values.
//!
//! Tables the run does not change are copied byte for byte; a table whose content would not
//! change is not rewritten.
//!
//! The pruning rules follow fontTools' subset module (`--no-hinting`; MIT License, see NOTICE).

pub(crate) mod cff;
mod gpos;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// The font-wide TrueType hinting tables deleted by default (pyftsubset's default list).
pub const DEFAULT_HINTING_TABLES: [Tag; 6] = [*b"cvt ", *b"cvar", *b"fpgm", *b"prep", *b"hdmx", *b"VDMX"];

/// The tables `keepFontHints` keeps even when they are in the hinting-table list.
const FONT_HINT_TABLES: [Tag; 4] = [*b"cvt ", *b"cvar", *b"fpgm", *b"prep"];

/// `head.flags` bits that only mean something for hinted glyphs: 2 (instructions may depend on
/// point size) and 4 (instructions may alter advance widths).
const HEAD_INSTRUCTION_FLAGS: u16 = 0x0004 | 0x0010;

/// The `gasp` behaviour written by [`GaspMode::Smooth`]: grid-fitting, grey-scale, symmetric
/// grid-fitting and symmetric smoothing, the value the font-quality checks expect on the last
/// range of a font.
pub const GASP_SMOOTH_FLAGS: u16 = 0x000F;

/// What happens to `gasp`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GaspMode {
    /// Leave it as it is.
    #[default]
    Keep,
    /// Replace it with one range covering every size ([`GASP_SMOOTH_FLAGS`]).
    Smooth,
    /// Delete it.
    Remove,
}

impl GaspMode {
    /// The mode named `keep`, `smooth` or `remove`.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "keep" => Ok(GaspMode::Keep),
            "smooth" => Ok(GaspMode::Smooth),
            "remove" => Ok(GaspMode::Remove),
            other => Err(TablesError::BadOptions(format!("`{other}` is not a gasp mode: give keep, smooth or remove"))),
        }
    }
}

/// What [`dehint`] removes and keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DehintOptions {
    /// Glyph ids to remove hints from; `None` is every glyph. With a selection, font-wide data
    /// is kept.
    pub glyphs: Option<BTreeSet<u16>>,
    /// Keep font-wide hinting data: the CFF Private DICT zones and stems, or the TrueType
    /// `fpgm`, `prep`, `cvt `, `cvar` and the `maxp` limits.
    pub keep_font_hints: bool,
    /// Font-wide tables deleted ([`DEFAULT_HINTING_TABLES`] by default).
    pub hinting_tables: Vec<Tag>,
    /// Keep flex in CFF charstrings; otherwise each flex becomes its two curves.
    pub keep_flex: bool,
    pub gasp: GaspMode,
}

impl Default for DehintOptions {
    fn default() -> Self {
        Self { glyphs: None, keep_font_hints: false, hinting_tables: DEFAULT_HINTING_TABLES.to_vec(), keep_flex: false, gasp: GaspMode::Keep }
    }
}

impl DehintOptions {
    fn selected(&self, gid: usize) -> bool {
        match &self.glyphs {
            None => true,
            Some(set) => u16::try_from(gid).is_ok_and(|g| set.contains(&g)),
        }
    }

    /// Font-wide data changes only on a run over every glyph.
    fn font_wide(&self) -> bool {
        self.glyphs.is_none()
    }
}

/// What a run changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DehintReport {
    /// Glyphs whose program or charstring lost hints.
    pub glyphs_changed: usize,
    /// Tables deleted, in tag order.
    pub tables_removed: Vec<Tag>,
    /// Tables rewritten, in tag order.
    pub tables_changed: Vec<Tag>,
}

/// The font without hints.
pub fn dehint(font: &SfntFont, options: &DehintOptions) -> Result<SfntFont> {
    dehint_with_report(font, options).map(|(font, _)| font)
}

/// The font without hints, with what changed.
pub fn dehint_with_report(font: &SfntFont, options: &DehintOptions) -> Result<(SfntFont, DehintReport)> {
    let mut out = font.clone();
    let mut report = DehintReport::default();
    let mut changed: BTreeSet<Tag> = BTreeSet::new();

    if let Some(glyf) = font.get(*b"glyf")
        && let Some(stripped) = strip_glyf(font, glyf, options)?
    {
        report.glyphs_changed += stripped.glyphs_changed;
        out.insert(*b"glyf", stripped.glyf);
        out.insert(*b"loca", stripped.loca);
        changed.insert(*b"glyf");
        changed.insert(*b"loca");
        if let Some(head) = stripped.head {
            out.insert(*b"head", head);
            changed.insert(*b"head");
        }
    }
    for tag in [*b"CFF ", *b"CFF2"] {
        if let Some(data) = font.get(tag) {
            let cff2 = tag == *b"CFF2";
            if let Some((bytes, glyphs)) = cff::dehint(data, cff2, options)? {
                report.glyphs_changed += glyphs;
                out.insert(tag, bytes);
                changed.insert(tag);
            }
        }
    }

    if options.font_wide() {
        let truetype = font.tables.contains_key(b"glyf");
        let mut drop: Vec<Tag> = options.hinting_tables.clone();
        if truetype {
            drop.push(*b"LTSH");
        }
        if options.keep_font_hints {
            drop.retain(|t| !FONT_HINT_TABLES.contains(t));
        }
        for tag in drop {
            if out.remove(tag).is_some() && !report.tables_removed.contains(&tag) {
                report.tables_removed.push(tag);
            }
        }
        if truetype && !options.keep_font_hints {
            if let Some(maxp) = out.get(*b"maxp").and_then(reset_maxp) {
                out.insert(*b"maxp", maxp);
                changed.insert(*b"maxp");
            }
            if let Some(head) = out.get(*b"head").and_then(clear_head_flags) {
                out.insert(*b"head", head);
                changed.insert(*b"head");
            }
        }
        if let Some(data) = font.get(*b"GPOS")
            && let Some(bytes) = gpos::prune(data)?
        {
            out.insert(*b"GPOS", bytes);
            changed.insert(*b"GPOS");
        }
    }

    match options.gasp {
        GaspMode::Keep => {}
        GaspMode::Remove => {
            if out.remove(*b"gasp").is_some() {
                report.tables_removed.push(*b"gasp");
            }
        }
        GaspMode::Smooth => {
            let table = smooth_gasp();
            if out.get(*b"gasp") != Some(table.as_slice()) {
                out.insert(*b"gasp", table);
                changed.insert(*b"gasp");
            }
        }
    }

    report.tables_removed.sort();
    changed.retain(|t| out.tables.contains_key(t));
    report.tables_changed = changed.into_iter().collect();
    Ok((out, report))
}

/// `gasp` version 1 with one range up to 0xFFFF ppem.
fn smooth_gasp() -> Vec<u8> {
    let mut t = Vec::with_capacity(8);
    t.extend_from_slice(&1u16.to_be_bytes());
    t.extend_from_slice(&1u16.to_be_bytes());
    t.extend_from_slice(&0xFFFFu16.to_be_bytes());
    t.extend_from_slice(&GASP_SMOOTH_FLAGS.to_be_bytes());
    t
}

/// `maxp` version 1.0 with the hinting limits pyftsubset resets: `maxZones` 1, and twilight
/// points, storage, function and instruction definitions, stack elements and instruction size 0.
/// `None` when the table is version 0.5 or already reset.
fn reset_maxp(maxp: &[u8]) -> Option<Vec<u8>> {
    if read_u32(maxp, 0)? != 0x0001_0000 || maxp.len() < 28 {
        return None;
    }
    let mut out = maxp.to_vec();
    put_u16(&mut out, 14, 1)?;
    for at in (16..28).step_by(2) {
        put_u16(&mut out, at, 0)?;
    }
    (out != maxp).then_some(out)
}

/// `head` without the instruction flags; `None` when they are already clear.
fn clear_head_flags(head: &[u8]) -> Option<Vec<u8>> {
    let flags = read_u16(head, 16)?;
    if flags & HEAD_INSTRUCTION_FLAGS == 0 {
        return None;
    }
    let mut out = head.to_vec();
    put_u16(&mut out, 16, flags & !HEAD_INSTRUCTION_FLAGS)?;
    Some(out)
}

struct StrippedGlyf {
    glyf: Vec<u8>,
    loca: Vec<u8>,
    /// `head` with `indexToLocFormat` switched to long offsets, when the short ones overflow.
    head: Option<Vec<u8>>,
    glyphs_changed: usize,
}

/// `glyf` and `loca` with the instructions of the selected glyphs removed; `None` when no
/// selected glyph has any.
fn strip_glyf(font: &SfntFont, glyf: &[u8], options: &DehintOptions) -> Result<Option<StrippedGlyf>> {
    let head = font.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?;
    let maxp = font.get(*b"maxp").ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    let loca = font.get(*b"loca").ok_or_else(|| TablesError::MissingTable("loca".into()))?;
    let long = match read_u16(head, 50) {
        Some(0) => false,
        Some(1) => true,
        _ => return Err(TablesError::invalid("head.indexToLocFormat is neither 0 nor 1")),
    };
    let count = usize::from(read_u16(maxp, 4).ok_or_else(|| TablesError::invalid("the maxp table is cut short"))?);
    let offsets = read_loca(loca, long, count)?;

    let mut pieces: Vec<std::borrow::Cow<'_, [u8]>> = Vec::with_capacity(count);
    let mut glyphs_changed = 0usize;
    for gid in 0..count {
        let (Some(&start), Some(&end)) = (offsets.get(gid), offsets.get(gid + 1)) else {
            return Err(TablesError::invalid("the loca table is cut short"));
        };
        let data = glyf.get(start..end).ok_or_else(|| TablesError::invalid(format!("glyph {gid} lies outside the glyf table")))?;
        let piece = if options.selected(gid) { strip_glyph(data, gid)? } else { None };
        match piece {
            Some(stripped) => {
                glyphs_changed += 1;
                pieces.push(std::borrow::Cow::Owned(stripped));
            }
            None => pieces.push(std::borrow::Cow::Borrowed(data)),
        }
    }
    if glyphs_changed == 0 {
        return Ok(None);
    }

    let align = if long { 4 } else { 2 };
    let mut out = Vec::with_capacity(glyf.len());
    let mut new_offsets = Vec::with_capacity(count + 1);
    for piece in &pieces {
        new_offsets.push(out.len());
        out.extend_from_slice(piece);
        while out.len() % align != 0 {
            out.push(0);
        }
    }
    new_offsets.push(out.len());

    let fits_short = new_offsets.iter().all(|o| o % 2 == 0 && o / 2 <= usize::from(u16::MAX));
    let use_long = long || !fits_short;
    let mut loca_out = Vec::with_capacity(new_offsets.len() * if use_long { 4 } else { 2 });
    for o in &new_offsets {
        if use_long {
            let v = u32::try_from(*o).map_err(|_| TablesError::TooLarge { what: "the glyf table", limit: u32::MAX as usize })?;
            loca_out.extend_from_slice(&v.to_be_bytes());
        } else {
            let v = u16::try_from(o / 2).map_err(|_| TablesError::invalid("a short loca offset overflows"))?;
            loca_out.extend_from_slice(&v.to_be_bytes());
        }
    }
    let head_out = if use_long && !long {
        let mut h = head.to_vec();
        put_u16(&mut h, 50, 1).ok_or_else(|| TablesError::invalid("the head table is cut short"))?;
        Some(h)
    } else {
        None
    };
    Ok(Some(StrippedGlyf { glyf: out, loca: loca_out, head: head_out, glyphs_changed }))
}

/// The `count + 1` byte offsets of `loca`.
fn read_loca(loca: &[u8], long: bool, count: usize) -> Result<Vec<usize>> {
    let entries = count.checked_add(1).ok_or_else(|| TablesError::invalid("the glyph count overflows"))?;
    let mut offsets = Vec::with_capacity(entries);
    for i in 0..entries {
        let value = if long { read_u32(loca, i * 4).map(|v| v as usize) } else { read_u16(loca, i * 2).map(|v| usize::from(v) * 2) };
        let value = value.ok_or_else(|| TablesError::invalid("the loca table is shorter than maxp.numGlyphs says"))?;
        if offsets.last().is_some_and(|prev| value < *prev) {
            return Err(TablesError::invalid(format!("loca offset {i} goes backwards")));
        }
        offsets.push(value);
    }
    Ok(offsets)
}

/// Composite glyph flags (OpenType `glyf`).
const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;

/// One glyph's data without its instructions; `None` when it has none.
fn strip_glyph(data: &[u8], gid: usize) -> Result<Option<Vec<u8>>> {
    if data.is_empty() {
        return Ok(None);
    }
    let cut = || TablesError::invalid(format!("glyph {gid} is cut short"));
    let contours = read_u16(data, 0).ok_or_else(cut)? as i16;
    if data.len() < 10 {
        return Err(cut());
    }
    if contours >= 0 {
        let contours = contours as usize;
        let at = 10 + 2 * contours;
        let length = usize::from(read_u16(data, at).ok_or_else(cut)?);
        if length == 0 {
            return Ok(None);
        }
        let points = match contours.checked_sub(1) {
            Some(last) => usize::from(read_u16(data, 10 + 2 * last).ok_or_else(cut)?) + 1,
            None => 0,
        };
        let flags_at = at.checked_add(2 + length).filter(|end| *end <= data.len()).ok_or_else(cut)?;
        // Bytes after the coordinates are the input's padding; the table adds its own.
        let end = simple_data_end(data, flags_at, points).ok_or_else(cut)?;
        let mut out = Vec::with_capacity(end);
        out.extend_from_slice(data.get(..at).ok_or_else(cut)?);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(data.get(flags_at..end).ok_or_else(cut)?);
        return Ok(Some(out));
    }
    // Composite: walk the components; the program follows the last one.
    let mut at = 10usize;
    let mut has_program = false;
    let mut flag_positions = Vec::new();
    loop {
        let flags = read_u16(data, at).ok_or_else(cut)?;
        flag_positions.push(at);
        has_program |= flags & WE_HAVE_INSTRUCTIONS != 0;
        let mut size = 4 + if flags & ARG_1_AND_2_ARE_WORDS != 0 { 4 } else { 2 };
        if flags & WE_HAVE_A_SCALE != 0 {
            size += 2;
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            size += 4;
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            size += 8;
        }
        at = at.checked_add(size).filter(|end| *end <= data.len()).ok_or_else(cut)?;
        if flags & MORE_COMPONENTS == 0 {
            break;
        }
    }
    if !has_program {
        return Ok(None);
    }
    let length = usize::from(read_u16(data, at).ok_or_else(cut)?);
    if at + 2 + length > data.len() {
        return Err(cut());
    }
    // The output ends after the last component, which drops the program and any padding.
    let mut out = data.get(..at).ok_or_else(cut)?.to_vec();
    for pos in flag_positions {
        let flags = read_u16(&out, pos).ok_or_else(cut)?;
        put_u16(&mut out, pos, flags & !WE_HAVE_INSTRUCTIONS).ok_or_else(cut)?;
    }
    Ok(Some(out))
}

/// Simple glyph flag bits (OpenType `glyf`).
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT_FLAG: u8 = 0x08;
const X_SAME_OR_POSITIVE: u8 = 0x10;
const Y_SAME_OR_POSITIVE: u8 = 0x20;

/// Where a simple glyph's coordinates end, given where its flags start and its point count
/// (at most 65,536: the contour end points are 16-bit). `None` when the flags or coordinates
/// run past the data, or a repeated flag covers more points than the glyph has.
fn simple_data_end(data: &[u8], flags_at: usize, points: usize) -> Option<usize> {
    let mut at = flags_at;
    let mut seen = 0usize;
    let mut coordinate_bytes = 0usize;
    while seen < points {
        let flag = *data.get(at)?;
        at += 1;
        let mut repeat = 1usize;
        if flag & REPEAT_FLAG != 0 {
            repeat += usize::from(*data.get(at)?);
            at += 1;
        }
        let x = if flag & X_SHORT != 0 {
            1
        } else if flag & X_SAME_OR_POSITIVE != 0 {
            0
        } else {
            2
        };
        let y = if flag & Y_SHORT != 0 {
            1
        } else if flag & Y_SAME_OR_POSITIVE != 0 {
            0
        } else {
            2
        };
        coordinate_bytes += (x + y) * repeat;
        seen += repeat;
    }
    let end = at.checked_add(coordinate_bytes)?;
    (seen == points && end <= data.len()).then_some(end)
}

fn read_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn put_u16(b: &mut [u8], at: usize, value: u16) -> Option<()> {
    b.get_mut(at..at.checked_add(2)?)?.copy_from_slice(&value.to_be_bytes());
    Some(())
}
