//! Rewriting a `CFF ` table without what the CFF specification (Adobe Technical Notes 5176 and
//! 5177) marks obsolete, for the audit fix `recompileCff` (row `tables.recompileCff`).
//!
//! | Removed | Where | Replaced by |
//! |---|---|---|
//! | `endchar` with four accent operands (seac) | charstrings | the base and accent charstrings drawn into the glyph |
//! | `dotsection` | charstrings | nothing (renderers ignore it) |
//! | `UniqueID`, `XUID`; `UIDBase` in a name-keyed font | Top DICT, Font DICTs | nothing |
//! | `ForceBold` | Private DICTs | nothing |
//!
//! Every subroutine call is expanded, the charstrings are cleaned, and the table is
//! subroutinised again by the subroutiniser of `fontcraft_cff`, so repeated charstring fragments
//! end up in subroutines whether or not the input had any. The result is checked: every glyph's
//! expanded charstring in the output equals its cleaned charstring, byte for byte, and a glyph
//! without seac or dotsection keeps the expanded charstring it had. Glyph outlines, hints and
//! widths are therefore unchanged.
//!
//! An accented glyph keeps its own operators, then draws its base and its accent from their own
//! origins, the accent moved by the seac offset. The base keeps its hints when the glyph has no
//! hints or path of its own; the accent's hints go, as in the CFF to CFF2 conversion
//! (`convert/cff_table.rs`). A part the charset has no glyph for draws nothing, as in a renderer.

use std::cell::OnceCell;
use std::collections::HashMap;

use fontcraft_cff::subr::{self, CharstringSet, SubrError, SubrOptions, TableRewrite};
use read_fonts::ps::cff::dict::{Entry, entries};
use read_fonts::ps::encoding::PredefinedEncoding;
use read_fonts::types::GlyphId;
use read_fonts::{FontData, FontRead};

use crate::limits::{MAX_CFF_STACK, MAX_CHARSTRING, MAX_CHARSTRING_MEMORY};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

const CFF: [u8; 4] = *b"CFF ";

/// Top DICT and Font DICT keys removed: UniqueID, XUID, and UIDBase (12 35).
const OBSOLETE_TOP: [u16; 3] = [13, 14, 1235];
/// The same in a CID-keyed font, which keeps UIDBase as part of its CID structure.
const OBSOLETE_TOP_CID: [u16; 2] = [13, 14];
/// Private DICT keys removed: ForceBold (12 14).
const OBSOLETE_PRIVATE: [u16; 1] = [1214];

// Charstring operators.
const HSTEM: u16 = 1;
const VSTEM: u16 = 3;
const VMOVETO: u16 = 4;
const RLINETO: u16 = 5;
const HLINETO: u16 = 6;
const VLINETO: u16 = 7;
const RRCURVETO: u16 = 8;
const CALLSUBR: u16 = 10;
const RETURN: u16 = 11;
const ENDCHAR: u16 = 14;
const HSTEMHM: u16 = 18;
const HINTMASK: u16 = 19;
const CNTRMASK: u16 = 20;
const RMOVETO: u16 = 21;
const HMOVETO: u16 = 22;
const VSTEMHM: u16 = 23;
const RCURVELINE: u16 = 24;
const RLINECURVE: u16 = 25;
const VVCURVETO: u16 = 26;
const HHCURVETO: u16 = 27;
const CALLGSUBR: u16 = 29;
const VHCURVETO: u16 = 30;
const HVCURVETO: u16 = 31;
const DOTSECTION: u16 = 1200;
const HFLEX: u16 = 1234;
const FLEX: u16 = 1235;
const HFLEX1: u16 = 1236;
const FLEX1: u16 = 1237;

const STEMS: [u16; 4] = [HSTEM, VSTEM, HSTEMHM, VSTEMHM];
const PATHS: [u16; 17] = [
    RMOVETO, HMOVETO, VMOVETO, RLINETO, HLINETO, VLINETO, RRCURVETO, RCURVELINE, RLINECURVE, VVCURVETO, HHCURVETO, VHCURVETO, HVCURVETO, HFLEX,
    FLEX, HFLEX1, FLEX1,
];

