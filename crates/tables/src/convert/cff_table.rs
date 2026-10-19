//! `CFF ` ↔ `CFF2` table conversion (M12.30), after fontTools `cffLib.CFFToCFF2` and
//! `cffLib.CFF2ToCFF`, and the drawing after `misc.psCharStrings.T2OutlineExtractor` (MIT,
//! Copyright (c) 2017 Just van Rossum and contributors), written anew.
//!
//! Both directions work on charstrings with every subroutine call expanded
//! (`fontcraft_cff::subr::desubroutinize_table`); the caller subroutinises the result again.
//! The DICT, INDEX and FDSelect readers follow the hint remover's (`dehint/cff.rs`).
//!
//! | Direction | Charstrings | DICTs |
//! |---|---|---|
//! | CFF → CFF2 | the width operand, `endchar` and `dotsection` go; stems, masks and flex stay | Top DICT: FontMatrix and the offsets; one Font DICT per Private DICT, holding only Private; Private DICT: the CFF2 keys (zones, stems, LanguageGroup, ExpansionFactor) |
//! | CFF2 → CFF | the advance from `hmtx` goes in front as the width, `endchar` is added, operators with more than 48 operands are split | name-keyed with the `post` names and one Font DICT; CID-keyed (Adobe-Identity-0, `cidNNNNN` order) otherwise, as the reference does; `defaultWidthX` and `nominalWidthX` chosen by the width optimiser |
//!
//! A variable `CFF2` table (one whose charstrings or Private DICTs blend) is refused: it must be
//! instantiated first.
//!
//! An accented glyph drawn with `seac` (`endchar` with four operands) has no CFF2 form: its base
//! and accent charstrings are written into it, the accent moved by its offset. The base keeps
//! its hints when the glyph has no path or hints of its own; the accent's hints go.
//!
//! [`Programs::draw`] draws the expanded charstrings as fontTools' `T2OutlineExtractor` does:
//! 16.16 operands added up exactly, no FontMatrix, an implied `moveto` before a path that lacks
//! one.

use super::outline::{Collect, Contour};
use crate::limits::{MAX_CFF_STACK, MAX_CFF2_STACK, MAX_CHARSTRING, MAX_SFNT};
use crate::{Result, TablesError};

/// Most entries one DICT may hold.
const MAX_DICT_ENTRIES: usize = 4_096;
/// Most Font DICTs (an FDSelect index of CFF is one byte).
const MAX_FONT_DICTS: usize = 256;
/// Most glyphs (CharStrings INDEX entries).
const MAX_GLYPHS: usize = 65_535;
/// Most operators one expanded charstring may hold.
const MAX_OPS_PER_GLYPH: usize = 1 << 20;

const OP_CHARSET: u16 = 15;
const OP_CHARSTRINGS: u16 = 17;
const OP_PRIVATE: u16 = 18;
const OP_SUBRS: u16 = 19;
const OP_VSINDEX: u16 = 22;
const OP_MAXSTACK: u16 = 25;
const OP_FONT_MATRIX: u16 = 1207;
const OP_ROS: u16 = 1230;
const OP_CID_COUNT: u16 = 1234;
const OP_FDARRAY: u16 = 1236;
const OP_FDSELECT: u16 = 1237;
const OP_FONT_NAME: u16 = 1238;
const OP_FONT_BBOX: u16 = 5;

/// The Private DICT keys CFF2 keeps: BlueValues, OtherBlues, FamilyBlues, FamilyOtherBlues,
/// StdHW, StdVW, BlueScale, BlueShift, BlueFuzz, StemSnapH, StemSnapV, LanguageGroup,
/// ExpansionFactor.
const CFF2_PRIVATE_KEYS: [u16; 13] = [6, 7, 8, 9, 10, 11, 1209, 1210, 1211, 1212, 1213, 1217, 1218];

/// One fixed-point unit (16.16).
const ONE: i64 = 1 << 16;

fn invalid(what: impl Into<String>) -> TablesError {
    TablesError::Invalid(what.into())
}

fn in_glyph(e: TablesError, gid: usize) -> TablesError {
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

// ── Reading ─────────────────────────────────────────────────────────────────────────────────────

/// One DICT entry: its operator, its operand bytes as found, and the operands as integers
/// (`None` for reals and CFF2 blends), and whether a CFF2 `blend` is among them.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    op: u16,
    raw: Vec<u8>,
    ints: Vec<Option<i64>>,
    blend: bool,
}

impl Entry {
    fn int(&self, i: usize) -> Option<i64> {
        self.ints.get(i).copied().flatten()
    }

    /// An entry of whole-number operands.
    fn of_ints(op: u16, values: &[i64]) -> Result<Self> {
        let mut raw = Vec::new();
        for v in values {
            encode_dict_int(*v, &mut raw)?;
        }
        Ok(Entry { op, raw, ints: values.iter().map(|v| Some(*v)).collect(), blend: false })
    }
}

/// A `CFF ` or `CFF2` table whose charstrings call no subroutines.
struct Flat<'a> {
    top: Vec<Entry>,
    /// CID-keyed fonts and CFF2: the Font DICTs.
    font_dicts: Option<Vec<Vec<Entry>>>,
    /// The Font DICT of every glyph.
    fd_select: Vec<u16>,
    privates: Vec<Vec<Entry>>,
    charstrings: Vec<&'a [u8]>,
}

impl<'a> Flat<'a> {
    fn read(data: &'a [u8], cff2: bool) -> Result<Self> {
        let major = *data.first().ok_or_else(|| invalid("the CFF table is empty"))?;
        let hdr_size = usize::from(*data.get(2).ok_or_else(|| invalid("the CFF header is cut short"))?);
        if major != if cff2 { 2 } else { 1 } {
            return Err(TablesError::Unsupported(format!("CFF major version {major}")));
        }
        let (top_bytes, gsubr_at) = if cff2 {
            let top_len = usize::from(read_u16(data, 3).ok_or_else(|| invalid("the CFF2 header is cut short"))?);
            let end = hdr_size.checked_add(top_len).ok_or_else(|| invalid("the CFF2 Top DICT lies outside the table"))?;
            (data.get(hdr_size..end).ok_or_else(|| invalid("the CFF2 Top DICT lies outside the table"))?, end)
        } else {
            let names = read_index(data, hdr_size, false)?;
            let tops = read_index(data, names.end, false)?;
            if tops.items.len() != 1 {
                return Err(TablesError::Unsupported(format!("a CFF table with {} fonts", tops.items.len())));
            }
            let strings = read_index(data, tops.end, false)?;
            (tops.items.first().copied().unwrap_or_default(), strings.end)
        };
        let global = read_index(data, gsubr_at, cff2)?;
        if !global.items.is_empty() {
            return Err(invalid("the expanded table still has global subroutines"));
        }
        let top = parse_dict(top_bytes, cff2)?;
        let offset = |op: u16| -> Result<Option<usize>> {
            match top.iter().find(|e| e.op == op) {
                None => Ok(None),
                Some(e) => {
                    let last = e.ints.len().checked_sub(1).and_then(|i| e.int(i));
                    last.and_then(|v| usize::try_from(v).ok()).map(Some).ok_or_else(|| invalid("a Top DICT offset is not a positive integer"))
                }
            }
        };
        let cs_at = offset(OP_CHARSTRINGS)?.ok_or_else(|| invalid("the Top DICT has no CharStrings"))?;
        let charstrings = read_index(data, cs_at, cff2)?.items;
        let glyphs = charstrings.len();
        if glyphs == 0 || glyphs > MAX_GLYPHS {
            return Err(invalid(format!("the CharStrings INDEX holds {glyphs} glyphs")));
        }
        let (font_dicts, refs) = match offset(OP_FDARRAY)? {
            Some(at) => {
                let fds = read_index(data, at, cff2)?.items;
                if fds.is_empty() || fds.len() > MAX_FONT_DICTS {
                    return Err(invalid(format!("the FDArray holds {} Font DICTs", fds.len())));
                }
                let mut dicts = Vec::with_capacity(fds.len());
                let mut refs = Vec::with_capacity(fds.len());
                for fd in fds {
                    let entries = parse_dict(fd, cff2)?;
                    refs.push(private_ref(&entries)?);
                    dicts.push(entries);
                }
                (Some(dicts), refs)
            }
            None if cff2 => return Err(invalid("the CFF2 Top DICT has no FDArray")),
            None => (None, vec![private_ref(&top)?]),
        };
        let fd_select = match offset(OP_FDSELECT)? {
            Some(at) => read_fd_select(data, at, glyphs, cff2)?,
            None => vec![0; glyphs],
        };
        if fd_select.iter().any(|fd| usize::from(*fd) >= refs.len()) {
            return Err(invalid("the FDSelect names a Font DICT that does not exist"));
        }
        let mut privates = Vec::with_capacity(refs.len());
        for r in refs {
            privates.push(match r {
                None => Vec::new(),
                Some((size, at)) => parse_dict(blob(data, at, size)?, cff2)?,
            });
        }
        Ok(Flat { top, font_dicts, fd_select, privates, charstrings })
    }

