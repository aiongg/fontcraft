//! The `CFF ` table of a merged font, after fontTools' `merge/tables.py` (fontTools 4.66.1, MIT
//! License, Copyright 2013 Google, Inc.; see `NOTICE`).
//!
//! - Every font's subroutine calls are expanded first, so no charstring depends on another
//!   font's subroutines.
//! - The first font's Top DICT, Private DICT, Encoding and strings are kept; the charset lists the
//!   merged glyph names.
//! - A glyph from a font whose `defaultWidthX` or `nominalWidthX` differ from the first font's
//!   gets its width operand rewritten for the first font's pair, as fontTools does. The other
//!   fonts' Private DICT values (blue zones, stems) are not carried over.
//! - `FontBBox` is the union of the merged glyphs' bounds, as fontTools sets it when it saves.
//! - The table is subroutinised again. fontTools leaves it without subroutines; when the
//!   subroutiniser cannot finish, the table stays that way too, with a warning.
//!
//! A CID-keyed font merges only into a CID-keyed result ([`super::cid`]); fontTools refuses
//! them.
//!
//! Options of AFDKO mergefonts: the map header's PostScript name and language group set the
//! Name INDEX and the Private DICT `LanguageGroup`, and a hint font's hint values (blue zones,
//! stems, `ForceBold`, `LanguageGroup`, `ExpansionFactor`) and global metrics (underline position
//! and thickness) replace the first font's.

use std::collections::HashMap;

use fontcraft_cff::subr::{SubrOptions, desubroutinize_table, subroutinize_table};

use super::MapHeader;
use super::plan::Plan;
use crate::reorder::cff::{self as cff, Cff};
use crate::reorder::names::num_glyphs;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Top DICT `FontBBox`.
const OP_FONT_BBOX: u16 = 5;
/// Private DICT `LanguageGroup`.
const OP_LANGUAGE_GROUP: u16 = 1217;
/// The Top DICT global metrics a hint font supplies: `UnderlinePosition`, `UnderlineThickness`.
const METRIC_OPS: [u16; 2] = [1203, 1204];
/// Operands a `CFF ` charstring may hold on its stack, plus the width.
const MAX_ARGS: usize = 49;

/// One font's `CFF ` table with every subroutine call expanded. A CID-keyed table is refused
/// unless `cid_result`.
pub(super) fn expand(font: &SfntFont, index: usize, cid_result: bool) -> Result<Vec<u8>> {
    let data = font.get(*b"CFF ").ok_or_else(|| TablesError::MissingTable("CFF ".into()))?;
    let parsed = cff::parse(data, num_glyphs(font)?)?;
    if parsed.is_cid() && !cid_result {
        return Err(TablesError::Unsupported(format!(
            "merging a CID-keyed CFF font (font {}) into a name-keyed one; give a cidfontinfo file or numeric map names for a CID-keyed result",
            index + 1
        )));
    }
    desubroutinize_table(data, false).map_err(|e| TablesError::invalid(format!("the 'CFF ' table of font {} cannot be expanded: {e}", index + 1)))
}

/// An expanded table ([`expand`]), parsed.
pub(super) struct Expanded<'a> {
    pub cff: Cff<'a>,
    /// `defaultWidthX` and `nominalWidthX` of glyph 0's Private DICT; `None` when they are not
    /// whole numbers.
    widths: Option<(f64, f64)>,
    index: usize,
}

impl<'a> Expanded<'a> {
    pub fn read(font: &SfntFont, expanded: &'a [u8], index: usize) -> Result<Self> {
        let cff = cff::parse(expanded, num_glyphs(font)?)?;
        let widths = match cff.widths(0) {
            (Some(default), Some(nominal)) => Some((default as f64, nominal as f64)),
            _ => None,
        };
        Ok(Expanded { cff, widths, index })
    }

    /// The widths a name-keyed merge rewrites charstrings with.
    fn widths(&self) -> Result<(f64, f64)> {
        self.widths.ok_or_else(|| {
            TablesError::Unsupported(format!("merging a CFF font whose Private DICT widths are not whole numbers (font {})", self.index + 1))
        })
    }

    /// Every charstring, in glyph order.
    pub fn charstrings(&self) -> Vec<Vec<u8>> {
        let n = self.cff.charstring_lengths().len();
        (0..n).map(|g| self.cff.charstring(g).unwrap_or_default().to_vec()).collect()
    }
}

/// The merged `CFF ` table.
pub(super) struct Built {
    pub table: Vec<u8>,
    /// Each glyph's bounds rounded outwards (xMin, yMin, xMax, yMax); `None` for a glyph that
    /// draws nothing.
    pub bounds: Vec<Option<[i16; 4]>>,
}

