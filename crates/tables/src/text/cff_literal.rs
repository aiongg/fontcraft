//! The CFF literal view (`-t CFF=literal`): the structures of a `CFF ` table as AFDKO `tx -dcf`
//! writes them, at its levels 0 (byte ranges), 1 (every byte, INDEX headers included) and 5
//! (values, the default), with its table selection (`-T`) and flowed-data switch (`-b`).
//! Ported from AFDKO `c/shared/tx_shared.cpp`, the `dcf` mode (Apache-2.0, Copyright 2014 Adobe
//! Systems Incorporated).
//!
//! Byte ranges are absolute: the table's offset in the font file plus the offset in the table.
//! Charstrings are written as stored (subroutine calls not followed); the length of each hint
//! mask comes from a first pass that follows the calls and counts the stems, as `tx` does, and a
//! subroutine's mask length is the stem count after the last call to it. The flattened listing
//! of selected glyphs (`tx -dcf -g`) and CFF2 tables are not built.

use super::Out;
use crate::{Result, TablesError};

/// How much the literal view writes (`tx -dcf -0`, `-1`, `-5`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CffLevel {
    /// The byte range of each structure.
    Offsets,
    /// Every byte, with the INDEX counts and offset arrays.
    Bytes,
    /// The values, String INDEX entries numbered by SID and subroutines by their biased number.
    #[default]
    Values,
}

/// Most operands a charstring stack holds while it is listed.
const MAX_STACK: usize = 513;
/// Deepest subroutine nesting followed while stems are counted (`tx`'s limit).
const MAX_SUBR_DEPTH: usize = 10;
/// Most operators the stem-counting pass runs per glyph.
const MAX_OPS_PER_GLYPH: usize = 100_000;
/// Most operators the stem-counting pass runs over the whole font.
const MAX_OPS: usize = 50_000_000;

/// The structures `--cff-parts` selects, by their `tx -dcf -T` letters.
const PARTS: &str = "hntsgeCfFcpl";

fn cut() -> TablesError {
    TablesError::invalid("the 'CFF ' table is cut short")
}

fn byte(d: &[u8], at: usize) -> Result<u8> {
    d.get(at).copied().ok_or_else(cut)
}

fn readn(d: &[u8], at: usize, n: usize) -> Result<usize> {
    let mut v = 0usize;
    for k in 0..n {
        v = (v << 8) | usize::from(byte(d, at.saturating_add(k))?);
    }
    Ok(v)
}

/// A CFF INDEX: where it starts and ends, its offset size and its items as ranges.
struct Index {
    begin: usize,
    end: usize,
    off_size: usize,
    items: Vec<(usize, usize)>,
}

impl Index {
    fn read(d: &[u8], begin: usize) -> Result<Self> {
        let count = readn(d, begin, 2)?;
        if count == 0 {
            return Ok(Self { begin, end: begin + 2, off_size: 0, items: Vec::new() });
        }
        let off_size = usize::from(byte(d, begin + 2)?);
        if !(1..=4).contains(&off_size) {
            return Err(TablesError::invalid("a CFF INDEX has an invalid offset size"));
        }
        let offsets = begin + 3;
        let data_ref = offsets + (count + 1) * off_size - 1;
        let mut items = Vec::with_capacity(count);
        let mut prev = readn(d, offsets, off_size)?;
        for k in 1..=count {
            let next = readn(d, offsets + k * off_size, off_size)?;
            let (a, b) = (data_ref.saturating_add(prev), data_ref.saturating_add(next));
            if b < a || b > d.len() {
                return Err(TablesError::invalid("a CFF INDEX has offsets out of order or past the table"));
            }
            items.push((a, b));
            prev = next;
        }
        let end = data_ref.saturating_add(prev);
        Ok(Self { begin, end, off_size, items })
    }
}