    fn private_blends(&self) -> bool {
        self.privates.iter().flatten().any(|e| e.op == OP_VSINDEX || e.blend)
    }
}

/// The Private DICT a Top or Font DICT points at: `(size, offset)`.
fn private_ref(entries: &[Entry]) -> Result<Option<(usize, usize)>> {
    let Some(e) = entries.iter().find(|e| e.op == OP_PRIVATE) else { return Ok(None) };
    let (Some(size), Some(at)) = (e.int(0), e.int(1)) else { return Err(invalid("a Private entry needs a size and an offset")) };
    let size = usize::try_from(size).map_err(|_| invalid("a negative Private DICT size"))?;
    let at = usize::try_from(at).map_err(|_| invalid("a negative Private DICT offset"))?;
    Ok(Some((size, at)))
}

fn blob(data: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    at.checked_add(len).and_then(|end| data.get(at..end)).ok_or_else(|| invalid("a CFF structure lies outside the table"))
}

struct Index<'a> {
    items: Vec<&'a [u8]>,
    end: usize,
}

/// The INDEX at `at`: a 16-bit count in CFF, a 32-bit count in CFF2.
fn read_index(data: &[u8], at: usize, cff2: bool) -> Result<Index<'_>> {
    let cut = || invalid("an INDEX is cut short");
    let (count, head) = if cff2 { (read_u32(data, at).ok_or_else(cut)? as usize, 4) } else { (usize::from(read_u16(data, at).ok_or_else(cut)?), 2) };
    let start = at.checked_add(head).ok_or_else(cut)?;
    if count == 0 {
        return Ok(Index { items: Vec::new(), end: start });
    }
    let off_size = usize::from(*data.get(start).ok_or_else(cut)?);
    if !(1..=4).contains(&off_size) {
        return Err(invalid(format!("an INDEX has offset size {off_size}")));
    }
    // Every item takes at least one offset's bytes, so a count beyond the data is a lie.
    let offsets_len = count.checked_add(1).and_then(|n| n.checked_mul(off_size)).filter(|n| *n <= data.len()).ok_or_else(cut)?;
    let offsets_at = start + 1;
    let base = (offsets_at + offsets_len).checked_sub(1).ok_or_else(cut)?;
    let read_off = |i: usize| -> Result<usize> {
        let p = offsets_at + i * off_size;
        let bytes = data.get(p..p + off_size).ok_or_else(cut)?;
        Ok(bytes.iter().fold(0usize, |acc, b| (acc << 8) | usize::from(*b)))
    };
    let mut items = Vec::with_capacity(count);
    let mut prev = read_off(0)?;
    if prev != 1 {
        return Err(invalid("an INDEX does not start at offset 1"));
    }
    for i in 1..=count {
        let next = read_off(i)?;
        let (a, b) = (base.checked_add(prev).ok_or_else(cut)?, base.checked_add(next).ok_or_else(cut)?);
        items.push(data.get(a..b).ok_or_else(|| invalid("an INDEX item lies outside the table"))?);
        prev = next;
    }
    Ok(Index { items, end: base + prev })
}

/// The DICT entries of `bytes`. In CFF2, `blend` (23) belongs to the operands of the entry it
/// appears in.
fn parse_dict(bytes: &[u8], cff2: bool) -> Result<Vec<Entry>> {
    let cut = || invalid("a DICT is cut short");
    let mut entries = Vec::new();
    let mut raw = Vec::new();
    let mut ints = Vec::new();
    let mut blend = false;
    let mut i = 0usize;
    while let Some(&b0) = bytes.get(i) {
        match b0 {
            28 => {
                let v = read_u16(bytes, i + 1).ok_or_else(cut)? as i16;
                raw.extend_from_slice(bytes.get(i..i + 3).ok_or_else(cut)?);
                ints.push(Some(i64::from(v)));
                i += 3;
            }
            29 => {
                let v = read_u32(bytes, i + 1).ok_or_else(cut)? as i32;
                raw.extend_from_slice(bytes.get(i..i + 5).ok_or_else(cut)?);
                ints.push(Some(i64::from(v)));
                i += 5;
            }
            30 => {
                let start = i;
                i += 1;
                loop {
                    let b = *bytes.get(i).ok_or_else(cut)?;
                    i += 1;
                    if b >> 4 == 0xF || b & 0xF == 0xF {
                        break;
                    }
                }
                raw.extend_from_slice(bytes.get(start..i).ok_or_else(cut)?);
                ints.push(None);
            }
            32..=246 => {
                raw.push(b0);
                ints.push(Some(i64::from(b0) - 139));
                i += 1;
            }
            247..=254 => {
                let b1 = i64::from(*bytes.get(i + 1).ok_or_else(cut)?);
                let v = if b0 <= 250 { (i64::from(b0) - 247) * 256 + b1 + 108 } else { -(i64::from(b0) - 251) * 256 - b1 - 108 };
                raw.extend_from_slice(&[b0, b1 as u8]);
                ints.push(Some(v));
                i += 2;
            }
            23 if cff2 => {
                raw.push(b0);
                ints.push(None);
                blend = true;
                i += 1;
            }
            0..=27 => {
                let op = if b0 == 12 {
                    let b1 = *bytes.get(i + 1).ok_or_else(cut)?;
                    i += 2;
                    1200 + u16::from(b1)
                } else {
                    i += 1;
                    u16::from(b0)
                };
                if entries.len() >= MAX_DICT_ENTRIES {
                    return Err(TablesError::TooLarge { what: "a DICT's entry count", limit: MAX_DICT_ENTRIES });
                }
                entries.push(Entry { op, raw: std::mem::take(&mut raw), ints: std::mem::take(&mut ints), blend: std::mem::take(&mut blend) });
            }
            _ => return Err(invalid(format!("byte {b0} is reserved in a DICT"))),
        }
    }
    if !raw.is_empty() {
        return Err(invalid("a DICT ends with operands and no operator"));
    }
    Ok(entries)
}

