//! `CFF ` and `CFF2` outlines at a new units per em, as fontTools' `ttLib.scaleUpem` scales them
//! (MIT License, Copyright (c) 2017 Just van Rossum; see NOTICE). Written anew here.
//!
//! - The table is first written without subroutines; scaleUpem desubroutinizes too.
//! - Every charstring operand is multiplied by the factor and rounded on its own, halves up, as
//!   the relative value it is: path coordinates, stem edges, widths, the flex depth, and the
//!   defaults and deltas of each `blend`. The operand of `vsindex`, the count of each `blend` and
//!   the hint mask bytes stay. Operators keep their order and kind.
//! - The two character codes of a `seac`-style `endchar` stay. scaleUpem scales them as well,
//!   which makes the accent name another glyph; that difference is kept on purpose.
//! - In each Private DICT some glyph uses: the blue zones and stem snaps (their absolute values
//!   scaled, then written as differences again), `StdHW`, `StdVW`, `defaultWidthX` and
//!   `nominalWidthX`. `BlueScale`, `BlueShift` and `BlueFuzz` stay. A value that varies (a
//!   `CFF2` DICT `blend`) stays as it is, with a warning.
//! - `CFF ` Top DICT: `UnderlinePosition`, `UnderlineThickness` and `StrokeWidth` scaled, their
//!   defaults (-100, 50, 0) when missing, and written when the result differs from the default;
//!   `FontBBox` worked out again from the scaled outlines, as fontTools does when it saves.
//! - An explicit `FontMatrix` is divided by the factor and left out when that gives the default.
//!   A missing one stays missing: scaleUpem divides the shared default, which then equals the
//!   value and is not written.
//! - The arithmetic and storage operators of `CFF ` charstrings are refused.

use super::glyf::ot_round;
use crate::dehint::cff::{Entry, Table, encode_dict};
use crate::instance::cff2::{dict_number, font_bbox, glyph_bounds, put_cs_number, read_dict_number};
use crate::limits::{CharstringMemory, MAX_CFF_STACK, MAX_CFF2_STACK, MAX_CHARSTRING, MAX_CHARSTRING_MEMORY, MAX_SFNT};
use crate::{Result, TablesError};

const OP_FONT_BBOX: u16 = 5;
const OP_SUBRS: u16 = 19;
const OP_UNDERLINE_POSITION: u16 = 1203;
const OP_UNDERLINE_THICKNESS: u16 = 1204;
const OP_FONT_MATRIX: u16 = 1207;
const OP_STROKE_WIDTH: u16 = 1208;
/// Private DICT arrays stored as differences: BlueValues, OtherBlues, FamilyBlues,
/// FamilyOtherBlues, StemSnapH, StemSnapV.
const PRIVATE_DELTA_OPS: [u16; 6] = [6, 7, 8, 9, 1212, 1213];
/// Private DICT numbers scaled one by one: StdHW, StdVW, defaultWidthX, nominalWidthX.
const PRIVATE_PLAIN_OPS: [u16; 4] = [10, 11, 20, 21];
pub(super) const DEFAULT_MATRIX: [f64; 6] = [0.001, 0.0, 0.0, 0.001, 0.0, 0.0];

/// A scaled table and what the rest of the font needs from it.
pub(super) struct Scaled {
    pub table: Vec<u8>,
    /// Each glyph's exact bounds in the scaled table; `None` for a glyph that draws nothing.
    pub bounds: Vec<Option<[f64; 4]>>,
    pub warnings: Vec<String>,
}

/// The `CFF ` (or, with `cff2`, `CFF2`) table `data` with every value in font units multiplied
/// by `factor`.
pub(super) fn scale_upem(data: &[u8], cff2: bool, factor: f64) -> Result<Scaled> {
    scale_upem_within(data, cff2, factor, MAX_SFNT.min(MAX_CHARSTRING_MEMORY))
}

