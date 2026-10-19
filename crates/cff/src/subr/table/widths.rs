//! Choosing `defaultWidthX` and `nominalWidthX` again for an existing `CFF ` table (M12.26, row
//! `optimize`): the choice is [`crate::width::optimize`], the port of fontTools
//! `cffLib.width.optimizeWidths`; this module reads each glyph's advance out of its charstring
//! and writes the charstrings and Private DICTs back with the new pair.
//!
//! A charstring holds its width as an extra first operand of its first stack-clearing operator.
//! The charstrings are expanded first ([`flat::flatten`]), so a width can never hide inside a
//! subroutine; the table comes back without subroutines and the caller subroutinises again.
//! Every glyph's advance is read back from the result and compared with the input's.

use super::super::flat::{self, Flat};
use super::super::{CharstringSet, MAX_OPS_PER_SET, SubrError};
use super::{Entry, Table, invalid};

const OP_DEFAULT_WIDTH: u16 = 20;
const OP_NOMINAL_WIDTH: u16 = 21;
/// Widest range of advances in one Font DICT the optimiser is given; its work and memory grow
/// with the range. A wider one keeps its widths.
const MAX_WIDTH_SPAN: i64 = 1 << 18;

/// What [`optimize_widths_table`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WidthsReport {
    /// Font DICTs whose `defaultWidthX` or `nominalWidthX` changed.
    pub changed: usize,
    /// Why a Font DICT kept its widths, one line each.
    pub kept: Vec<String>,
}

/// The `CFF ` table with the `defaultWidthX` and `nominalWidthX` of each Private DICT chosen to
/// make the charstrings smallest, and every width operand rewritten to match. The charstrings
/// come back with their subroutine calls expanded and no subroutines left.
pub fn optimize_widths_table(data: &[u8]) -> Result<(Vec<u8>, WidthsReport), SubrError> {
    let mut table = Table::read(data, false)?;
    let set = super::super::desubroutinize(&table.set)?;
    let flats = flatten_all(&set)?;
    let fd_count = table.privates.len().max(1);
    let mut report = WidthsReport::default();
    let mut charstrings = set.charstrings.clone();
    let mut advances = Vec::with_capacity(flats.len());
    for fd in 0..fd_count {
        let (default, nominal) = match pair(table.privates.get(fd)) {
            Some(p) => p,
            None => {
                report.kept.push(format!("Font DICT {fd}: its widths are not whole numbers"));
                continue;
            }
        };
        let glyphs: Vec<usize> = (0..flats.len()).filter(|&g| set.fd_of(g) == fd).collect();
        let mut widths = Vec::with_capacity(glyphs.len());
        let mut found = Vec::with_capacity(glyphs.len());
        for &g in &glyphs {
            let Some(flat) = flats.get(g) else { continue };
            match width_of(flat) {
                Ok(w) => {
                    let advance = match w.operand {
                        Some(v) => nominal.checked_add(v),
                        None => Some(default),
                    };
                    let Some(advance) = advance else { break };
                    widths.push(advance);
                    found.push((g, w.skip, advance));
                }
                Err(why) => {
                    report.kept.push(format!("Font DICT {fd}: glyph {g}: {why}"));
                    break;
                }
            }
        }
        if found.len() != glyphs.len() {
            if !report.kept.iter().any(|k| k.starts_with(&format!("Font DICT {fd}:"))) {
                report.kept.push(format!("Font DICT {fd}: a width does not fit"));
            }
            continue;
        }
        let (Some(&lo), Some(&hi)) = (widths.iter().min(), widths.iter().max()) else { continue };
        if hi.saturating_sub(lo) > MAX_WIDTH_SPAN {
            report.kept.push(format!("Font DICT {fd}: its advances span more than {MAX_WIDTH_SPAN} units"));
            continue;
        }
        let Some((new_default, new_nominal)) = crate::width::optimize(&widths) else { continue };
        if (new_default, new_nominal) == (default, nominal) {
            advances.extend(found.iter().map(|&(g, _, a)| (g, a)));
            continue;
        }
        let mut rewritten = Vec::with_capacity(found.len());
        let mut ok = true;
        for &(g, skip, advance) in &found {
            let Some(flat) = flats.get(g) else { continue };
            let mut bytes = Vec::with_capacity(flat.bytes.len() + 3);
            if advance != new_default {
                let v = advance.saturating_sub(new_nominal);
                if crate::number::encode_t2_number(v as f64, "a glyph width", &mut bytes).is_err() {
                    ok = false;
                    break;
                }
            }
            bytes.extend_from_slice(flat.bytes.get(skip..).unwrap_or_default());
            rewritten.push((g, bytes));
        }
        if !ok {
            report.kept.push(format!("Font DICT {fd}: a width difference does not fit a charstring number"));
            continue;
        }
        let Some(private) = table.privates.get_mut(fd) else { continue };
        private.retain(|e| e.op != OP_DEFAULT_WIDTH && e.op != OP_NOMINAL_WIDTH);
        for (op, v) in [(OP_DEFAULT_WIDTH, new_default), (OP_NOMINAL_WIDTH, new_nominal)] {
            if v != 0 {
                let mut raw = Vec::with_capacity(5);
                crate::number::encode_dict_int(v, "a width", &mut raw).map_err(|e| invalid(e.to_string()))?;
                private.push(Entry { op, raw, ints: vec![Some(v)] });
            }
        }
        for (g, bytes) in rewritten {
            if let Some(slot) = charstrings.get_mut(g) {
                *slot = bytes;
            }
        }
        advances.extend(found.iter().map(|&(g, _, a)| (g, a)));
        report.changed += 1;
    }
    let out = CharstringSet { charstrings, ..set };
    let bytes = table.write(&out)?;
    check(&bytes, &advances)?;
    Ok((bytes, report))
}