/// The Font DICT of every glyph.
fn read_fd_select(data: &[u8], at: usize, glyphs: usize, cff2: bool) -> Result<Vec<u16>> {
    let cut = || invalid("the FDSelect is cut short");
    let format = *data.get(at).ok_or_else(cut)?;
    match format {
        0 => Ok(data.get(at + 1..at + 1 + glyphs).ok_or_else(cut)?.iter().map(|b| u16::from(*b)).collect()),
        3 | 4 if format == 3 || cff2 => {
            let wide = format == 4;
            let (count, mut p) = if wide {
                (read_u32(data, at + 1).ok_or_else(cut)? as usize, at + 5)
            } else {
                (usize::from(read_u16(data, at + 1).ok_or_else(cut)?), at + 3)
            };
            if count == 0 || count > glyphs {
                return Err(invalid(format!("the FDSelect has {count} ranges for {glyphs} glyphs")));
            }
            let mut map = vec![0u16; glyphs];
            let read_first = |p: usize| if wide { read_u32(data, p).map(|v| v as usize) } else { read_u16(data, p).map(usize::from) };
            let mut first = read_first(p).ok_or_else(cut)?;
            for _ in 0..count {
                let fd = if wide { read_u16(data, p + 4).ok_or_else(cut)? } else { u16::from(*data.get(p + 2).ok_or_else(cut)?) };
                p += if wide { 6 } else { 3 };
                let next = read_first(p).ok_or_else(cut)?;
                if next < first || next > glyphs {
                    return Err(invalid("the FDSelect ranges are out of order"));
                }
                for slot in map.get_mut(first..next).ok_or_else(cut)? {
                    *slot = fd;
                }
                first = next;
            }
            Ok(map)
        }
        other => Err(invalid(format!("FDSelect format {other}"))),
    }
}

// ── Charstrings ─────────────────────────────────────────────────────────────────────────────────

/// One charstring operator with its operands (16.16 fixed point) and, for `hintmask` and
/// `cntrmask`, its mask bytes. `code` is the operator byte, or 1200 plus the byte after the
/// escape.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Op {
    code: u16,
    args: Vec<i64>,
    mask: Vec<u8>,
}

/// An expanded charstring cut into operators.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Program {
    /// CFF: the width operand of the first stack-clearing operator.
    width: Option<i64>,
    /// Every operator but `endchar`.
    ops: Vec<Op>,
    /// CFF: the operands of an accented `endchar` (seac): adx, ady, base code, accent code.
    seac: Option<[i64; 4]>,
}

const HINTMASK: u16 = 19;
const CNTRMASK: u16 = 20;
const ENDCHAR: u16 = 14;
const DOTSECTION: u16 = 1200;

/// Cuts an expanded charstring into operators. A variable CFF2 charstring (`blend`,
/// `vsindex`) and the arithmetic operators are refused.
fn parse_program(code: &[u8], cff2: bool) -> Result<Program> {
    let cut = || invalid("a charstring is cut short");
    let mut program = Program::default();
    let mut args: Vec<i64> = Vec::new();
    let mut stems = 0usize;
    let mut cleared = false;
    let mut ended = false;
    let mut i = 0usize;
    let limit = if cff2 { MAX_CFF2_STACK } else { MAX_CFF_STACK };
    while let Some(&b0) = code.get(i) {
        if ended {
            break;
        }
        if program.ops.len() >= MAX_OPS_PER_GLYPH {
            return Err(TablesError::TooLarge { what: "the operators of one glyph", limit: MAX_OPS_PER_GLYPH });
        }
        let number = match b0 {
            28 => {
                let v = read_u16(code, i + 1).ok_or_else(cut)? as i16;
                i += 3;
                Some(i64::from(v) * ONE)
            }
            32..=246 => {
                i += 1;
                Some((i64::from(b0) - 139) * ONE)
            }
            247..=254 => {
                let b1 = i64::from(*code.get(i + 1).ok_or_else(cut)?);
                i += 2;
                Some(if b0 <= 250 { (i64::from(b0) - 247) * 256 + b1 + 108 } else { -(i64::from(b0) - 251) * 256 - b1 - 108 } * ONE)
            }
            255 => {
                let v = read_u32(code, i + 1).ok_or_else(cut)? as i32;
                i += 5;
                Some(i64::from(v))
            }
            _ => None,
        };
        if let Some(v) = number {
            if args.len() >= limit {
                return Err(invalid(format!("more than {limit} operands on the stack")));
            }
            args.push(v);
            continue;
        }
        let op = if b0 == 12 {
            let b1 = *code.get(i + 1).ok_or_else(cut)?;
            i += 2;
            1200 + u16::from(b1)
        } else {
            i += 1;
            u16::from(b0)
        };
        // The first stack-clearing operator carries the width when it has one operand more
        // than it needs.
        let width_parity = match op {
            1 | 3 | 18 | 23 | HINTMASK | CNTRMASK => Some(args.len() % 2 == 1),
            21 => Some(args.len() == 3),
            4 | 22 => Some(args.len() == 2),
            ENDCHAR if !cff2 => Some(args.len() == 1 || args.len() == 5),
            _ => None,
        };
        if let Some(has_width) = width_parity {
            if !cff2 && !cleared && has_width && !args.is_empty() {
                program.width = Some(args.remove(0));
            }
            cleared = true;
        }
        let mut mask = Vec::new();
        match op {
            1 | 3 | 18 | 23 => stems += args.len() / 2,
            HINTMASK | CNTRMASK => {
                stems += args.len() / 2;
                let n = stems.div_ceil(8);
                mask = code.get(i..i + n).ok_or_else(|| invalid("a hint mask is cut short"))?.to_vec();
                i += n;
            }
            ENDCHAR if !cff2 => {
                if let [adx, ady, base, accent] = args.as_slice() {
                    let code = |v: i64| (0..=255 * ONE).contains(&v) && v % ONE == 0;
                    if !(code(*base) && code(*accent)) {
                        return Err(invalid("seac names a character code outside 0 to 255"));
                    }
                    program.seac = Some([*adx, *ady, *base, *accent]);
                    args.clear();
                    ended = true;
                    continue;
                }
                if !args.is_empty() {
                    return Err(invalid("endchar has operands left"));
                }
                ended = true;
                continue;
            }
            4 | 5 | 6 | 7 | 8 | 21 | 22 | 24 | 25 | 26 | 27 | 30 | 31 | DOTSECTION | 1234..=1237 => {}
            10 | 29 => return Err(invalid("a subroutine call is left after expansion")),
            11 if !cff2 => return Err(invalid("a return is left after expansion")),
            15 | 16 if cff2 => return Err(TablesError::Unsupported("a variable CFF2 font: instantiate it first".into())),
            1203..=1205 | 1209..=1212 | 1214 | 1215 | 1218 | 1220..=1224 | 1226..=1230 => {
                return Err(TablesError::Unsupported(format!("the charstring arithmetic operator 12 {}", op - 1200)));
            }
            other => return Err(invalid(format!("operator {other} is reserved"))),
        }
        program.ops.push(Op { code: op, args: std::mem::take(&mut args), mask });
    }
    if !args.is_empty() {
        return Err(invalid("operands are left at the end of the charstring"));
    }
    if !cff2 && !ended {
        return Err(invalid("the charstring has no endchar"));
    }
    Ok(program)
}

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

const STEM_OPS: [u16; 6] = [1, 3, 18, 23, HINTMASK, CNTRMASK];

fn is_path(code: u16) -> bool {
    matches!(code, 4..=8 | 21 | 22 | 24..=27 | 30 | 31 | 1234..=1237)
}

/// A 16.16 value in font units.
fn fixed(v: i64) -> f64 {
    v as f64 / ONE as f64
}

/// The pen position in 16.16 units, moved as a Type 2 charstring moves it.
struct Turtle<'a> {
    pen: &'a mut Collect,
    at: (i64, i64),
    moved: bool,
}