/// [`scale_upem`] with the memory the expanded and the written charstrings may take together.
pub(super) fn scale_upem_within(data: &[u8], cff2: bool, factor: f64, max_bytes: usize) -> Result<Scaled> {
    if !factor.is_finite() || factor <= 0.0 {
        return Err(TablesError::BadOptions(format!("{factor} is not a scale factor")));
    }
    let flat = fontcraft_cff::subr::desubroutinize_table_within(data, cff2, max_bytes).map_err(subr_error)?;
    let mut memory = CharstringMemory::new(max_bytes);
    memory.charge(flat.len())?;
    let mut table = Table::read(&flat, cff2)?;

    let mut used = vec![false; table.privates.len()];
    let mut charstrings = Vec::with_capacity(table.charstrings.len());
    let mut seacs = 0usize;
    for (gid, code) in table.charstrings.iter().enumerate() {
        let fd = table.fd_select.as_ref().and_then(|s| s.get(gid).copied()).map_or(0, usize::from);
        let private = table.privates.get(fd).ok_or_else(|| invalid(format!("glyph {gid} names Font DICT {fd}, which does not exist")))?;
        if let Some(u) = used.get_mut(fd) {
            *u = true;
        }
        let mut cs = Charstring { cff2, regions: &table.regions, vsindex: private.vsindex, factor, ..Charstring::default() };
        cs.run(code).map_err(|e| in_glyph(e, gid))?;
        if cs.out.len() > MAX_CHARSTRING {
            return Err(TablesError::TooLarge { what: "a scaled charstring", limit: MAX_CHARSTRING });
        }
        memory.charge(cs.out.len().saturating_add(std::mem::size_of::<Vec<u8>>()))?;
        seacs += usize::from(cs.seac);
        charstrings.push(cs.out);
    }

    let mut warnings = Vec::new();
    if seacs > 0 {
        warnings.push(format!("{seacs} glyphs build an accented character from two others (seac); their character codes are kept"));
    }
    let mut varied = 0usize;
    let mut privates = Vec::with_capacity(table.privates.len());
    for (p, used) in table.privates.iter().zip(&used) {
        let mut entries: Vec<Entry> = p.entries.iter().filter(|e| e.op != OP_SUBRS).cloned().collect();
        if *used {
            for e in entries.iter_mut() {
                if !scale_private_entry(e, factor)? {
                    varied += 1;
                }
            }
        }
        privates.push(encode_dict(entries.iter(), &[])?);
    }
    if varied > 0 {
        warnings.push(format!("{varied} Private DICT values vary across the design space and are left unscaled"));
    }

    if !cff2 {
        for (op, default) in [(OP_UNDERLINE_POSITION, -100.0), (OP_UNDERLINE_THICKNESS, 50.0), (OP_STROKE_WIDTH, 0.0)] {
            let value = match table.top.iter().find(|e| e.op == op) {
                Some(e) => match numbers(&e.raw)?.as_deref() {
                    Some([v]) => *v,
                    _ => continue,
                },
                None => default,
            };
            let scaled = ot_round(value * factor);
            set_entry(&mut table.top, op, (scaled != default).then_some(vec![scaled]))?;
        }
    }
    if let Some(e) = table.top.iter().find(|e| e.op == OP_FONT_MATRIX)
        && let Some(m) = numbers(&e.raw)?
        && m.len() == 6
    {
        let m: Vec<f64> = m.iter().map(|v| v / factor).collect();
        let default = m.iter().zip(DEFAULT_MATRIX).all(|(v, d)| (v - d).abs() <= 1e-12);
        set_entry(&mut table.top, OP_FONT_MATRIX, (!default).then_some(m))?;
    }

    let first = table.write(&charstrings, &privates)?;
    let bounds = glyph_bounds(&first)?;
    let table = if cff2 {
        first
    } else {
        let bbox = font_bbox(&bounds);
        let values = (bbox != [0; 4]).then(|| bbox.iter().map(|v| *v as f64).collect());
        set_entry(&mut table.top, OP_FONT_BBOX, values)?;
        table.write(&charstrings, &privates)?
    };
    Ok(Scaled { table, bounds, warnings })
}

pub(super) fn invalid(what: impl Into<String>) -> TablesError {
    TablesError::Invalid(what.into())
}

pub(super) fn in_glyph(e: TablesError, gid: usize) -> TablesError {
    match e {
        TablesError::Invalid(m) => TablesError::Invalid(format!("glyph {gid}: {m}")),
        TablesError::Unsupported(m) => TablesError::Unsupported(format!("glyph {gid}: {m}")),
        other => other,
    }
}