/// The `defaultWidthX` and `nominalWidthX` of a Private DICT (0 when missing); `None` when one
/// is not an integer.
fn pair(private: Option<&Vec<Entry>>) -> Option<(i64, i64)> {
    let get = |op: u16| match private.and_then(|p| p.iter().find(|e| e.op == op)) {
        None => Some(0),
        Some(e) => e.int(0),
    };
    Some((get(OP_DEFAULT_WIDTH)?, get(OP_NOMINAL_WIDTH)?))
}

/// The width a flat charstring carries: its value relative to `nominalWidthX`, and how many
/// bytes it takes at the start.
struct Width {
    operand: Option<i64>,
    skip: usize,
}

fn width_of(flat: &Flat) -> Result<Width, &'static str> {
    let is_operator = |t: &flat::Token| flat.token_bytes(t).first().is_some_and(|&b| b <= 31 && b != 28);
    let first = flat.tokens.iter().find(|t| is_operator(t)).ok_or("the charstring has no operator")?;
    let op = flat.token_bytes(first).first().copied().unwrap_or(0);
    let depth = usize::from(first.depth);
    let has_width = match op {
        // hstem, vstem, hstemhm, vstemhm, hintmask, cntrmask, rmoveto, endchar
        1 | 3 | 18 | 23 | 19 | 20 | 21 | 14 => depth % 2 == 1,
        // vmoveto, hmoveto
        4 | 22 => depth == 2,
        _ => return Err("the charstring does not start with a hint, a move or endchar"),
    };
    if !has_width {
        return Ok(Width { operand: None, skip: 0 });
    }
    let token = flat.tokens.first().ok_or("the charstring is empty")?;
    let bytes = flat.token_bytes(token);
    let value = int_value(bytes).ok_or("its width is not a whole number")?;
    Ok(Width { operand: Some(value), skip: bytes.len() })
}

/// The integer a Type 2 number token holds; `None` for a fraction or an operator.
fn int_value(b: &[u8]) -> Option<i64> {
    let at = |i: usize| b.get(i).map(|&v| i64::from(v));
    let b0 = at(0)?;
    match b0 {
        28 => Some(i64::from(i16::from_be_bytes([*b.get(1)?, *b.get(2)?]))),
        32..=246 => Some(b0 - 139),
        247..=250 => Some((b0 - 247) * 256 + at(1)? + 108),
        251..=254 => Some(-(b0 - 251) * 256 - at(1)? - 108),
        255 => {
            let v = i32::from_be_bytes([*b.get(1)?, *b.get(2)?, *b.get(3)?, *b.get(4)?]);
            (v & 0xFFFF == 0).then_some(i64::from(v >> 16))
        }
        _ => None,
    }
}