impl Turtle<'_> {
    fn step(&mut self, d: (i64, i64)) -> (f64, f64) {
        self.at = (self.at.0.saturating_add(d.0), self.at.1.saturating_add(d.1));
        (fixed(self.at.0), fixed(self.at.1))
    }

    fn move_by(&mut self, dx: i64, dy: i64) {
        self.close();
        let p = self.step((dx, dy));
        self.pen.move_to(p);
        self.moved = true;
    }

    fn line_by(&mut self, dx: i64, dy: i64) {
        if !self.moved {
            self.move_by(0, 0);
        }
        let p = self.step((dx, dy));
        self.pen.line_to(p);
    }

    fn curve_by(&mut self, a: (i64, i64), b: (i64, i64), c: (i64, i64)) {
        if !self.moved {
            self.move_by(0, 0);
        }
        let p1 = self.step(a);
        let p2 = self.step(b);
        let p3 = self.step(c);
        self.pen.curve_to(p1, p2, p3);
    }

    fn close(&mut self) {
        if self.moved {
            self.pen.close();
        }
        self.moved = false;
    }
}

/// Draws expanded charstring operators into `pen` from the origin; answers the pen position
/// at the end.
fn run(ops: &[Op], pen: &mut Collect) -> Result<(i64, i64)> {
    let mut t = Turtle { pen, at: (0, 0), moved: false };
    for op in ops {
        let a = op.args.as_slice();
        let bad = || invalid(format!("operator {} cannot take {} operands", op.code, a.len()));
        match (op.code, a) {
            (21, [x, y]) => t.move_by(*x, *y),
            (22, [x]) => t.move_by(*x, 0),
            (4, [y]) => t.move_by(0, *y),
            (5, _) if a.len() % 2 == 0 => {
                for [x, y] in a.as_chunks::<2>().0 {
                    t.line_by(*x, *y);
                }
            }
            (6 | 7, _) => {
                let mut horizontal = op.code == 6;
                for v in a {
                    if horizontal {
                        t.line_by(*v, 0)
                    } else {
                        t.line_by(0, *v)
                    }
                    horizontal = !horizontal;
                }
            }
            (8, _) if a.len() % 6 == 0 => curves(&mut t, a),
            (24, _) if a.len() >= 8 && (a.len() - 2) % 6 == 0 => {
                let (c, l) = a.split_at(a.len() - 2);
                curves(&mut t, c);
                if let [x, y] = l {
                    t.line_by(*x, *y);
                }
            }
            (25, _) if a.len() >= 6 && a.len() % 2 == 0 => {
                let (l, c) = a.split_at(a.len() - 6);
                for [x, y] in l.as_chunks::<2>().0 {
                    t.line_by(*x, *y);
                }
                curves(&mut t, c);
            }
            (26 | 27, _) => {
                let (mut first, rest) = match a.len() % 4 {
                    0 => (0, a),
                    1 => (a.first().copied().unwrap_or(0), a.get(1..).unwrap_or(&[])),
                    _ => return Err(bad()),
                };
                for [p, q, r, s] in rest.as_chunks::<4>().0 {
                    if op.code == 26 {
                        t.curve_by((first, *p), (*q, *r), (0, *s));
                    } else {
                        t.curve_by((*p, first), (*q, *r), (*s, 0));
                    }
                    first = 0;
                }
            }
            (30 | 31, _) => {
                let mut rest = a;
                let mut vertical = op.code == 30;
                while !rest.is_empty() {
                    let (p, q, r, s, last, tail) = match rest {
                        [p, q, r, s, last] => (*p, *q, *r, *s, *last, &[][..]),
                        [p, q, r, s, tail @ ..] => (*p, *q, *r, *s, 0, tail),
                        _ => return Err(bad()),
                    };
                    if vertical {
                        t.curve_by((0, p), (q, r), (s, last));
                    } else {
                        t.curve_by((p, 0), (q, r), (last, s));
                    }
                    vertical = !vertical;
                    rest = tail;
                }
            }
            (1234, [dx1, dx2, dy2, dx3, dx4, dx5, dx6]) => {
                t.curve_by((*dx1, 0), (*dx2, *dy2), (*dx3, 0));
                t.curve_by((*dx4, 0), (*dx5, -dy2), (*dx6, 0));
            }
            (1235, [dx1, dy1, dx2, dy2, dx3, dy3, dx4, dy4, dx5, dy5, dx6, dy6, _fd]) => {
                t.curve_by((*dx1, *dy1), (*dx2, *dy2), (*dx3, *dy3));
                t.curve_by((*dx4, *dy4), (*dx5, *dy5), (*dx6, *dy6));
            }
            (1236, [dx1, dy1, dx2, dy2, dx3, dx4, dx5, dy5, dx6]) => {
                t.curve_by((*dx1, *dy1), (*dx2, *dy2), (*dx3, 0));
                t.curve_by((*dx4, 0), (*dx5, *dy5), (*dx6, -(dy1 + dy2 + dy5)));
            }
            (1237, [dx1, dy1, dx2, dy2, dx3, dy3, dx4, dy4, dx5, dy5, d6]) => {
                let dx = dx1 + dx2 + dx3 + dx4 + dx5;
                let dy = dy1 + dy2 + dy3 + dy4 + dy5;
                let last = if dx.abs() > dy.abs() { (*d6, -dy) } else { (-dx, *d6) };
                t.curve_by((*dx1, *dy1), (*dx2, *dy2), (*dx3, *dy3));
                t.curve_by((*dx4, *dy4), (*dx5, *dy5), last);
            }
            (code, _) if is_path(code) => return Err(bad()),
            _ => {}
        }
    }
    t.close();
    Ok(t.at)
}

/// `rrcurveto` operands: six per curve.
fn curves(t: &mut Turtle<'_>, a: &[i64]) {
    for [a1, b1, a2, b2, a3, b3] in a.as_chunks::<6>().0 {
        t.curve_by((*a1, *b1), (*a2, *b2), (*a3, *b3));
    }
}

/// An accented glyph's parts: character codes in the standard encoding, the accent's offset
/// in font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Seac {
    pub adx: f64,
    pub ady: f64,
    pub base: u8,
    pub accent: u8,
}

/// A glyph as drawn: its own contours, and its `seac` parts.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Drawing {
    pub contours: Vec<Contour>,
    pub seac: Option<Seac>,
}

/// The expanded charstrings of a `CFF ` or static `CFF2` table, cut into operators.
pub(super) struct Programs {
    programs: Vec<Program>,
}

impl Programs {
    pub(super) fn read(table: &[u8], cff2: bool) -> Result<Self> {
        let expanded = fontcraft_cff::subr::desubroutinize_table(table, cff2).map_err(subr_error)?;
        let flat = Flat::read(&expanded, cff2)?;
        if cff2 && flat.private_blends() {
            return Err(TablesError::Unsupported("a variable CFF2 font: instantiate it first".into()));
        }
        let programs =
            flat.charstrings.iter().enumerate().map(|(gid, code)| parse_program(code, cff2).map_err(|e| in_glyph(e, gid))).collect::<Result<_>>()?;
        Ok(Programs { programs })
    }

    pub(super) fn len(&self) -> usize {
        self.programs.len()
    }

    pub(super) fn is_accented(&self, gid: usize) -> bool {
        self.programs.get(gid).is_some_and(|p| p.seac.is_some())
    }

    pub(super) fn draw(&self, gid: usize) -> Result<Drawing> {
        let p = self.programs.get(gid).ok_or_else(|| invalid(format!("glyph {gid} is beyond the CharStrings")))?;
        let mut pen = Collect::default();
        run(&p.ops, &mut pen).map_err(|e| in_glyph(e, gid))?;
        let code = |v: i64| u8::try_from(v / ONE).unwrap_or(0);
        let seac = p.seac.map(|[adx, ady, base, accent]| Seac { adx: fixed(adx), ady: fixed(ady), base: code(base), accent: code(accent) });
        Ok(Drawing { contours: pen.done()?, seac })
    }
}