pub(super) fn subr_error(e: fontcraft_cff::subr::SubrError) -> TablesError {
    use fontcraft_cff::subr::SubrError;
    match e {
        SubrError::Invalid(m) => TablesError::Invalid(format!("the CFF data is damaged: {m}")),
        SubrError::Unsupported(m) => TablesError::Unsupported(m),
        SubrError::TooLarge { what, limit } => TablesError::TooLarge { what, limit },
        other => TablesError::Invalid(other.to_string()),
    }
}

/// The operands of a DICT entry; `None` when a `blend` makes them vary.
pub(super) fn numbers(raw: &[u8]) -> Result<Option<Vec<f64>>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(&b) = raw.get(i) {
        if b == 23 {
            return Ok(None);
        }
        let (v, next) = read_dict_number(raw, i)?;
        out.push(v);
        i = next;
    }
    Ok(Some(out))
}

/// Gives the entry `op` these operands, adding it at the end when missing; `None` removes it.
pub(super) fn set_entry(entries: &mut Vec<Entry>, op: u16, values: Option<Vec<f64>>) -> Result<()> {
    let Some(values) = values else {
        entries.retain(|e| e.op != op);
        return Ok(());
    };
    let mut raw = Vec::new();
    for v in &values {
        raw.extend(dict_number(*v)?);
    }
    let ints = values.iter().map(|v| (v.fract() == 0.0).then_some(*v as i64)).collect();
    let entry = Entry { op, raw, ints };
    match entries.iter_mut().find(|e| e.op == op) {
        Some(e) => *e = entry,
        None => entries.push(entry),
    }
    Ok(())
}

/// Scales one Private DICT entry when scaleUpem scales it; `false` when it varies and stays.
fn scale_private_entry(e: &mut Entry, factor: f64) -> Result<bool> {
    let delta = PRIVATE_DELTA_OPS.contains(&e.op);
    if !delta && !PRIVATE_PLAIN_OPS.contains(&e.op) {
        return Ok(true);
    }
    let Some(values) = numbers(&e.raw)? else { return Ok(false) };
    let scaled: Vec<f64> = if delta {
        let mut absolute = 0.0;
        let mut previous = 0.0;
        values
            .iter()
            .map(|d| {
                absolute += d;
                let s = ot_round(absolute * factor);
                let out = s - previous;
                previous = s;
                out
            })
            .collect()
    } else {
        values.iter().map(|v| ot_round(v * factor)).collect()
    };
    let op = e.op;
    let mut one = vec![e.clone()];
    set_entry(&mut one, op, Some(scaled))?;
    if let Some(new) = one.pop() {
        *e = new;
    }
    Ok(true)
}

/// One charstring being scaled. It holds no subroutine calls: the table was expanded first.
#[derive(Default)]
struct Charstring<'t> {
    cff2: bool,
    /// Region counts of the variation store's item variation data, by `vsindex`.
    regions: &'t [usize],
    vsindex: usize,
    factor: f64,
    out: Vec<u8>,
    /// The numbers read since the last operator: the value and where its bytes lie.
    pending: Vec<(f64, usize, usize)>,
    /// Values on the operand stack, a `blend`'s results counted once each.
    depth: usize,
    stems: usize,
    seac: bool,
}