/// The operators of a DICT with their numeric operands.
fn dict_ops(d: &[u8]) -> Vec<(u16, Vec<f64>)> {
    let mut out = Vec::new();
    let mut stack = Vec::new();
    let mut i = 0usize;
    let b = |k: usize| d.get(k).copied().unwrap_or(0);
    while let Some(&b0) = d.get(i) {
        i += 1;
        match b0 {
            0..=21 => {
                let op = if b0 == 12 {
                    i += 1;
                    1200 + u16::from(b(i - 1))
                } else {
                    u16::from(b0)
                };
                out.push((op, std::mem::take(&mut stack)));
            }
            28 => {
                stack.push(f64::from(i16::from_be_bytes([b(i), b(i + 1)])));
                i += 2;
            }
            29 => {
                stack.push(f64::from(i32::from_be_bytes([b(i), b(i + 1), b(i + 2), b(i + 3)])));
                i += 4;
            }
            30 => {
                // Reals are not needed for the structure; skip the nibbles.
                while let Some(&n) = d.get(i) {
                    i += 1;
                    if n & 0x0F == 0x0F || n >> 4 == 0x0F {
                        break;
                    }
                }
                stack.push(0.0);
            }
            32..=246 => stack.push(f64::from(i16::from(b0) - 139)),
            247..=250 => {
                stack.push(f64::from((i16::from(b0) - 247) * 256 + i16::from(b(i)) + 108));
                i += 1;
            }
            251..=254 => {
                stack.push(f64::from(-(i16::from(b0) - 251) * 256 - i16::from(b(i)) - 108));
                i += 1;
            }
            _ => {}
        }
        if stack.len() > MAX_STACK {
            break;
        }
    }
    out
}

fn op_arg(ops: &[(u16, Vec<f64>)], op: u16, k: usize) -> Option<f64> {
    ops.iter().find(|(o, _)| *o == op).and_then(|(_, a)| a.get(k).copied())
}

/// The Top DICT of a CFF table, from its first entry.
fn top_dict(d: &[u8]) -> Option<Vec<(u16, Vec<f64>)>> {
    let names = Index::read(d, usize::from(*d.get(2)?)).ok()?;
    let tops = Index::read(d, names.end).ok()?;
    let (a, b) = *tops.items.first()?;
    Some(dict_ops(d.get(a..b)?))
}

/// Whether a `CFF ` table is CID-keyed (its Top DICT has ROS).
pub(crate) fn is_cid(d: &[u8]) -> bool {
    top_dict(d).is_some_and(|ops| ops.iter().any(|(o, _)| *o == 1230))
}

/// A run of subroutines: their ranges, bias, and the stem count after the last call to each.
#[derive(Default)]
struct Subrs {
    items: Vec<(usize, usize)>,
    bias: i64,
    stems: Vec<usize>,
}

impl Subrs {
    fn new(index: Option<&Index>) -> Self {
        let items = index.map(|i| i.items.clone()).unwrap_or_default();
        let bias = match items.len() {
            n if n < 1240 => 107,
            n if n < 33900 => 1131,
            _ => 32768,
        };
        let stems = vec![0; items.len()];
        Self { items, bias, stems }
    }
}

/// One font dict: its Private DICT range and local subroutines.
struct Fd {
    private: Option<(usize, usize)>,
    local: Option<Index>,
    subrs: Subrs,
}

fn fd_of(d: &[u8], ops: &[(u16, Vec<f64>)]) -> Result<Fd> {
    let size = op_arg(ops, 18, 0).unwrap_or(0.0).max(0.0) as usize;
    let Some(offset) = op_arg(ops, 18, 1).map(|o| o.max(0.0) as usize) else {
        return Ok(Fd { private: None, local: None, subrs: Subrs::default() });
    };
    let end = offset.saturating_add(size);
    let private = d.get(offset..end).ok_or_else(cut)?;
    let local = match op_arg(&dict_ops(private), 19, 0) {
        Some(s) => Some(Index::read(d, offset.saturating_add(s.max(0.0) as usize))?),
        None => None,
    };
    let subrs = Subrs::new(local.as_ref());
    Ok(Fd { private: Some((offset, end)), local, subrs })
}

/// The flowed text of `tx`: separators that depend on whether flowed data is broken into lines.
struct Flow<'a> {
    out: &'a mut Out,
    sep: &'static str,
    wrap: bool,
}

impl Flow<'_> {
    fn raw(&mut self, s: &str) -> Result<()> {
        self.out.push(s)
    }
    fn title(&mut self, t: &str) -> Result<()> {
        self.sep = "";
        self.out.line(&format!("--- {t}"))
    }
    fn brk(&mut self, s: &str) -> Result<()> {
        self.out.push(self.sep)?;
        self.out.push(s)?;
        self.sep = if self.wrap { "\n" } else { " " };
        Ok(())
    }
    fn arg(&mut self, s: &str) -> Result<()> {
        self.out.push(self.sep)?;
        self.out.push(s)?;
        self.sep = " ";
        Ok(())
    }
    fn op(&mut self, s: &str) -> Result<()> {
        self.out.push(self.sep)?;
        self.out.push(s)?;
        self.sep = if self.wrap { "\n  " } else { " " };
        Ok(())
    }
    fn elem_beg(&mut self, index: i64, complex: bool) -> Result<()> {
        self.out.push(self.sep)?;
        self.out.push(&format!("[{index}]={{{}", if complex && self.wrap { "\n" } else { "" }))
    }
    fn elem_end(&mut self) -> Result<()> {
        self.out.push("}")?;
        self.sep = if self.wrap { "\n" } else { " " };
        Ok(())
    }
    fn end(&mut self) -> Result<()> {
        self.out.push("\n")
    }
}