/// The operators of a `seac` glyph with its base and accent written in. `standard_glyph` finds
/// the glyph of a standard-encoding code; a part the font lacks is left out, as the reference
/// leaves out a component it cannot find.
fn flatten_seac(charstrings: &[&[u8]], own: &[Op], seac: [i64; 4], standard_glyph: &dyn Fn(u8) -> Option<usize>) -> Result<Vec<Op>> {
    let [adx, ady, base, accent] = seac;
    let part = |code: i64| -> Result<Option<Program>> {
        let code = u8::try_from(code / ONE).map_err(|_| invalid("seac names a character code outside 0 to 255"))?;
        let Some(gid) = standard_glyph(code) else { return Ok(None) };
        let bytes = charstrings.get(gid).ok_or_else(|| invalid("seac names a glyph beyond the CharStrings"))?;
        let p = parse_program(bytes, false)?;
        if p.seac.is_some() {
            return Err(invalid("a seac part is itself accented"));
        }
        Ok(Some(p))
    };
    let keep_base_hints = !own.iter().any(|op| STEM_OPS.contains(&op.code) || is_path(op.code));
    let mut out: Vec<Op> = own.to_vec();
    let mut at = run(own, &mut Collect::default())?;
    for (program, shift, hints) in [(part(base)?, (0, 0), keep_base_hints), (part(accent)?, (adx, ady), false)] {
        let Some(program) = program else { continue };
        let end = run(&program.ops, &mut Collect::default())?;
        let mut first = true;
        for op in program.ops {
            if STEM_OPS.contains(&op.code) {
                if hints {
                    out.push(op);
                }
                continue;
            }
            if first && is_path(op.code) {
                first = false;
                // The part draws from its own origin: move there from where the last part ended.
                let (dx, dy) = (shift.0 - at.0, shift.1 - at.1);
                let moved = match (op.code, op.args.as_slice()) {
                    (21, [x, y]) => Some((x + dx, y + dy)),
                    (22, [x]) => Some((x + dx, dy)),
                    (4, [y]) => Some((dx, y + dy)),
                    _ => None,
                };
                let (x, y) = moved.unwrap_or((dx, dy));
                out.push(Op { code: 21, args: vec![x, y], mask: Vec::new() });
                if moved.is_some() {
                    continue;
                }
            }
            out.push(op);
        }
        if !first {
            at = (shift.0 + end.0, shift.1 + end.1);
        }
    }
    Ok(out)
}

/// Writes one operand: an integer in the shortest form, anything else as 16.16 fixed.
fn encode_num(v: i64, out: &mut Vec<u8>) -> Result<()> {
    if v % ONE == 0 {
        let n = v / ONE;
        match n {
            -107..=107 => out.push((n + 139) as u8),
            108..=1131 => {
                let m = n - 108;
                out.extend_from_slice(&[(m / 256 + 247) as u8, (m % 256) as u8]);
            }
            -1131..=-108 => {
                let m = -n - 108;
                out.extend_from_slice(&[(m / 256 + 251) as u8, (m % 256) as u8]);
            }
            -32768..=32767 => {
                out.push(28);
                out.extend_from_slice(&(n as i16).to_be_bytes());
            }
            _ => return Err(invalid(format!("the number {n} does not fit in a charstring"))),
        }
        return Ok(());
    }
    let raw = i32::try_from(v).map_err(|_| invalid("a fixed-point number does not fit in a charstring"))?;
    out.push(255);
    out.extend_from_slice(&raw.to_be_bytes());
    Ok(())
}

fn encode_op(op: &Op, out: &mut Vec<u8>) -> Result<()> {
    for a in &op.args {
        encode_num(*a, out)?;
    }
    if op.code >= 1200 {
        out.extend_from_slice(&[12, (op.code - 1200) as u8]);
    } else {
        out.push(op.code as u8);
    }
    out.extend_from_slice(&op.mask);
    if out.len() > MAX_CHARSTRING {
        return Err(TablesError::TooLarge { what: "an expanded charstring", limit: MAX_CHARSTRING });
    }
    Ok(())
}

/// `op` as operators of at most `limit` operands each, with the same drawing.
fn split_for_stack(op: Op, limit: usize) -> Result<Vec<Op>> {
    let n = op.args.len();
    if n <= limit {
        return Ok(vec![op]);
    }
    let chunked = |code: u16, args: &[i64], group: usize| -> Vec<Op> {
        let per = (limit / group).max(1) * group;
        args.chunks(per).map(|c| Op { code, args: c.to_vec(), mask: Vec::new() }).collect()
    };
    let too_many = || TablesError::Unsupported(format!("operator {} with {n} operands, more than CFF's {limit}", op.code));
    match op.code {
        // rlineto, rrcurveto
        5 => Ok(chunked(5, &op.args, 2)),
        8 => Ok(chunked(8, &op.args, 6)),
        // hlineto, vlineto alternate direction: an even chunk length keeps the alternation.
        6 | 7 => Ok(chunked(op.code, &op.args, 2)),
        // vvcurveto, hhcurveto: an optional first operand, then groups of four.
        26 | 27 => {
            let lead = n % 4;
            let (head, rest) = op.args.split_at(lead.min(n));
            let mut first = head.to_vec();
            let room = (limit - first.len()) / 4 * 4;
            let take = room.min(rest.len());
            first.extend_from_slice(rest.get(..take).unwrap_or_default());
            let mut out = vec![Op { code: op.code, args: first, mask: Vec::new() }];
            out.extend(chunked(op.code, rest.get(take..).unwrap_or_default(), 4));
            Ok(out)
        }
        // hvcurveto, vhcurveto: groups of four alternating, the last with an optional fifth
        // operand. Chunks of an even number of curves keep the alternation.
        30 | 31 => {
            let mut out = Vec::new();
            let mut rest = op.args.as_slice();
            let per = (limit / 8).max(1) * 8;
            while rest.len() > limit {
                let take = if rest.len() - per < 4 { per - 8 } else { per };
                let (a, b) = rest.split_at(take);
                out.push(Op { code: op.code, args: a.to_vec(), mask: Vec::new() });
                rest = b;
            }
            out.push(Op { code: op.code, args: rest.to_vec(), mask: Vec::new() });
            Ok(out)
        }
        // rcurveline: curves, then one line.
        24 => {
            let (curves, line) = op.args.split_at(n.saturating_sub(2));
            let mut out = chunked(8, curves, 6);
            out.push(Op { code: 5, args: line.to_vec(), mask: Vec::new() });
            Ok(out)
        }
        // rlinecurve: lines, then one curve.
        25 => {
            let (lines, curve) = op.args.split_at(n.saturating_sub(6));
            let mut out = chunked(5, lines, 2);
            out.push(Op { code: 8, args: curve.to_vec(), mask: Vec::new() });
            Ok(out)
        }
        _ => Err(too_many()),
    }
}

// ── Writing ─────────────────────────────────────────────────────────────────────────────────────