/// One fixed-point unit (16.16).
const ONE: i64 = 1 << 16;

/// What [`recompile_cff`] did.
#[derive(Clone, Debug)]
pub struct CffRecompile {
    pub font: SfntFont,
    /// Glyphs whose charstring changed (seac drawn out or dotsection removed).
    pub glyphs: usize,
    /// Glyphs that were drawn with seac.
    pub seac_glyphs: usize,
    /// The DICT keys removed, each once.
    pub removed_keys: Vec<&'static str>,
    /// Size of the `CFF ` table before and after.
    pub bytes_before: usize,
    pub bytes_after: usize,
}

fn key_name(op: u16) -> &'static str {
    match op {
        13 => "UniqueID",
        14 => "XUID",
        1235 => "UIDBase",
        1214 => "ForceBold",
        _ => "?",
    }
}

fn invalid(what: impl Into<String>) -> SubrError {
    SubrError::Invalid(what.into())
}

fn from_subr(e: SubrError) -> TablesError {
    match e {
        SubrError::Invalid(m) => TablesError::Invalid(format!("the CFF table: {m}")),
        SubrError::Unsupported(m) => TablesError::Unsupported(m),
        SubrError::TooLarge { what, limit } => TablesError::TooLarge { what, limit },
        other => TablesError::Invalid(other.to_string()),
    }
}

/// The font with its `CFF ` table rewritten without obsolete operators and DICT keys, and
/// subroutinised again.
pub fn recompile_cff(font: &SfntFont) -> Result<CffRecompile> {
    let data = font.get(CFF).ok_or_else(|| TablesError::MissingTable("CFF ".into()))?;
    let cff = read_fonts::tables::cff::Cff::read(FontData::new(data)).map_err(|e| TablesError::invalid(format!("the CFF table: {e}")))?;
    let top = cff.top_dicts().get(0).ok_or_else(|| TablesError::invalid("the CFF table has no Top DICT"))?;
    let cid = entries(top, None).take(4_096).any(|e| matches!(e, Ok(Entry::Ros { .. })));
    let top_keys: &[u16] = if cid { &OBSOLETE_TOP_CID } else { &OBSOLETE_TOP };
    // Glyph names, read once the first seac asks for a part.
    let by_name: OnceCell<HashMap<&[u8], usize>> = OnceCell::new();
    let standard_glyph = |code: u8| -> Option<usize> {
        if cid {
            return None;
        }
        let names = by_name.get_or_init(|| {
            let mut map = HashMap::new();
            if let Some(charset) = cff.charset(0) {
                for g in 0..charset.num_glyphs() {
                    if let Some(name) = charset.string_id(GlyphId::new(g)).and_then(|sid| cff.string(sid)) {
                        map.entry(name).or_insert(g as usize);
                    }
                }
            }
            map
        });
        names.get(PredefinedEncoding::Standard.name(code).as_bytes()).copied()
    };

    let mut expected: Vec<Vec<u8>> = Vec::new();
    let mut changed = 0usize;
    let mut seac_glyphs = 0usize;
    let rewrite = TableRewrite { top: top_keys, private: &OBSOLETE_PRIVATE };
    let (table, removed) = subr::rewrite_table(data, false, &rewrite, &mut |set: &CharstringSet| {
        let mut flat = subr::desubroutinize_within(set, MAX_CHARSTRING_MEMORY)?;
        let mut replaced: Vec<(usize, Vec<u8>)> = Vec::new();
        for (gid, code) in flat.charstrings.iter().enumerate() {
            if let Some((bytes, seac)) = clean(&flat.charstrings, gid, code, &standard_glyph).map_err(|e| in_glyph(e, gid))? {
                seac_glyphs += usize::from(seac);
                if bytes.len() > MAX_CHARSTRING {
                    return Err(SubrError::TooLarge { what: "bytes in a drawn-out accented charstring", limit: MAX_CHARSTRING });
                }
                replaced.push((gid, bytes));
            }
        }
        changed = replaced.len();
        for (gid, bytes) in replaced {
            if let Some(slot) = flat.charstrings.get_mut(gid) {
                *slot = bytes;
            }
        }
        let out = subr::subroutinize(&flat, &SubrOptions::default())?;
        expected = std::mem::take(&mut flat.charstrings);
        Ok(out)
    })
    .map_err(from_subr)?;

    let back = subr::charstrings_of_table(&table, false).and_then(|s| subr::desubroutinize_within(&s, MAX_CHARSTRING_MEMORY)).map_err(from_subr)?;
    if let Some(gid) = (0..expected.len().max(back.charstrings.len())).find(|&g| back.charstrings.get(g) != expected.get(g)) {
        return Err(TablesError::invalid(format!("glyph {gid} would draw differently after the CFF table is written again; the font was left as it was")));
    }

    let bytes_before = data.len();
    let bytes_after = table.len();
    let mut out = font.clone();
    out.insert(CFF, table);
    Ok(CffRecompile { font: out, glyphs: changed, seac_glyphs, removed_keys: removed.into_iter().map(key_name).collect(), bytes_before, bytes_after })
}