fn flatten_all(set: &CharstringSet) -> Result<Vec<Flat>, SubrError> {
    let mut budget = MAX_OPS_PER_SET;
    (0..set.charstrings.len()).map(|g| flat::flatten(set, g, &mut budget)).collect()
}

/// Reads every advance of the rewritten table back and compares it with the one it had.
fn check(bytes: &[u8], advances: &[(usize, i64)]) -> Result<(), SubrError> {
    let table = Table::read(bytes, false)?;
    let flats = flatten_all(&table.set)?;
    for &(g, expected) in advances {
        let fd = table.set.fd_of(g);
        let (default, nominal) = pair(table.privates.get(fd)).ok_or(SubrError::Mismatch { glyph: g })?;
        let flat = flats.get(g).ok_or(SubrError::Mismatch { glyph: g })?;
        let w = width_of(flat).map_err(|_| SubrError::Mismatch { glyph: g })?;
        let advance = match w.operand {
            Some(v) => nominal.checked_add(v),
            None => Some(default),
        };
        if advance != Some(expected) {
            return Err(SubrError::Mismatch { glyph: g });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Font, Glyph, PathOp, Widths};

    /// A name-keyed table whose widths are all explicit (`defaultWidthX` and `nominalWidthX` 0).
    fn table(advances: &[f64]) -> Vec<u8> {
        let square = vec![PathOp::MoveTo((10.0, 0.0)), PathOp::LineTo((90.0, 0.0)), PathOp::LineTo((90.0, 80.0)), PathOp::LineTo((10.0, 80.0))];
        let glyphs = advances
            .iter()
            .enumerate()
            .map(|(i, &advance)| Glyph {
                name: if i == 0 { ".notdef".into() } else { format!("g{i}") },
                advance,
                path: if i % 2 == 0 { Vec::new() } else { square.clone() },
                hints: None,
            })
            .collect();
        let font = Font { name: "Test".into(), glyphs, widths: Widths::Fixed { default_width_x: 0, nominal_width_x: 0 }, ..Font::default() };
        crate::build(&font).unwrap().table
    }

    fn read_advances(bytes: &[u8]) -> Vec<i64> {
        let table = Table::read(bytes, false).unwrap();
        let flats = flatten_all(&table.set).unwrap();
        let (default, nominal) = pair(table.privates.first()).unwrap();
        flats.iter().map(|f| width_of(f).unwrap().operand.map_or(default, |v| nominal + v)).collect()
    }

    #[test]
    fn widths_are_chosen_again_and_every_advance_stays() {
        // Enough glyphs that the width operands saved outweigh the fixed cost of the rewrite's
        // five-byte DICT offsets.
        let pattern = [500, 600, 600, 600, 600, 1200, 250];
        let expected: Vec<i64> = pattern.iter().copied().cycle().take(pattern.len() * 10).collect();
        let advances: Vec<f64> = expected.iter().map(|&a| a as f64).collect();
        let input = table(&advances);
        let before = read_advances(&input);
        assert_eq!(before, expected);
        let (out, report) = optimize_widths_table(&input).unwrap();
        assert_eq!(report.changed, 1, "{report:?}");
        assert!(out.len() < input.len(), "{} >= {}", out.len(), input.len());
        assert_eq!(read_advances(&out), before);
        let table = Table::read(&out, false).unwrap();
        assert_eq!(pair(table.privates.first()).unwrap().0, 600, "the most common advance becomes the default");
        let (again, report) = optimize_widths_table(&out).unwrap();
        assert_eq!((again.len(), report.changed), (out.len(), 0), "a second run finds nothing to change");
    }

    #[test]
    fn a_table_that_is_not_cff_is_refused() {
        assert!(optimize_widths_table(&[2, 0, 5, 0, 0]).is_err());
        assert!(optimize_widths_table(&[]).is_err());
    }
}