fn encode_dict_int(v: i64, out: &mut Vec<u8>) -> Result<()> {
    match v {
        -107..=107 => out.push((v + 139) as u8),
        108..=1131 => {
            let m = v - 108;
            out.extend_from_slice(&[(m / 256 + 247) as u8, (m % 256) as u8]);
        }
        -1131..=-108 => {
            let m = -v - 108;
            out.extend_from_slice(&[(m / 256 + 251) as u8, (m % 256) as u8]);
        }
        -32768..=32767 => {
            out.push(28);
            out.extend_from_slice(&(v as i16).to_be_bytes());
        }
        _ => {
            let v = i32::try_from(v).map_err(|_| invalid("a DICT number does not fit in 32 bits"))?;
            out.push(29);
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    Ok(())
}

fn push_op(op: u16, out: &mut Vec<u8>) {
    if op >= 1200 {
        out.extend_from_slice(&[12, (op - 1200) as u8]);
    } else {
        out.push(op as u8);
    }
}

fn encode_entries(entries: &[Entry], out: &mut Vec<u8>) {
    for e in entries {
        out.extend_from_slice(&e.raw);
        push_op(e.op, out);
    }
}

/// An offset entry: every operand a five-byte integer, so its size is known before its value.
fn push_offsets(op: u16, values: &[usize], out: &mut Vec<u8>) -> Result<()> {
    for v in values {
        let v = i32::try_from(*v).map_err(|_| TablesError::TooLarge { what: "a CFF offset", limit: i32::MAX as usize })?;
        out.push(29);
        out.extend_from_slice(&v.to_be_bytes());
    }
    push_op(op, out);
    Ok(())
}

fn off_size(last: usize) -> usize {
    match last {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    }
}

fn index_len(lens: &[usize], cff2: bool) -> usize {
    let head = if cff2 { 4 } else { 2 };
    if lens.is_empty() {
        return head;
    }
    let total: usize = lens.iter().sum();
    head + 1 + (lens.len() + 1) * off_size(total + 1) + total
}

fn write_index(out: &mut Vec<u8>, items: &[&[u8]], cff2: bool) -> Result<()> {
    let count = items.len();
    if cff2 {
        let c = u32::try_from(count).map_err(|_| TablesError::TooLarge { what: "an INDEX", limit: u32::MAX as usize })?;
        out.extend_from_slice(&c.to_be_bytes());
    } else {
        let c = u16::try_from(count).map_err(|_| TablesError::TooLarge { what: "an INDEX", limit: 65_535 })?;
        out.extend_from_slice(&c.to_be_bytes());
    }
    if count == 0 {
        return Ok(());
    }
    let total: usize = items.iter().map(|i| i.len()).sum();
    let size = off_size(total + 1);
    out.push(size as u8);
    let put = |out: &mut Vec<u8>, v: usize| {
        let bytes = (v as u32).to_be_bytes();
        out.extend_from_slice(bytes.get(4 - size..).unwrap_or_default());
    };
    let mut offset = 1usize;
    put(out, offset);
    for item in items {
        offset += item.len();
        put(out, offset);
    }
    for item in items {
        out.extend_from_slice(item);
    }
    Ok(())
}

/// The parts of a table to lay out.
struct Parts {
    cff2: bool,
    /// CFF: the font name (Name INDEX) and the String INDEX.
    name: Vec<u8>,
    strings: Vec<Vec<u8>>,
    /// Top DICT entries without the offsets.
    top: Vec<Entry>,
    charset: Option<Vec<u8>>,
    fd_select: Option<Vec<u8>>,
    charstrings: Vec<Vec<u8>>,
    /// Font DICT entries without Private; `None` puts the one Private DICT in the Top DICT.
    font_dicts: Option<Vec<Vec<Entry>>>,
    privates: Vec<Vec<u8>>,
}

impl Parts {
    /// Header, (Name INDEX, Top DICT INDEX, String INDEX) or the CFF2 Top DICT, an empty Global
    /// Subr INDEX, charset, FDSelect, CharStrings, FDArray, Private DICTs.
    fn write(&self) -> Result<Vec<u8>> {
        let cff2 = self.cff2;
        let top_with = |at: &Layout| -> Result<Vec<u8>> {
            let mut top = Vec::new();
            encode_entries(&self.top, &mut top);
            if self.charset.is_some() {
                push_offsets(OP_CHARSET, &[at.charset], &mut top)?;
            }
            if self.fd_select.is_some() {
                push_offsets(OP_FDSELECT, &[at.fd_select], &mut top)?;
            }
            push_offsets(OP_CHARSTRINGS, &[at.charstrings], &mut top)?;
            if self.font_dicts.is_some() {
                push_offsets(OP_FDARRAY, &[at.fdarray], &mut top)?;
            } else {
                let size = self.privates.first().map_or(0, Vec::len);
                push_offsets(OP_PRIVATE, &[size, at.privates.first().copied().unwrap_or(0)], &mut top)?;
            }
            Ok(top)
        };
        let fd_with = |i: usize, at: &Layout| -> Result<Vec<u8>> {
            let mut fd = Vec::new();
            if let Some(entries) = self.font_dicts.as_ref().and_then(|d| d.get(i)) {
                encode_entries(entries, &mut fd);
            }
            push_offsets(OP_PRIVATE, &[self.privates.get(i).map_or(0, Vec::len), at.privates.get(i).copied().unwrap_or(0)], &mut fd)?;
            Ok(fd)
        };
        let zero = Layout { privates: vec![0; self.privates.len()], ..Layout::default() };
        let top_len = top_with(&zero)?.len();
        let fd_lens: Vec<usize> = match &self.font_dicts {
            Some(d) => (0..d.len()).map(|i| fd_with(i, &zero).map(|b| b.len())).collect::<Result<_>>()?,
            None => Vec::new(),
        };
        let string_lens: Vec<usize> = self.strings.iter().map(Vec::len).collect();
        let cs_lens: Vec<usize> = self.charstrings.iter().map(Vec::len).collect();

        let header_len = if cff2 { 5 } else { 4 };
        let mut pos = header_len;
        if cff2 {
            pos += top_len;
        } else {
            pos += index_len(&[self.name.len()], false) + index_len(&[top_len], false) + index_len(&string_lens, false);
        }
        pos += index_len(&[], cff2);
        let mut at = Layout::default();
        if let Some(c) = &self.charset {
            at.charset = pos;
            pos += c.len();
        }
        if let Some(f) = &self.fd_select {
            at.fd_select = pos;
            pos += f.len();
        }
        at.charstrings = pos;
        pos += index_len(&cs_lens, cff2);
        at.fdarray = pos;
        if self.font_dicts.is_some() {
            pos += index_len(&fd_lens, cff2);
        }
        for p in &self.privates {
            at.privates.push(pos);
            pos += p.len();
        }
        if pos > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the CFF table", limit: MAX_SFNT });
        }

        let top = top_with(&at)?;
        let mut out = Vec::with_capacity(pos);
        if cff2 {
            let len = u16::try_from(top.len()).map_err(|_| TablesError::TooLarge { what: "the CFF2 Top DICT", limit: 65_535 })?;
            out.extend_from_slice(&[2, 0, 5]);
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(&top);
        } else {
            out.extend_from_slice(&[1, 0, 4, 4]);
            write_index(&mut out, &[self.name.as_slice()], false)?;
            write_index(&mut out, &[top.as_slice()], false)?;
            let strings: Vec<&[u8]> = self.strings.iter().map(Vec::as_slice).collect();
            write_index(&mut out, &strings, false)?;
        }
        write_index(&mut out, &[], cff2)?;
        for part in [&self.charset, &self.fd_select].into_iter().flatten() {
            out.extend_from_slice(part);
        }
        let cs: Vec<&[u8]> = self.charstrings.iter().map(Vec::as_slice).collect();
        write_index(&mut out, &cs, cff2)?;
        if let Some(d) = &self.font_dicts {
            let fds: Vec<Vec<u8>> = (0..d.len()).map(|i| fd_with(i, &at)).collect::<Result<_>>()?;
            let refs: Vec<&[u8]> = fds.iter().map(Vec::as_slice).collect();
            write_index(&mut out, &refs, cff2)?;
        }
        for p in &self.privates {
            out.extend_from_slice(p);
        }
        if out.len() != pos {
            return Err(invalid("the CFF layout did not come out as planned"));
        }
        Ok(out)
    }
}

#[derive(Default)]
struct Layout {
    charset: usize,
    fd_select: usize,
    charstrings: usize,
    fdarray: usize,
    privates: Vec<usize>,
}

/// FDSelect format 3 for `map` (one Font DICT per glyph).
fn fd_select_format3(map: &[u16]) -> Result<Vec<u8>> {
    let mut ranges: Vec<(usize, u16)> = Vec::new();
    for (gid, fd) in map.iter().enumerate() {
        if ranges.last().is_none_or(|r| r.1 != *fd) {
            ranges.push((gid, *fd));
        }
    }
    let mut out = vec![3];
    out.extend_from_slice(&u16::try_from(ranges.len()).map_err(|_| invalid("too many FDSelect ranges"))?.to_be_bytes());
    for (first, fd) in &ranges {
        out.extend_from_slice(&u16::try_from(*first).map_err(|_| invalid("a glyph id beyond 65535"))?.to_be_bytes());
        out.push(u8::try_from(*fd).map_err(|_| invalid("a Font DICT index beyond 255"))?);
    }
    out.extend_from_slice(&u16::try_from(map.len()).map_err(|_| invalid("more than 65535 glyphs"))?.to_be_bytes());
    Ok(out)
}

/// The Top DICT FontMatrix the CFF2 table needs: the Top DICT's own, or the one every Font DICT
/// shares when the Top DICT has none or an identity matrix.
fn cff2_font_matrix(flat: &Flat<'_>) -> Result<Option<Entry>> {
    let top = flat.top.iter().find(|e| e.op == OP_FONT_MATRIX).cloned();
    let fd_matrices: Vec<Option<&Entry>> = flat.font_dicts.iter().flatten().map(|d| d.iter().find(|e| e.op == OP_FONT_MATRIX)).collect();
    if fd_matrices.iter().all(Option::is_none) {
        return Ok(top);
    }
    let identity = top.as_ref().is_none_or(|e| e.ints == [Some(1), Some(0), Some(0), Some(1), Some(0), Some(0)]);
    let first = fd_matrices.first().copied().flatten();
    if identity && fd_matrices.iter().all(|m| m.map(|e| &e.raw) == first.map(|e| &e.raw)) {
        return Ok(first.cloned());
    }
    Err(TablesError::Unsupported("Font DICTs with different FontMatrix values (CFF2 has one, in the Top DICT)".into()))
}

/// The `CFF2` table for a `CFF ` table, and whether any `dotsection` was dropped.
/// `standard_glyph` finds the glyph of a standard-encoding code, for `seac` glyphs.
pub(super) fn cff_to_cff2(cff: &[u8], standard_glyph: &dyn Fn(u8) -> Option<usize>) -> Result<(Vec<u8>, bool)> {
    let expanded = fontcraft_cff::subr::desubroutinize_table(cff, false).map_err(subr_error)?;
    let flat = Flat::read(&expanded, false)?;
    let mut charstrings = Vec::with_capacity(flat.charstrings.len());
    let mut dropped_dotsection = false;
    let mut total = 0usize;
    for (gid, code) in flat.charstrings.iter().enumerate() {
        let program = parse_program(code, false).map_err(|e| in_glyph(e, gid))?;
        let ops = match program.seac {
            None => program.ops,
            Some(seac) => flatten_seac(&flat.charstrings, &program.ops, seac, standard_glyph).map_err(|e| in_glyph(e, gid))?,
        };
        let mut out = Vec::new();
        for op in &ops {
            if op.code == DOTSECTION {
                dropped_dotsection = true;
                continue;
            }
            encode_op(op, &mut out).map_err(|e| in_glyph(e, gid))?;
        }
        total = total.saturating_add(out.len());
        if total > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the CFF2 charstrings", limit: MAX_SFNT });
        }
        charstrings.push(out);
    }
    let mut top = Vec::new();
    if let Some(m) = cff2_font_matrix(&flat)? {
        top.push(m);
    }
    let privates: Vec<Vec<u8>> = flat
        .privates
        .iter()
        .map(|p| {
            let kept: Vec<Entry> = p.iter().filter(|e| CFF2_PRIVATE_KEYS.contains(&e.op)).cloned().collect();
            let mut out = Vec::new();
            encode_entries(&kept, &mut out);
            out
        })
        .collect();
    let fd_count = privates.len();
    let fd_select = if fd_count > 1 { Some(fd_select_format3(&flat.fd_select)?) } else { None };
    let parts = Parts {
        cff2: true,
        name: Vec::new(),
        strings: Vec::new(),
        top,
        charset: None,
        fd_select,
        charstrings,
        font_dicts: Some(vec![Vec::new(); fd_count]),
        privates,
    };
    Ok((parts.write()?, dropped_dotsection))
}