fn in_glyph(e: SubrError, gid: usize) -> SubrError {
    match e {
        SubrError::Invalid(m) => SubrError::Invalid(format!("glyph {gid}: {m}")),
        SubrError::Unsupported(m) => SubrError::Unsupported(format!("glyph {gid}: {m}")),
        other => other,
    }
}

// ── Charstrings ─────────────────────────────────────────────────────────────────────────────────

/// One operator of a flat Type 2 charstring with its operands (bytes as found, and the value in
/// 16.16) and, for `hintmask` and `cntrmask`, its mask bytes.
struct Op<'a> {
    code: u16,
    args: Vec<(&'a [u8], i64)>,
    mask: &'a [u8],
}

fn parse(code: &[u8]) -> std::result::Result<Vec<Op<'_>>, SubrError> {
    let cut = || invalid("the charstring is cut short");
    let byte = |i: usize| code.get(i).copied().ok_or_else(cut);
    let mut ops = Vec::new();
    let mut args: Vec<(&[u8], i64)> = Vec::new();
    let mut stems = 0usize;
    let mut i = 0usize;
    while let Some(&b0) = code.get(i) {
        let (len, value) = match b0 {
            28 => (3, i64::from(i16::from_be_bytes([byte(i + 1)?, byte(i + 2)?])) * ONE),
            32..=246 => (1, (i64::from(b0) - 139) * ONE),
            247..=250 => (2, ((i64::from(b0) - 247) * 256 + i64::from(byte(i + 1)?) + 108) * ONE),
            251..=254 => (2, (-(i64::from(b0) - 251) * 256 - i64::from(byte(i + 1)?) - 108) * ONE),
            255 => (5, i64::from(i32::from_be_bytes([byte(i + 1)?, byte(i + 2)?, byte(i + 3)?, byte(i + 4)?]))),
            _ => {
                let (op, op_len) = if b0 == 12 { (1200 + u16::from(byte(i + 1)?), 2) } else { (u16::from(b0), 1) };
                i += op_len;
                if matches!(op, CALLSUBR | CALLGSUBR | RETURN) {
                    return Err(invalid("a subroutine call is left after expansion"));
                }
                if STEMS.contains(&op) || op == HINTMASK || op == CNTRMASK {
                    stems = stems.saturating_add(args.len() / 2);
                }
                let mut mask: &[u8] = &[];
                if op == HINTMASK || op == CNTRMASK {
                    let n = stems.div_ceil(8);
                    mask = code.get(i..i + n).ok_or_else(cut)?;
                    i += n;
                }
                ops.push(Op { code: op, args: std::mem::take(&mut args), mask });
                continue;
            }
        };
        let bytes = code.get(i..i + len).ok_or_else(cut)?;
        if args.len() >= MAX_CFF_STACK {
            return Err(invalid("more operands than the stack holds"));
        }
        args.push((bytes, value));
        i += len;
    }
    if !args.is_empty() {
        return Err(invalid("the charstring ends with operands and no operator"));
    }
    Ok(ops)
}