/// A number as C's `%g` writes it.
fn g(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e6 {
        return format!("{}", v as i64);
    }
    let e = format!("{v:.5e}");
    let (mantissa, exp) = e.split_once('e').unwrap_or((e.as_str(), "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let trim = |s: String| -> String { if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s } };
    if (-4..6).contains(&exp) {
        trim(format!("{v:.*}", (5 - exp) as usize))
    } else {
        format!("{}e{}{:02}", trim(mantissa.to_string()), if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

const DICT_OPS: [&str; 32] = [
    "version",
    "Notice",
    "FullName",
    "FamilyName",
    "Weight",
    "FontBBox",
    "BlueValues",
    "OtherBlues",
    "FamilyBlues",
    "FamilyOtherBlues",
    "StdHW",
    "StdVW",
    "escape",
    "UniqueID",
    "XUID",
    "charset",
    "Encoding",
    "CharStrings",
    "Private",
    "Subrs",
    "defaultWidthX",
    "nominalWidthX",
    "vsindex",
    "blend",
    "VarStore",
    "maxstack",
    "reserved26",
    "reserved27",
    "shortint",
    "longint",
    "real",
    "blendLE",
];

const DICT_ESC_OPS: [&str; 40] = [
    "Copyright",
    "isFixedPitch",
    "ItalicAngle",
    "UnderlinePosition",
    "UnderlineThickness",
    "PaintType",
    "CharstringType",
    "FontMatrix",
    "StrokeWidth",
    "BlueScale",
    "BlueShift",
    "BlueFuzz",
    "StemSnapH",
    "StemSnapV",
    "ForceBold",
    "reservedESC15",
    "lenIV",
    "LanguageGroup",
    "ExpansionFactor",
    "initialRandomSeed",
    "SyntheticBase",
    "PostScript",
    "BaseFontName",
    "BaseFontBlend",
    "numMasters",
    "reservedESC25",
    "reservedESC26",
    "reservedESC27",
    "reservedESC28",
    "reservedESC29",
    "ROS",
    "CIDFontVersion",
    "CIDFontRevision",
    "CIDFontType",
    "CIDCount",
    "UIDBase",
    "FDArray",
    "FDIndex",
    "FontName",
    "Chameleon",
];

const CS_OPS: [&str; 32] = [
    "reserved0",
    "hstem",
    "reserved2",
    "vstem",
    "vmoveto",
    "rlineto",
    "hlineto",
    "vlineto",
    "rrcurveto",
    "reserved9",
    "callsubr",
    "return",
    "escape",
    "reserved13",
    "endchar",
    "vsindex",
    "blend",
    "reserved17",
    "hstemhm",
    "hintmask",
    "cntrmask",
    "rmoveto",
    "hmoveto",
    "vstemhm",
    "rcurveline",
    "rlinecurve",
    "vvcurveto",
    "hhcurveto",
    "shortint",
    "callgsubr",
    "vhcurveto",
    "hvcurveto",
];

const CS_ESC_OPS: [&str; 39] = [
    "dotsection",
    "reservedESC1",
    "reservedESC2",
    "and",
    "or",
    "not",
    "reservedESC6",
    "reservedESC7",
    "reservedESC8",
    "abs",
    "add",
    "sub",
    "div",
    "reservedESC13",
    "neg",
    "eq",
    "reservedESC16",
    "reservedESC17",
    "drop",
    "reservedESC19",
    "put",
    "get",
    "ifelse",
    "random",
    "mul",
    "reservedESC25",
    "sqrt",
    "dup",
    "exch",
    "index",
    "roll",
    "reservedESC31",
    "reservedESC32",
    "reservedESC33",
    "hflex",
    "flex",
    "hflex1",
    "flex1",
    "cntron",
];

/// A DICT's operators and operands, as `tx` lists them.
fn dump_dict(f: &mut Flow, d: &[u8]) -> Result<()> {
    f.sep = if f.wrap { "  " } else { "" };
    let mut i = 0usize;
    let b = |k: usize| d.get(k).copied().unwrap_or(0);
    while let Some(&b0) = d.get(i) {
        i += 1;
        match b0 {
            12 => {
                let esc = b(i);
                i += 1;
                match DICT_ESC_OPS.get(usize::from(esc)) {
                    Some(name) => f.op(name)?,
                    None => f.op(&format!("reservedESC{esc}"))?,
                }
            }
            28 => {
                f.arg(&i16::from_be_bytes([b(i), b(i + 1)]).to_string())?;
                i += 2;
            }
            29 => {
                f.arg(&i32::from_be_bytes([b(i), b(i + 1), b(i + 2), b(i + 3)]).to_string())?;
                i += 4;
            }
            30 => {
                let mut s = String::new();
                let mut count = 0usize;
                let mut cur = 0u8;
                loop {
                    let nibble = if count & 1 == 1 {
                        cur & 0x0F
                    } else {
                        let Some(&n) = d.get(i) else { break };
                        i += 1;
                        cur = n;
                        n >> 4
                    };
                    count += 1;
                    if nibble == 0x0F {
                        break;
                    }
                    s.push(char::from(*b"0123456789.EE?-?".get(usize::from(nibble)).unwrap_or(&b'?')));
                    if nibble == 0x0C {
                        s.push('-');
                    }
                }
                f.arg(&s)?;
            }
            0..=27 | 31 => f.op(DICT_OPS.get(usize::from(b0)).copied().unwrap_or("reserved"))?,
            247..=250 => {
                f.arg(&(108 + 256 * (i32::from(b0) - 247) + i32::from(b(i))).to_string())?;
                i += 1;
            }
            251..=254 => {
                f.arg(&(-108 - 256 * (i32::from(b0) - 251) - i32::from(b(i))).to_string())?;
                i += 1;
            }
            255 => f.op("reserved255")?,
            _ => f.arg(&(i32::from(b0) - 139).to_string())?,
        }
    }
    if f.wrap {
        f.raw("\n")?;
    }
    Ok(())
}

/// The charstring operand bytes at `i`: the value and the bytes it takes, or `None` for an
/// operator byte.
fn cs_number(d: &[u8], i: usize, b0: u8) -> Option<(f64, usize)> {
    let b = |k: usize| d.get(k).copied().unwrap_or(0);
    match b0 {
        28 => Some((f64::from(i16::from_be_bytes([b(i), b(i + 1)])), 2)),
        32..=246 => Some((f64::from(i16::from(b0) - 139), 0)),
        247..=250 => Some((f64::from(108 + 256 * (i32::from(b0) - 247) + i32::from(b(i))), 1)),
        251..=254 => Some((f64::from(-108 - 256 * (i32::from(b0) - 251) - i32::from(b(i))), 1)),
        255 => Some((f64::from(i32::from_be_bytes([b(i), b(i + 1), b(i + 2), b(i + 3)])) / 65536.0, 4)),
        _ => None,
    }
}

/// The stem-counting pass over one charstring, following subroutine calls (`tx`'s
/// `DCF_Flatten | DCF_SaveStemCnt`).
struct Counter<'a> {
    d: &'a [u8],
    stems: usize,
    hints_done: bool,
    stack: Vec<f64>,
    ops: usize,
}

impl Counter<'_> {
    fn run(&mut self, range: (usize, usize), global: &mut Subrs, local: &mut Subrs, depth: usize) -> Result<()> {
        let cs = self.d.get(range.0..range.1).ok_or_else(cut)?;
        let mut i = 0usize;
        while let Some(&b0) = cs.get(i) {
            i += 1;
            if let Some((v, n)) = cs_number(cs, i, b0) {
                i += n;
                if self.stack.len() >= MAX_STACK {
                    return Err(TablesError::invalid("a charstring overflows its stack"));
                }
                self.stack.push(v);
                continue;
            }
            self.ops += 1;
            if self.ops > MAX_OPS_PER_GLYPH {
                return Err(TablesError::invalid("a charstring runs too many operators"));
            }
            match b0 {
                1 | 3 | 18 | 23 => {
                    self.stems += self.stack.len();
                    self.stack.clear();
                }
                19 | 20 => {
                    if !self.hints_done {
                        self.stems += self.stack.len();
                    }
                    self.hints_done = true;
                    self.stack.clear();
                    i += (self.stems / 2).div_ceil(8);
                }
                4 | 21 | 22 => {
                    self.hints_done = true;
                    self.stack.clear();
                }
                10 | 29 => {
                    if depth >= MAX_SUBR_DEPTH {
                        return Err(TablesError::invalid("charstring subroutines nest too deep"));
                    }
                    let subrs: &mut Subrs = if b0 == 10 { &mut *local } else { &mut *global };
                    let arg = (self.stack.pop().unwrap_or(0.0) as i64).saturating_add(subrs.bias);
                    let Some(callee) = usize::try_from(arg).ok().and_then(|a| subrs.items.get(a).copied()) else {
                        return Err(TablesError::invalid("a charstring calls a subroutine that does not exist"));
                    };
                    self.run(callee, global, local, depth + 1)?;
                    let subrs: &mut Subrs = if b0 == 10 { &mut *local } else { &mut *global };
                    if let Some(slot) = usize::try_from(arg).ok().and_then(|a| subrs.stems.get_mut(a)) {
                        *slot = self.stems;
                    }
                }
                11 => {}
                12 => {
                    i += 1;
                    self.stack.clear();
                }
                _ => self.stack.clear(),
            }
        }
        Ok(())
    }
}

/// A charstring as stored, with hint masks `stems` stems long.
fn dump_charstring(f: &mut Flow, d: &[u8], range: (usize, usize), stems: usize) -> Result<()> {
    let cs = d.get(range.0..range.1).ok_or_else(cut)?;
    f.sep = if f.wrap { "  " } else { "" };
    let mut stack: Vec<f64> = Vec::new();
    let flush = |f: &mut Flow, stack: &mut Vec<f64>| -> Result<()> {
        for v in stack.drain(..) {
            f.arg(&g(v))?;
        }
        Ok(())
    };
    let mut i = 0usize;
    while let Some(&b0) = cs.get(i) {
        i += 1;
        if let Some((v, n)) = cs_number(cs, i, b0) {
            i += n;
            if stack.len() >= MAX_STACK {
                return Err(TablesError::invalid("a charstring overflows its stack"));
            }
            stack.push(v);
            continue;
        }
        flush(f, &mut stack)?;
        match b0 {
            19 | 20 => {
                f.op(&format!("{}[", CS_OPS.get(usize::from(b0)).copied().unwrap_or("")))?;
                let len = (stems / 2).div_ceil(8);
                for k in 0..len {
                    let Some(m) = cs.get(i + k) else { break };
                    f.raw(&format!("{m:02X}"))?;
                }
                i += len;
                f.raw("]")?;
            }
            12 => {
                let esc = cs.get(i).copied().unwrap_or(0);
                i += 1;
                match CS_ESC_OPS.get(usize::from(esc)) {
                    Some(name) => f.op(name)?,
                    None => f.op(&format!("reservedESC{esc}"))?,
                }
            }
            _ => f.op(CS_OPS.get(usize::from(b0)).copied().unwrap_or("reserved"))?,
        }
    }
    if f.wrap {
        flush(f, &mut stack)?;
        f.raw("\n")?;
    }
    Ok(())
}

/// The literal view of a `CFF ` table found at `base` in the font file. `parts` holds the
/// `tx -dcf -T` letters of the structures to write; empty writes all of them.
pub fn dump(d: &[u8], base: u32, parts: &str, level: CffLevel, wrap: bool) -> Result<String> {
    if let Some(c) = parts.chars().find(|c| !PARTS.contains(*c)) {
        return Err(TablesError::BadOptions(format!("`{c}` is not a CFF part: give letters of {PARTS}")));
    }
    let on = |c: char| parts.is_empty() || parts.contains(c);
    let major = byte(d, 0)?;
    if major != 1 {
        return Err(TablesError::NotBuilt("the literal view of a CFF2 table"));
    }
    let header_size = usize::from(byte(d, 2)?);
    let names = Index::read(d, header_size)?;
    let tops = Index::read(d, names.end)?;
    let strings = Index::read(d, tops.end)?;
    let globals = Index::read(d, strings.end)?;
    let &(ta, tb) = tops.items.first().ok_or_else(|| TablesError::invalid("the CFF table has no Top DICT"))?;
    let top = dict_ops(d.get(ta..tb).ok_or_else(cut)?);
    let cid = top.iter().any(|(o, _)| *o == 1230);
    let offset = |op: u16, default: usize| op_arg(&top, op, 0).map_or(default, |v| v.max(0.0) as usize);
    let charstrings_at = op_arg(&top, 17, 0).ok_or_else(|| TablesError::invalid("the CFF Top DICT has no CharStrings"))?;
    let charstrings = Index::read(d, charstrings_at.max(0.0) as usize)?;
    let glyphs = charstrings.items.len();

    // The structures that are not INDEXes, with their ranges.
    let encoding = offset(16, 0);
    let encoding_end = if encoding > 1 && !cid {
        let format = byte(d, encoding)?;
        let n = usize::from(byte(d, encoding + 1)?);
        let mut end = encoding + 2 + if format & 0x7F == 0 { n } else { 2 * n };
        if format & 0x80 != 0 {
            end += 1 + 3 * usize::from(byte(d, end)?);
        }
        end
    } else {
        0
    };
    let charset = offset(15, 0);
    let charset_end = if charset > 2 {
        let format = byte(d, charset)?;
        let mut at = charset + 1;
        let mut gid = 1usize;
        match format {
            0 => at += 2 * glyphs.saturating_sub(1),
            1 | 2 => {
                while gid < glyphs {
                    let n = if format == 1 { usize::from(byte(d, at + 2)?) } else { readn(d, at + 2, 2)? };
                    at += if format == 1 { 3 } else { 4 };
                    gid += n + 1;
                }
            }
            _ => return Err(TablesError::invalid("the CFF charset has an unknown format")),
        }
        at
    } else {
        0
    };
    let fd_select = if cid { op_arg(&top, 1237, 0).map(|v| v.max(0.0) as usize) } else { None };
    let fd_select_end = match fd_select {
        Some(at) => match byte(d, at)? {
            0 => Some(at + 1 + glyphs),
            3 => Some(at + 3 + 3 * readn(d, at + 1, 2)? + 2),
            _ => return Err(TablesError::invalid("the CFF FDSelect has an unknown format")),
        },
        None => None,
    };
    let fd_array = if cid { op_arg(&top, 1236, 0).map(|v| Index::read(d, v.max(0.0) as usize)).transpose()? } else { None };
    let mut fds = Vec::new();
    match &fd_array {
        Some(index) => {
            for &(a, b) in &index.items {
                fds.push(fd_of(d, &dict_ops(d.get(a..b).ok_or_else(cut)?))?);
            }
        }
        None => fds.push(fd_of(d, &top)?),
    }
    // The font dict of each glyph, from FDSelect formats 0 and 3.
    let mut glyph_fd = vec![0usize; glyphs];
    if let Some(at) = fd_select {
        if byte(d, at)? == 0 {
            for (gid, slot) in glyph_fd.iter_mut().enumerate() {
                *slot = usize::from(byte(d, at + 1 + gid)?);
            }
        } else {
            let n = readn(d, at + 1, 2)?;
            let mut covered = 0usize;
            for k in 0..n {
                let first = readn(d, at + 3 + 3 * k, 2)?;
                let next = readn(d, at + 6 + 3 * k, 2)?;
                if first < covered || next < first {
                    return Err(TablesError::invalid("the CFF FDSelect ranges are out of order"));
                }
                covered = next;
                let fd = usize::from(byte(d, at + 5 + 3 * k)?);
                for slot in glyph_fd.iter_mut().take(next).skip(first) {
                    *slot = fd;
                }
            }
        }
    }

    // Stem counts for the hint masks.
    let mut global = Subrs::new(Some(&globals));
    let mut glyph_stems = vec![0usize; glyphs];
    if level != CffLevel::Offsets && (on('c') || on('g') || on('l')) {
        let mut empty = Subrs::default();
        let mut total = 0usize;
        for (gid, range) in charstrings.items.iter().enumerate() {
            let fd = glyph_fd.get(gid).copied().unwrap_or(0);
            let local = fds.get_mut(fd).map_or(&mut empty, |f| &mut f.subrs);
            let mut counter = Counter { d, stems: 0, hints_done: false, stack: Vec::new(), ops: 0 };
            // A charstring that cannot be followed keeps the stems counted before the failure.
            let _ = counter.run(*range, &mut global, local, 0);
            total = total.saturating_add(counter.ops);
            if total > MAX_OPS {
                return Err(TablesError::TooLarge { what: "the charstring operators counted for the hint masks", limit: MAX_OPS });
            }
            if let Some(slot) = glyph_stems.get_mut(gid) {
                *slot = counter.stems;
            }
        }
    }

    let mut out = Out::new();
    let mut f = Flow { out: &mut out, sep: "", wrap };
    let abs = |at: usize| u64::from(base).saturating_add(at as u64);
    let tag_line = |f: &mut Flow, title: &str, begin: usize, end: usize| -> Result<()> {
        let range = format!("({:08x}-{:08x})", abs(begin), abs(end).saturating_sub(1));
        if level == CffLevel::Offsets {
            let dots = " ................";
            let n = dots.len().saturating_sub(title.len());
            f.raw(&format!("### {title}{} {range}\n", dots.get(..n).unwrap_or(dots)))
        } else {
            f.raw(&format!("### {title} {range}\n"))
        }
    };
    // An INDEX with each item written by `item` (index as printed, range).
    let index = |f: &mut Flow, title: &str, index: &Index, item: &mut dyn FnMut(&mut Flow, usize, (usize, usize)) -> Result<()>| -> Result<()> {
        tag_line(f, title, index.begin, index.end)?;
        if level == CffLevel::Offsets {
            return Ok(());
        }
        if level == CffLevel::Bytes {
            f.raw(&format!("count  ={}\n", index.items.len()))?;
            if index.items.is_empty() {
                return Ok(());
            }
            f.raw(&format!("offSize={}\n", index.off_size))?;
            f.title("offset[index]=value")?;
            let offsets = index.begin + 3;
            for k in 0..=index.items.len() {
                f.brk(&format!("[{k}]={}", readn(d, offsets + k * index.off_size, index.off_size)?))?;
            }
            f.end()?;
        } else if index.items.is_empty() {
            return f.raw("empty\n");
        }
        f.title("object[index]={value}")?;
        for (k, r) in index.items.iter().enumerate() {
            item(f, k, *r)?;
        }
        f.end()
    };
    let string = |f: &mut Flow, k: i64, (a, b): (usize, usize)| -> Result<()> {
        f.elem_beg(k, false)?;
        let text: String = d.get(a..b).unwrap_or_default().iter().map(|c| char::from(*c)).collect();
        f.raw(&text)?;
        f.elem_end()
    };
    let dict = |f: &mut Flow, k: usize, (a, b): (usize, usize)| -> Result<()> {
        f.elem_beg(k as i64, true)?;
        dump_dict(f, d.get(a..b).ok_or_else(cut)?)?;
        f.elem_end()
    };
    let biased = |k: usize, bias: i64| if level == CffLevel::Values { k as i64 - bias } else { k as i64 };

    if on('h') {
        tag_line(&mut f, "Header", 0, header_size)?;
        if level != CffLevel::Offsets {
            f.raw(&format!("major  ={major}\nminor  ={}\nhdrSize={header_size}\noffSize={}\n", byte(d, 1)?, byte(d, 3)?))?;
        }
    }
    if on('n') {
        index(&mut f, "Name INDEX", &names, &mut |f, k, r| string(f, k as i64, r))?;
    }
    if on('t') {
        index(&mut f, "Top DICT INDEX", &tops, &mut |f, k, r| dict(f, k, r))?;
    }
    if on('s') {
        let shift = if level == CffLevel::Values { 391 } else { 0 };
        index(&mut f, "String INDEX", &strings, &mut |f, k, r| string(f, k as i64 + shift, r))?;
    }
    if on('g') {
        index(&mut f, "GlobalSubrINDEX", &globals, &mut |f, k, r| {
            f.elem_beg(biased(k, global.bias), true)?;
            dump_charstring(f, d, r, global.stems.get(k).copied().unwrap_or(0))?;
            f.elem_end()
        })?;
    }
    if on('e') && !cid {
        match encoding {
            0 => f.raw("### Encoding ........ (Standard)\n")?,
            1 => f.raw("### Encoding ........ (Expert)\n")?,
            at => {
                tag_line(&mut f, "Encoding", at, encoding_end)?;
                if level != CffLevel::Offsets {
                    let format = byte(d, at)?;
                    f.raw(&format!("format ={format:x}\n"))?;
                    let n = usize::from(byte(d, at + 1)?);
                    let mut p = at + 2;
                    if format & 0x7F == 0 {
                        f.raw(&format!("nCodes ={n}\n"))?;
                        f.title("glyph[gid]=code")?;
                        for gid in 1..=n {
                            f.brk(&format!("[{gid}]={}", byte(d, p)?))?;
                            p += 1;
                        }
                    } else {
                        f.raw(&format!("nRanges={n}\n"))?;
                        f.title("Range1={first,nLeft}")?;
                        for k in 0..n {
                            f.brk(&format!("[{k}]={{{},{}}}", byte(d, p)?, byte(d, p + 1)?))?;
                            p += 2;
                        }
                    }
                    f.end()?;
                    if format & 0x80 != 0 {
                        let n = usize::from(byte(d, p)?);
                        p += 1;
                        f.raw(&format!("nSups={n}\n"))?;
                        f.title("Supplement={code,sid}")?;
                        for k in 0..n {
                            f.brk(&format!("[{k}]={{{},{}}}", byte(d, p)?, readn(d, p + 1, 2)?))?;
                            p += 3;
                        }
                        f.end()?;
                    }
                }
            }
        }
    }
    if on('C') {
        match charset {
            0 => f.raw("### Charset ......... (ISOAdobe)\n")?,
            1 => f.raw("### Charset ......... (Expert)\n")?,
            2 => f.raw("### Charset ......... (Expert Subset)\n")?,
            at => {
                tag_line(&mut f, "Charset", at, charset_end)?;
                if level != CffLevel::Offsets {
                    let format = byte(d, at)?;
                    f.raw(&format!("format={format}\n"))?;
                    let mut p = at + 1;
                    let mut gid = 1usize;
                    match format {
                        0 => {
                            f.title(if cid { "glyph[gid]=cid" } else { "glyph[gid]=sid" })?;
                            while gid < glyphs {
                                f.brk(&format!("[{gid}]={}", readn(d, p, 2)?))?;
                                p += 2;
                                gid += 1;
                            }
                        }
                        _ => {
                            f.title(if format == 1 { "Range1[index]={first,nLeft}" } else { "Range2[index]={first,nLeft}" })?;
                            let mut k = 0usize;
                            while gid < glyphs {
                                let first = readn(d, p, 2)?;
                                let n = if format == 1 { usize::from(byte(d, p + 2)?) } else { readn(d, p + 2, 2)? };
                                f.brk(&format!("[{k}]={{{first},{n}}}"))?;
                                p += if format == 1 { 3 } else { 4 };
                                gid += n + 1;
                                k += 1;
                            }
                        }
                    }
                    f.end()?;
                }
            }
        }
    }
    if on('f')
        && let (Some(at), Some(end)) = (fd_select, fd_select_end)
    {
        tag_line(&mut f, "FDSelect", at, end)?;
        if level != CffLevel::Offsets {
            let format = byte(d, at)?;
            f.raw(&format!("format ={format}\n"))?;
            if format == 0 {
                f.title("glyph[gid]=fd")?;
                for gid in 0..glyphs {
                    f.brk(&format!("[{gid}]={}", byte(d, at + 1 + gid)?))?;
                }
                f.end()?;
            } else {
                let n = readn(d, at + 1, 2)?;
                f.raw(&format!("nRanges={n}\n"))?;
                f.title("Range3[index]={first,fd}")?;
                for k in 0..n {
                    f.brk(&format!("[{k}]={{{},{}}}", readn(d, at + 3 + 3 * k, 2)?, byte(d, at + 5 + 3 * k)?))?;
                }
                f.end()?;
                f.raw(&format!("sentinel={}\n", readn(d, at + 3 + 3 * n, 2)?))?;
            }
        }
    }
    if on('F')
        && let Some(fd_array) = &fd_array
    {
        index(&mut f, "FDArray INDEX", fd_array, &mut |f, k, r| dict(f, k, r))?;
    }
    if on('c') {
        index(&mut f, "CharStrings INDEX", &charstrings, &mut |f, k, r| {
            f.elem_beg(k as i64, true)?;
            dump_charstring(f, d, r, glyph_stems.get(k).copied().unwrap_or(0))?;
            f.elem_end()
        })?;
    }
    let many = fds.len() > 1;
    for (i, fd) in fds.iter().enumerate() {
        if many && (on('p') || on('l')) {
            f.raw(&format!("--- FD[{i}]\n"))?;
        }
        if on('p')
            && let Some((a, b)) = fd.private
        {
            tag_line(&mut f, "Private DICT", a, b)?;
            if level != CffLevel::Offsets {
                dump_dict(&mut f, d.get(a..b).ok_or_else(cut)?)?;
            }
        }
        if on('l')
            && let Some(local) = &fd.local
        {
            index(&mut f, "Local Subr INDEX", local, &mut |f, k, r| {
                f.elem_beg(biased(k, fd.subrs.bias), true)?;
                dump_charstring(f, d, r, fd.subrs.stems.get(k).copied().unwrap_or(0))?;
                f.elem_end()
            })?;
        }
    }
    drop(f);
    Ok(out.text)
}