/// The `CFF ` table for a static `CFF2` table: `advances` are the `hmtx` widths in glyph order.
/// With `names` (one per glyph, `.notdef` first) and one Font DICT the result is name-keyed;
/// otherwise it is CID-keyed, Adobe-Identity-0 with CID n for glyph n, as the reference writes.
pub(super) fn cff2_to_cff(cff2: &[u8], advances: &[u16], names: Option<&[String]>, font_name: &str, font_bbox: [i64; 4]) -> Result<(Vec<u8>, bool)> {
    let expanded = fontcraft_cff::subr::desubroutinize_table(cff2, true).map_err(subr_error)?;
    let flat = Flat::read(&expanded, true)?;
    if flat.private_blends() {
        return Err(TablesError::Unsupported("a variable CFF2 font: instantiate it first".into()));
    }
    let glyphs = flat.charstrings.len();
    if advances.len() < glyphs {
        return Err(invalid(format!("hmtx has {} advances for {glyphs} glyphs", advances.len())));
    }
    let mut charstrings = Vec::with_capacity(glyphs);
    let mut total = 0usize;
    for (gid, code) in flat.charstrings.iter().enumerate() {
        let program = parse_program(code, true).map_err(|e| in_glyph(e, gid))?;
        let width = i64::from(advances.get(gid).copied().unwrap_or(0));
        let mut out = Vec::new();
        let mut first = true;
        for op in program.ops {
            if first && width != 0 {
                // The first operator clears the stack; the width goes in front of its operands.
                if op.args.len() + 1 > MAX_CFF_STACK {
                    return Err(in_glyph(TablesError::Unsupported(format!("operator {} with {} operands and a width", op.code, op.args.len())), gid));
                }
                encode_num(width * ONE, &mut out)?;
            }
            if first && !matches!(op.code, 1 | 3 | 4 | 18 | 19 | 20 | 21 | 22 | 23) {
                return Err(in_glyph(invalid("the charstring does not start with a stem, a mask or a move"), gid));
            }
            first = false;
            for piece in split_for_stack(op, MAX_CFF_STACK).map_err(|e| in_glyph(e, gid))? {
                encode_op(&piece, &mut out).map_err(|e| in_glyph(e, gid))?;
            }
        }
        if first && width != 0 {
            encode_num(width * ONE, &mut out)?;
        }
        out.push(ENDCHAR as u8);
        if out.len() > MAX_CHARSTRING {
            return Err(TablesError::TooLarge { what: "a charstring", limit: MAX_CHARSTRING });
        }
        total = total.saturating_add(out.len());
        if total > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the CFF charstrings", limit: MAX_SFNT });
        }
        charstrings.push(out);
    }

    let mut strings = Strings::default();
    let mut top = Vec::new();
    let keyed_by_name = names.filter(|n| n.len() == glyphs && flat.privates.len() == 1);
    let (name, charset, font_dicts, fd_select) = match keyed_by_name {
        Some(names) => {
            let mut charset = vec![0u8];
            for n in names.iter().skip(1) {
                charset.extend_from_slice(&strings.sid(n)?.to_be_bytes());
            }
            (font_name, charset, None, None)
        }
        None => {
            let registry = strings.sid("Adobe")?; // brand-ok: the CFF registry value
            let ordering = strings.sid("Identity")?;
            top.push(Entry::of_ints(OP_ROS, &[i64::from(registry), i64::from(ordering), 0])?);
            // CID n is glyph n: format 2 with one range.
            let mut charset = vec![2u8];
            if glyphs > 1 {
                charset.extend_from_slice(&1u16.to_be_bytes());
                charset.extend_from_slice(&u16::try_from(glyphs - 2).map_err(|_| invalid("too many glyphs"))?.to_be_bytes());
            }
            let fds: Vec<Vec<Entry>> = (0..flat.privates.len())
                .map(|i| {
                    let sid = strings.sid(&format!("{font_name}-{i}"))?;
                    Entry::of_ints(OP_FONT_NAME, &[i64::from(sid)]).map(|e| vec![e])
                })
                .collect::<Result<_>>()?;
            (font_name, charset, Some(fds), Some(fd_select_format3(&flat.fd_select)?))
        }
    };
    if let Some(m) = flat.top.iter().find(|e| e.op == OP_FONT_MATRIX) {
        top.push(m.clone());
    }
    top.push(Entry::of_ints(OP_FONT_BBOX, &font_bbox)?);
    if font_dicts.is_some() {
        top.push(Entry::of_ints(OP_CID_COUNT, &[i64::try_from(glyphs).unwrap_or(0)])?);
    }
    // Widths are explicit against nominalWidthX 0 here; the width optimiser chooses the pair.
    let privates: Vec<Vec<u8>> = flat
        .privates
        .iter()
        .map(|p| {
            let kept: Vec<Entry> = p.iter().filter(|e| !matches!(e.op, OP_SUBRS | OP_VSINDEX | OP_MAXSTACK)).cloned().collect();
            let mut out = Vec::new();
            encode_entries(&kept, &mut out);
            out
        })
        .collect();
    let parts =
        Parts { cff2: false, name: latin1(name), strings: strings.items, top, charset: Some(charset), fd_select, charstrings, font_dicts, privates };
    let table = parts.write()?;
    let (table, _) = fontcraft_cff::subr::optimize_widths_table(&table).map_err(subr_error)?;
    Ok((table, keyed_by_name.is_some()))
}