/// The index of the operator that carries the glyph's width, if one does.
fn width_op(ops: &[Op<'_>]) -> Option<usize> {
    for (i, op) in ops.iter().enumerate() {
        let n = op.args.len();
        let has_width = match op.code {
            c if STEMS.contains(&c) || c == HINTMASK || c == CNTRMASK => n % 2 == 1,
            RMOVETO => n > 2,
            HMOVETO | VMOVETO => n > 1,
            ENDCHAR => n == 1 || n == 5,
            _ => continue,
        };
        return has_width.then_some(i);
    }
    None
}

/// The operands of `ops[i]` without the width.
fn operands<'o, 'a>(ops: &'o [Op<'a>], i: usize, width: Option<usize>) -> &'o [(&'a [u8], i64)] {
    let args = ops.get(i).map_or(&[][..], |o| o.args.as_slice());
    if width == Some(i) { args.get(1..).unwrap_or_default() } else { args }
}

/// Where the path of `ops` ends, from the origin, in 16.16.
fn path_end(ops: &[Op<'_>], width: Option<usize>) -> (i64, i64) {
    let (mut x, mut y) = (0i64, 0i64);
    for (i, op) in ops.iter().enumerate() {
        let a: Vec<i64> = operands(ops, i, width).iter().map(|(_, v)| *v).collect();
        let at = |k: usize| a.get(k).copied().unwrap_or(0);
        let sum = |pick: &dyn Fn(usize) -> bool| a.iter().enumerate().filter(|(k, _)| pick(*k)).fold(0i64, |s, (_, v)| s.saturating_add(*v));
        let (x0, y0) = (x, y);
        match op.code {
            RMOVETO | RLINETO | RRCURVETO | RCURVELINE | RLINECURVE | FLEX => {
                let n = if op.code == FLEX { 12.min(a.len()) } else { a.len() - a.len() % 2 };
                x = x.saturating_add(sum(&|k| k < n && k % 2 == 0));
                y = y.saturating_add(sum(&|k| k < n && k % 2 == 1));
            }
            HMOVETO => x = x.saturating_add(at(0)),
            VMOVETO => y = y.saturating_add(at(0)),
            HLINETO | VLINETO => {
                let first_h = op.code == HLINETO;
                x = x.saturating_add(sum(&|k| (k % 2 == 0) == first_h));
                y = y.saturating_add(sum(&|k| (k % 2 == 0) != first_h));
            }
            HHCURVETO => {
                let odd = a.len() % 2 == 1;
                let s = usize::from(odd);
                x = x.saturating_add(sum(&|k| k >= s && matches!((k - s) % 4, 0 | 1 | 3)));
                y = y.saturating_add(sum(&|k| (odd && k == 0) || (k >= s && (k - s) % 4 == 2)));
            }
            VVCURVETO => {
                let odd = a.len() % 2 == 1;
                let s = usize::from(odd);
                x = x.saturating_add(sum(&|k| (odd && k == 0) || (k >= s && (k - s) % 4 == 1)));
                y = y.saturating_add(sum(&|k| k >= s && matches!((k - s) % 4, 0 | 2 | 3)));
            }
            HVCURVETO | VHCURVETO => {
                // Curves of four operands alternate between starting horizontally and vertically;
                // a fifth operand on the last curve is its final off-direction delta.
                let mut horizontal = op.code == HVCURVETO;
                let mut k = 0usize;
                while k + 4 <= a.len() {
                    let last = a.len() - k < 8;
                    let extra = if last && a.len() - k == 5 { at(k + 4) } else { 0 };
                    if horizontal {
                        x = x.saturating_add(at(k)).saturating_add(at(k + 1));
                        y = y.saturating_add(at(k + 2)).saturating_add(at(k + 3));
                        x = x.saturating_add(extra);
                    } else {
                        y = y.saturating_add(at(k)).saturating_add(at(k + 2));
                        x = x.saturating_add(at(k + 1)).saturating_add(at(k + 3));
                        y = y.saturating_add(extra);
                    }
                    horizontal = !horizontal;
                    k += 4;
                }
            }
            HFLEX => x = x.saturating_add(sum(&|k| k != 2 && k < 7)),
            HFLEX1 => x = x.saturating_add(sum(&|k| matches!(k, 0 | 2 | 4 | 5 | 6 | 8))),
            FLEX1 => {
                let dx = sum(&|k| k < 10 && k % 2 == 0);
                let dy = sum(&|k| k < 10 && k % 2 == 1);
                if dx.saturating_abs() > dy.saturating_abs() {
                    x = x0.saturating_add(dx).saturating_add(at(10));
                    y = y0;
                } else {
                    x = x0;
                    y = y0.saturating_add(dy).saturating_add(at(10));
                }
            }
            _ => {}
        }
    }
    (x, y)
}

/// Writes one operand: an integer in the shortest form, anything else as 16.16 fixed.
fn encode(v: i64, out: &mut Vec<u8>) -> std::result::Result<(), SubrError> {
    if v % ONE == 0 {
        let n = v / ONE;
        match n {
            -107..=107 => out.push((n + 139) as u8),
            108..=1131 => {
                let w = n - 108;
                out.extend_from_slice(&[(247 + (w >> 8)) as u8, (w & 0xff) as u8]);
            }
            -1131..=-108 => {
                let w = -n - 108;
                out.extend_from_slice(&[(251 + (w >> 8)) as u8, (w & 0xff) as u8]);
            }
            _ => {
                let s = i16::try_from(n).map_err(|_| invalid(format!("an accent offset of {n} does not fit in a charstring")))?;
                out.push(28);
                out.extend_from_slice(&s.to_be_bytes());
            }
        }
        return Ok(());
    }
    let f = i32::try_from(v).map_err(|_| invalid("an accent offset does not fit in a charstring"))?;
    out.push(255);
    out.extend_from_slice(&f.to_be_bytes());
    Ok(())
}

fn push_op(op: &Op<'_>, skip_first: bool, out: &mut Vec<u8>) {
    for (bytes, _) in op.args.iter().skip(usize::from(skip_first)) {
        out.extend_from_slice(bytes);
    }
    if op.code >= 1200 {
        out.extend_from_slice(&[12, (op.code - 1200) as u8]);
    } else {
        out.push(op.code as u8);
    }
    out.extend_from_slice(op.mask);
}

/// The cleaned charstring of glyph `gid` and whether it was drawn with seac; `None` when it has
/// neither seac nor dotsection.
fn clean(
    charstrings: &[Vec<u8>],
    gid: usize,
    code: &[u8],
    standard_glyph: &dyn Fn(u8) -> Option<usize>,
) -> std::result::Result<Option<(Vec<u8>, bool)>, SubrError> {
    let ops = parse(code)?;
    let width = width_op(&ops);
    let seac = ops.iter().enumerate().rev().find(|(_, o)| o.code == ENDCHAR).filter(|(i, _)| operands(&ops, *i, width).len() == 4).map(|(i, _)| i);
    let dotsection = ops.iter().any(|o| o.code == DOTSECTION);
    let Some(seac_at) = seac else {
        if !dotsection {
            return Ok(None);
        }
        let mut out = Vec::with_capacity(code.len());
        for op in ops.iter().filter(|o| o.code != DOTSECTION) {
            push_op(op, false, &mut out);
        }
        return Ok(Some((out, false)));
    };

    let accent_args: Vec<i64> = operands(&ops, seac_at, width).iter().map(|(_, v)| *v).collect();
    let [adx, ady, base, accent] = accent_args.as_slice() else { return Err(invalid("seac needs four operands")) };
    let char_code = |v: i64| -> std::result::Result<u8, SubrError> {
        u8::try_from(v / ONE).ok().filter(|_| v % ONE == 0).ok_or_else(|| invalid("seac names a character code outside 0 to 255"))
    };
    let (base, accent) = (char_code(*base)?, char_code(*accent)?);

    let mut out = Vec::with_capacity(code.len() * 4);
    // The width carried by seac's own endchar goes in front of the first operator written.
    let mut pending_width: Option<&[u8]> = None;
    let own: Vec<&Op<'_>> = ops.iter().enumerate().filter(|(i, o)| *i != seac_at && o.code != DOTSECTION).map(|(_, o)| o).collect();
    if width == Some(seac_at) {
        pending_width = ops.get(seac_at).and_then(|o| o.args.first()).map(|(b, _)| *b);
    }
    for op in &own {
        push_op(op, false, &mut out);
    }
    let keep_base_hints = !own.iter().any(|o| STEMS.contains(&o.code) || o.code == HINTMASK || o.code == CNTRMASK || PATHS.contains(&o.code));
    let own_ops: Vec<Op<'_>> = own.iter().map(|o| Op { code: o.code, args: o.args.clone(), mask: o.mask }).collect();
    let mut at = path_end(&own_ops, width_op(&own_ops));

    for (part, shift, hints) in [(base, (0i64, 0i64), keep_base_hints), (accent, (*adx, *ady), false)] {
        let Some(part_gid) = standard_glyph(part) else { continue };
        if part_gid == gid {
            return Err(invalid("seac names the glyph itself"));
        }
        let part_code = charstrings.get(part_gid).ok_or_else(|| invalid("seac names a glyph beyond the CharStrings"))?;
        let part_ops = parse(part_code)?;
        let part_width = width_op(&part_ops);
        if part_ops.iter().any(|o| o.code == ENDCHAR && o.args.len() >= 4) {
            return Err(invalid("a seac part is itself accented"));
        }
        let end = path_end(&part_ops, part_width);
        let mut first_path = true;
        for (i, op) in part_ops.iter().enumerate() {
            if op.code == ENDCHAR || op.code == DOTSECTION {
                continue;
            }
            let hint_op = STEMS.contains(&op.code) || op.code == HINTMASK || op.code == CNTRMASK;
            if hint_op && !hints {
                continue;
            }
            let skip_width = part_width == Some(i);
            if first_path && PATHS.contains(&op.code) {
                first_path = false;
                // The part draws from its own origin: move there from where the drawing stands.
                let a = operands(&part_ops, i, part_width);
                let (dx, dy) = (shift.0.saturating_sub(at.0), shift.1.saturating_sub(at.1));
                let value = |k: usize| a.get(k).map_or(0, |(_, v)| *v);
                let moved = match op.code {
                    RMOVETO => Some((value(0).saturating_add(dx), value(1).saturating_add(dy))),
                    HMOVETO => Some((value(0).saturating_add(dx), dy)),
                    VMOVETO => Some((dx, value(0).saturating_add(dy))),
                    _ => None,
                };
                let (mx, my) = moved.unwrap_or((dx, dy));
                if let Some(w) = pending_width.take() {
                    out.extend_from_slice(w);
                }
                encode(mx, &mut out)?;
                encode(my, &mut out)?;
                out.push(RMOVETO as u8);
                if moved.is_some() {
                    continue;
                }
            }
            if let Some(w) = pending_width.take() {
                out.extend_from_slice(w);
            }
            push_op(op, skip_width, &mut out);
        }
        if !first_path {
            at = (shift.0.saturating_add(end.0), shift.1.saturating_add(end.1));
        }
    }
    if let Some(w) = pending_width.take() {
        out.extend_from_slice(w);
    }
    out.push(ENDCHAR as u8);
    Ok(Some((out, true)))
}

#[cfg(test)]
#[path = "recompile_cff_tests.rs"]
mod tests;