/// The name-keyed `CFF ` table of the glyphs of `plan` named `names`, from the expanded tables
/// of the fonts. `header` (the first font's map header) renames the font and sets its
/// `LanguageGroup`; `hints` supplies the hint values and global metrics.
pub(super) fn build(
    fonts: &[Expanded<'_>],
    plan: &Plan,
    names: &[String],
    header: &MapHeader,
    hints: Option<&Cff<'_>>,
    warnings: &mut Vec<String>,
) -> Result<Built> {
    let first = fonts.first().ok_or_else(|| TablesError::BadOptions("give fonts to merge".into()))?;
    let first_widths = first.widths()?;
    let mut replace: HashMap<usize, Vec<u8>> = HashMap::with_capacity(plan.slots.len());
    let mut new_to_old: Vec<u16> = Vec::with_capacity(plan.slots.len());
    for (g, slot) in plan.slots.iter().enumerate() {
        let source = fonts.get(slot.font).ok_or_else(|| TablesError::invalid("a glyph comes from a font that was not given"))?;
        let cs = source.cff.charstring(usize::from(slot.gid)).ok_or_else(|| TablesError::invalid("a glyph has no charstring"))?;
        let widths = source.widths()?;
        let bytes = if widths == first_widths { cs.to_vec() } else { rewrite_width(cs, widths, first_widths)? };
        replace.insert(g, bytes);
        new_to_old.push(if slot.font == 0 { slot.gid } else { 0 });
    }
    let mut top = first.cff.clone();
    if let Some(name) = &header.ps_name {
        top.set_font_name(name)?;
    }
    if let Some(h) = hints {
        top.copy_hints_from(0, h, 0)?;
    }
    if let Some(group) = header.language_group {
        top.set_private_ints(0, OP_LANGUAGE_GROUP, &[i64::from(group)])?;
    }
    let table = top.rebuild_with(&new_to_old, Some(names), &replace, false)?;
    finish(&table, plan.slots.len(), hints, warnings)
}

/// A merged table finished: `FontBBox` set to the union of the glyphs' bounds, the global
/// metrics of `hints` taken, then subroutinised again.
pub(super) fn finish(table: &[u8], glyph_count: usize, hints: Option<&Cff<'_>>, warnings: &mut Vec<String>) -> Result<Built> {
    let glyph_bounds = crate::ttx::read_cff::glyph_bounds(table)?;
    let rounded: Vec<Option<[f64; 4]>> = glyph_bounds.iter().map(|b| b.map(|b| [b[0].floor(), b[1].floor(), b[2].ceil(), b[3].ceil()])).collect();
    let bbox = rounded.iter().flatten().fold(None, |acc: Option<[f64; 4]>, b| {
        Some(match acc {
            None => *b,
            Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
        })
    });
    let mut top = cff::parse(table, glyph_count)?;
    let bbox = bbox.unwrap_or_default();
    top.set_top_ints(OP_FONT_BBOX, &bbox.map(|v| v as i64))?;
    if let Some(h) = hints {
        top.copy_top_from(h, &METRIC_OPS);
    }
    let identity: Vec<u16> = (0..glyph_count)
        .map(|g| u16::try_from(g).map_err(|_| TablesError::TooLarge { what: "the merged glyph count", limit: usize::from(u16::MAX) }))
        .collect::<Result<_>>()?;
    let table = top.rebuild(&identity, None)?;
    let table = match subroutinize_table(&table, false, &SubrOptions::default()) {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("the merged 'CFF ' table is left without subroutines: {e}"));
            table
        }
    };
    let clamp = |v: f64| v.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
    let bounds = rounded.iter().map(|b| b.map(|b| b.map(clamp))).collect();
    Ok(Built { table, bounds })
}

/// The charstring `cs` of a font whose widths are `from`, with its width operand written for a
/// font whose widths are `to` (fontTools' rule: the width is read with `from`, then stored only
/// when it differs from the new `defaultWidthX`).
pub(super) fn rewrite_width(cs: &[u8], from: (f64, f64), to: (f64, f64)) -> Result<Vec<u8>> {
    let (width, rest) = match width_operand(cs)? {
        Some((value, len)) => (from.1 + value, cs.get(len..).unwrap_or_default()),
        None => (from.0, cs),
    };
    let mut out = Vec::with_capacity(cs.len() + 5);
    if width != to.0 {
        encode_number(width - to.1, &mut out)?;
    }
    out.extend_from_slice(rest);
    Ok(out)
}