/// Latin-1 bytes of `s`; other characters become `?`.
fn latin1(s: &str) -> Vec<u8> {
    s.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect()
}

/// The String INDEX being built: standard strings resolve to their SIDs, others are added once.
#[derive(Default)]
struct Strings {
    items: Vec<Vec<u8>>,
    seen: std::collections::HashMap<Vec<u8>, u16>,
}

impl Strings {
    fn sid(&mut self, s: &str) -> Result<u16> {
        let standard = read_fonts::ps::string::STANDARD_STRINGS;
        if let Some(i) = standard.iter().position(|t| *t == s) {
            return u16::try_from(i).map_err(|_| invalid("a standard string index"));
        }
        let bytes = latin1(s);
        if let Some(sid) = self.seen.get(&bytes) {
            return Ok(*sid);
        }
        let sid = u16::try_from(standard.len() + self.items.len())
            .ok()
            .filter(|s| *s <= 64_999)
            .ok_or(TablesError::TooLarge { what: "the CFF strings (use CID-keyed output)", limit: 65_000 })?;
        self.seen.insert(bytes.clone(), sid);
        self.items.push(bytes);
        Ok(sid)
    }
}

fn read_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// A charstring of `(operands, operator)` steps, operands in font units.
#[cfg(test)]
pub(super) fn test_charstring(steps: &[(&[f64], u16)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (args, code) in steps {
        let op = Op { code: *code, args: args.iter().map(|v| (v * ONE as f64).round() as i64).collect(), mask: Vec::new() };
        encode_op(&op, &mut out).unwrap();
    }
    out
}

/// A name-keyed `CFF ` table of these glyphs (`.notdef` first) and charstrings, without
/// subroutines; `top` is DICT data the Top DICT holds besides the offsets.
#[cfg(test)]
pub(super) fn test_table(names: &[&str], charstrings: Vec<Vec<u8>>, top: &[u8]) -> Vec<u8> {
    let mut charset = vec![0u8];
    for i in 1..names.len() {
        charset.extend_from_slice(&(390 + i as u16).to_be_bytes());
    }
    let parts = Parts {
        cff2: false,
        name: b"Test".to_vec(),
        strings: names.iter().skip(1).map(|n| n.as_bytes().to_vec()).collect(),
        top: parse_dict(top, false).unwrap(),
        charset: Some(charset),
        fd_select: None,
        charstrings,
        font_dicts: None,
        // defaultWidthX 0
        privates: vec![vec![139, 20]],
    };
    parts.write().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(code: u16, args: &[i64]) -> Op {
        Op { code, args: args.iter().map(|a| a * ONE).collect(), mask: Vec::new() }
    }

    #[test]
    fn long_operators_split_within_the_cff_stack_and_keep_their_alternation() {
        let args: Vec<i64> = (1..=100).collect();
        // hvcurveto with 25 curves and a final fifth operand: 101 operands.
        let mut with_last = args.clone();
        with_last.push(7);
        let pieces = split_for_stack(op(31, &with_last), 48).unwrap();
        assert!(pieces.iter().all(|p| p.args.len() <= 48 && p.code == 31));
        // Every piece but the last holds an even number of curves.
        for p in pieces.iter().take(pieces.len() - 1) {
            assert_eq!(p.args.len() % 8, 0);
        }
        assert_eq!(pieces.iter().map(|p| p.args.len()).sum::<usize>(), 101);
        let lines = split_for_stack(op(6, &args), 48).unwrap();
        assert_eq!(lines.iter().map(|p| p.args.len()).collect::<Vec<_>>(), vec![48, 48, 4]);
        let rcl: Vec<i64> = (1..=56).collect();
        let pieces = split_for_stack(op(24, &rcl), 48).unwrap();
        assert_eq!(pieces.last().map(|p| (p.code, p.args.len())), Some((5, 2)));
        assert!(split_for_stack(op(1, &args), 48).is_err(), "stems are not split");
    }

    #[test]
    fn the_width_comes_off_the_first_stack_clearing_operator() {
        // 500 (width) 10 20 rmoveto 30 hlineto endchar
        let mut code = Vec::new();
        for v in [500, 10, 20] {
            encode_num(v * ONE, &mut code).unwrap();
        }
        code.push(21);
        encode_num(30 * ONE, &mut code).unwrap();
        code.push(6);
        code.push(14);
        let p = parse_program(&code, false).unwrap();
        assert_eq!(p.width, Some(500 * ONE));
        assert_eq!(p.ops, vec![op(21, &[10, 20]), op(6, &[30])]);
        // seac keeps its operands for the converters; a code past 255 is refused.
        let p = parse_program(&test_charstring(&[(&[600.0, 150.5, 20.0, 65.0, 194.0], 14)]), false).unwrap();
        assert_eq!((p.width, p.seac), (Some(600 * ONE), Some([150 * ONE + ONE / 2, 20 * ONE, 65 * ONE, 194 * ONE])));
        assert!(p.ops.is_empty());
        assert!(parse_program(&test_charstring(&[(&[0.0, 0.0, 65.0, 256.0], 14)]), false).is_err());
    }
}