impl Charstring<'_> {
    fn run(&mut self, code: &[u8]) -> Result<()> {
        let cut = || invalid("the charstring is cut short");
        let max_stack = if self.cff2 { MAX_CFF2_STACK } else { MAX_CFF_STACK };
        let mut i = 0usize;
        while let Some(&b0) = code.get(i) {
            let start = i;
            let byte = |k: usize| code.get(k).copied().ok_or_else(cut);
            let number = match b0 {
                28 => {
                    i += 3;
                    Some(f64::from(i16::from_be_bytes([byte(start + 1)?, byte(start + 2)?])))
                }
                32..=246 => {
                    i += 1;
                    Some(f64::from(i32::from(b0) - 139))
                }
                247..=250 => {
                    i += 2;
                    Some(f64::from((i32::from(b0) - 247) * 256 + i32::from(byte(start + 1)?) + 108))
                }
                251..=254 => {
                    i += 2;
                    Some(f64::from(-(i32::from(b0) - 251) * 256 - i32::from(byte(start + 1)?) - 108))
                }
                255 => {
                    i += 5;
                    let bits = i32::from_be_bytes([byte(start + 1)?, byte(start + 2)?, byte(start + 3)?, byte(start + 4)?]);
                    Some(f64::from(bits) / 65536.0)
                }
                _ => None,
            };
            if let Some(v) = number {
                if self.depth >= max_stack || self.pending.len() >= max_stack {
                    return Err(invalid(format!("more than {max_stack} operands are on the stack")));
                }
                self.pending.push((v, start, i));
                self.depth += 1;
                continue;
            }
            i += 1;
            match b0 {
                12 => {
                    let b1 = byte(i)?;
                    i += 1;
                    match b1 {
                        0 if !self.cff2 => self.emit(code, &[12, 0], 0)?,
                        34..=37 => self.emit(code, &[12, b1], 0)?,
                        _ => return Err(TablesError::Unsupported(format!("the charstring operator 12 {b1}"))),
                    }
                }
                1 | 3 | 18 | 23 => {
                    self.stems += self.depth / 2;
                    self.emit(code, &[b0], 0)?;
                }
                19 | 20 => {
                    self.stems += self.depth / 2;
                    self.emit(code, &[b0], 0)?;
                    let n = self.stems.div_ceil(8);
                    let mask = code.get(i..i.saturating_add(n)).ok_or_else(cut)?;
                    self.out.extend_from_slice(mask);
                    i += n;
                }
                15 if self.cff2 => {
                    let &(v, ..) = self.pending.last().ok_or_else(|| invalid("vsindex has no operand"))?;
                    if v.fract() != 0.0 || v < 0.0 || v >= self.regions.len() as f64 {
                        return Err(invalid(format!("vsindex {v} names no item variation data")));
                    }
                    self.vsindex = v as usize;
                    self.emit(code, &[b0], 1)?;
                }
                16 if self.cff2 => {
                    let &(n, ..) = self.pending.last().ok_or_else(|| invalid("blend has no operand"))?;
                    if n.fract() != 0.0 || n < 0.0 || n >= max_stack as f64 {
                        return Err(invalid(format!("blend of {n} values")));
                    }
                    let n = n as usize;
                    let k = *self.regions.get(self.vsindex).ok_or_else(|| invalid("blend without item variation data"))?;
                    let need = k.checked_add(1).and_then(|r| r.checked_mul(n)).and_then(|v| v.checked_add(1));
                    let Some(need) = need.filter(|need| *need <= self.depth) else {
                        return Err(invalid(format!("blend of {n} values needs more operands than the stack holds")));
                    };
                    let depth = self.depth - need + n;
                    self.emit(code, &[b0], 1)?;
                    self.depth = depth;
                }
                14 if !self.cff2 => {
                    let keep = if self.depth >= 4 && self.pending.len() >= 4 {
                        self.seac = true;
                        2
                    } else {
                        0
                    };
                    self.emit(code, &[b0], keep)?;
                }
                4..=8 | 21 | 22 | 24..=27 | 30 | 31 => self.emit(code, &[b0], 0)?,
                10 | 11 | 29 => return Err(invalid("a subroutine call is left after expansion")),
                _ => return Err(invalid(format!("byte {b0} is not a charstring operator"))),
            }
        }
        if !self.pending.is_empty() {
            self.emit(code, &[], 0)?;
        }
        Ok(())
    }

    /// Writes the pending numbers, scaled except for the last `keep`, then the operator.
    fn emit(&mut self, code: &[u8], op: &[u8], keep: usize) -> Result<()> {
        let n = self.pending.len();
        for (k, &(v, start, end)) in self.pending.iter().enumerate() {
            if k + keep >= n {
                self.out.extend_from_slice(code.get(start..end).ok_or_else(|| invalid("the charstring is cut short"))?);
            } else {
                let scaled = ot_round(v * self.factor);
                if scaled.abs() > 32_767.0 {
                    return Err(invalid(format!("a charstring value would be {scaled}, outside ±32,767")));
                }
                put_cs_number(&mut self.out, scaled)?;
            }
        }
        self.pending.clear();
        self.out.extend_from_slice(op);
        self.depth = 0;
        Ok(())
    }
}

#[cfg(test)]
#[path = "cff_tests.rs"]
pub(super) mod tests;