/// The width operand of a charstring with its byte length: the extra first operand of its first
/// stack-clearing operator. `None` when it has none.
pub(super) fn width_operand(cs: &[u8]) -> Result<Option<(f64, usize)>> {
    let mut at = 0usize;
    let mut args: Vec<(f64, usize)> = Vec::new();
    let byte = |i: usize| cs.get(i).copied().ok_or_else(|| TablesError::invalid("a charstring is cut short"));
    while let Some(&b0) = cs.get(at) {
        let (value, len) = match b0 {
            28 => (f64::from(i16::from_be_bytes([byte(at + 1)?, byte(at + 2)?])), 3),
            32..=246 => (f64::from(b0) - 139.0, 1),
            247..=250 => ((f64::from(b0) - 247.0) * 256.0 + f64::from(byte(at + 1)?) + 108.0, 2),
            251..=254 => (-(f64::from(b0) - 251.0) * 256.0 - f64::from(byte(at + 1)?) - 108.0, 2),
            255 => (f64::from(i32::from_be_bytes([byte(at + 1)?, byte(at + 2)?, byte(at + 3)?, byte(at + 4)?])) / 65536.0, 5),
            op => {
                let count = args.len();
                let has_width = match op {
                    // hstem, vstem, hstemhm, vstemhm, hintmask, cntrmask, rmoveto, endchar.
                    1 | 3 | 18 | 23 | 19 | 20 | 21 | 14 => !count.is_multiple_of(2),
                    // vmoveto, hmoveto.
                    4 | 22 => count >= 2 && count.is_multiple_of(2),
                    _ => false,
                };
                return Ok(if has_width { args.first().copied() } else { None });
            }
        };
        if args.len() >= MAX_ARGS {
            return Err(TablesError::invalid("a charstring has more operands than its stack holds"));
        }
        args.push((value, len));
        at += len;
    }
    Ok(None)
}

/// A charstring number in fontTools' encoding: a 16-bit integer in its shortest form, anything
/// else as 16.16 fixed point. A number 16.16 cannot hold is refused, as fontTools refuses it.
fn encode_number(v: f64, out: &mut Vec<u8>) -> Result<()> {
    if v.fract() == 0.0 && (-32768.0..=32767.0).contains(&v) {
        let i = v as i32;
        match i {
            -107..=107 => out.push((i + 139) as u8),
            108..=1131 => out.extend_from_slice(&[(((i - 108) >> 8) + 247) as u8, ((i - 108) & 0xFF) as u8]),
            -1131..=-108 => out.extend_from_slice(&[(((-i - 108) >> 8) + 251) as u8, ((-i - 108) & 0xFF) as u8]),
            _ => {
                out.push(28);
                out.extend_from_slice(&(i as i16).to_be_bytes());
            }
        }
    } else {
        let fixed = (v * 65536.0).round();
        if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&fixed) {
            return Err(TablesError::invalid(format!("a glyph width operand of {v} is outside the range a charstring number holds")));
        }
        out.push(255);
        out.extend_from_slice(&(fixed as i32).to_be_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn widths_are_moved_to_the_first_fonts_pair() {
        // 100 hmoveto endchar with nominal 500: a width of 600.
        let cs = [139 + 100, 139 + 50, 22, 14];
        // The new default is 600: the operand goes.
        assert_eq!(rewrite_width(&cs, (0.0, 500.0), (600.0, 0.0)).unwrap(), [139 + 50, 22, 14]);
        // The new nominal is 550: the operand becomes 50.
        assert_eq!(rewrite_width(&cs, (0.0, 500.0), (0.0, 550.0)).unwrap(), [139 + 50, 139 + 50, 22, 14]);
        // No operand: the old default (700) is written against the new nominal (0).
        let bare = [139 + 50, 22, 14];
        assert_eq!(rewrite_width(&bare, (700.0, 0.0), (0.0, 0.0)).unwrap(), [249, 80, 139 + 50, 22, 14]);
        // An endchar-only glyph: one operand is a width, four are an accent.
        assert_eq!(width_operand(&[139 + 5, 14]).unwrap(), Some((5.0, 1)));
        assert_eq!(width_operand(&[139, 139, 139, 139, 14]).unwrap(), None);
        assert_eq!(width_operand(&[14]).unwrap(), None);
    }

    #[test]
    fn numbers_take_fonttools_encodings() {
        let enc = |v: f64| {
            let mut out = Vec::new();
            encode_number(v, &mut out).unwrap();
            out
        };
        assert_eq!(enc(0.0), [139]);
        assert_eq!(enc(-1131.0), [254, 255]);
        assert_eq!(enc(1132.0), [28, 0x04, 0x6C]);
        assert_eq!(enc(0.5), [255, 0, 0, 0x80, 0]);
        assert_eq!(enc(-32768.0), [28, 0x80, 0]);
        assert_eq!(enc(32767.5), [255, 0x7F, 0xFF, 0x80, 0]);
    }

    #[test]
    fn a_width_operand_past_16_16_is_refused() {
        // A width of 20,000 against a new nominal of -20,000: an operand of 40,000.
        let bare = [139 + 50, 22, 14];
        let err = rewrite_width(&bare, (20_000.0, 0.0), (0.0, -20_000.0)).unwrap_err().to_string();
        assert!(err.contains("40000"), "{err}");
        let mut out = Vec::new();
        assert!(encode_number(-32768.5, &mut out).is_err() && out.is_empty());
    }
}
