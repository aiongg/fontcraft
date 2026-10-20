//! The byte-level pieces of `CFF ` and `CFF2` tables that the readers and writers share: INDEX
//! structures, FDSelect, DICT entries and numbers, charstring numbers, operators and offsets, the
//! charset of a CID-keyed table, and splitting a path operator into generic segments.
//!
//! Tools that pass charstrings through instead of building them from outlines (the subroutiniser,
//! the variable-font instancer) lay their tables out with these. Layout rules follow fontTools
//! `cffLib` (MIT, Copyright (c) 2017 Just van Rossum and contributors).
//!
//! Readers return [`SubrError`]: `Invalid` for malformed data, `TooLarge` past a limit.

use crate::subr::SubrError;

pub use crate::number::{encode_dict_int, encode_dict_value, encode_t2_number_exact};

/// Most bytes one real number in a DICT may take.
pub const MAX_REAL_BYTES: usize = 32;
/// Most entries one DICT may hold.
pub const MAX_DICT_ENTRIES: usize = 4_096;

fn invalid(what: impl Into<String>) -> SubrError {
    SubrError::Invalid(what.into())
}

/// The big-endian `u16` at `at`.
pub fn read_u16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
}

/// The big-endian `u32` at `at`.
pub fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?]))
}

/// The `len` bytes at `at`.
pub fn blob(data: &[u8], at: usize, len: usize) -> Result<&[u8], SubrError> {
    at.checked_add(len).and_then(|end| data.get(at..end)).ok_or_else(|| invalid("a CFF structure lies outside the table"))
}

// ---------- INDEX ----------

/// An INDEX as read: its items and where the bytes after it start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index<'a> {
    pub items: Vec<&'a [u8]>,
    pub end: usize,
}

/// The INDEX at `at`: a 16-bit count in `CFF `, a 32-bit count in `CFF2`.
pub fn read_index(data: &[u8], at: usize, cff2: bool) -> Result<Index<'_>, SubrError> {
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
    let read_off = |i: usize| -> Result<usize, SubrError> {
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

/// The smallest offset size (1 to 4 bytes) that holds `last`.
pub fn off_size(last: usize) -> usize {
    match last {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    }
}

/// The bytes of an INDEX whose items have these lengths.
pub fn index_len(lens: impl Iterator<Item = usize>, cff2: bool) -> usize {
    let head = if cff2 { 4 } else { 2 };
    let (mut count, mut total) = (0usize, 0usize);
    for l in lens {
        count += 1;
        total = total.saturating_add(l);
    }
    if count == 0 {
        return head;
    }
    head + 1 + (count + 1) * off_size(total + 1) + total
}

/// Appends an INDEX of `items`: offsets start at 1, the offset size is the smallest that holds
/// the last offset, an empty INDEX is its count alone.
pub fn write_index<I>(out: &mut Vec<u8>, items: I, cff2: bool) -> Result<(), SubrError>
where
    I: IntoIterator + Clone,
    I::Item: AsRef<[u8]>,
{
    let count = items.clone().into_iter().count();
    if cff2 {
        let c = u32::try_from(count).map_err(|_| SubrError::TooLarge { what: "items in an INDEX", limit: u32::MAX as usize })?;
        out.extend_from_slice(&c.to_be_bytes());
    } else {
        let c = u16::try_from(count).map_err(|_| SubrError::TooLarge { what: "items in an INDEX", limit: 65_535 })?;
        out.extend_from_slice(&c.to_be_bytes());
    }
    if count == 0 {
        return Ok(());
    }
    let total: usize = items.clone().into_iter().map(|i| i.as_ref().len()).sum();
    let size = off_size(total + 1);
    out.push(size as u8);
    let put = |out: &mut Vec<u8>, v: usize| {
        let bytes = (v as u32).to_be_bytes();
        out.extend_from_slice(bytes.get(4 - size..).unwrap_or_default());
    };
    let mut offset = 1usize;
    put(out, offset);
    for item in items.clone() {
        offset += item.as_ref().len();
        put(out, offset);
    }
    for item in items {
        out.extend_from_slice(item.as_ref());
    }
    Ok(())
}

// ---------- FDSelect ----------

/// An FDSelect as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FdSelect {
    /// The Font DICT of every glyph.
    pub map: Vec<u16>,
    /// 0, 3 or (`CFF2`) 4.
    pub format: u8,
    /// Its length in bytes.
    pub len: usize,
}

/// The FDSelect at `at` for `glyphs` glyphs. Format 4 is read only in `CFF2`.
pub fn read_fd_select(data: &[u8], at: usize, glyphs: usize, cff2: bool) -> Result<FdSelect, SubrError> {
    let cut = || invalid("the FDSelect is cut short");
    let format = *data.get(at).ok_or_else(cut)?;
    match format {
        0 => {
            let map = data.get(at + 1..at + 1 + glyphs).ok_or_else(cut)?.iter().map(|b| u16::from(*b)).collect();
            Ok(FdSelect { map, format, len: 1 + glyphs })
        }
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
            let sentinel = if wide { 4 } else { 2 };
            Ok(FdSelect { map, format, len: p + sentinel - at })
        }
        other => Err(invalid(format!("FDSelect format {other}"))),
    }
}

/// The FDSelect format [`write_fd_select`] writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FdSelectFormat {
    Format0,
    Format3,
    /// `CFF2` only: 32-bit glyph ids and 16-bit Font DICT indexes.
    Format4,
    /// The smaller of format 0 and format 3. On a tie, format 0 when `zero_on_tie`, as the static
    /// CID writer chooses; format 3 otherwise, as fontTools does for a table it rebuilds.
    Smaller {
        zero_on_tie: bool,
    },
}

/// The FDSelect of `map` (the Font DICT of every glyph).
pub fn write_fd_select(map: &[u16], format: FdSelectFormat, cff2: bool) -> Result<Vec<u8>, SubrError> {
    let f0 = || {
        let mut v = vec![0u8];
        v.extend(map.iter().map(|f| *f as u8));
        v
    };
    let mut ranges: Vec<(usize, u16)> = Vec::new();
    for (g, fd) in map.iter().enumerate() {
        if ranges.last().is_none_or(|(_, f)| f != fd) {
            ranges.push((g, *fd));
        }
    }
    if map.iter().any(|f| *f > 255) && (!cff2 || format != FdSelectFormat::Format4) {
        return Err(SubrError::Unsupported("more than 256 Font DICTs in a CFF table".into()));
    }
    let f3 = || -> Result<Vec<u8>, SubrError> {
        let mut v = vec![3u8];
        v.extend_from_slice(&u16::try_from(ranges.len()).map_err(|_| invalid("too many FDSelect ranges"))?.to_be_bytes());
        for (first, fd) in &ranges {
            v.extend_from_slice(&u16::try_from(*first).map_err(|_| invalid("an FDSelect glyph id does not fit 16 bits"))?.to_be_bytes());
            v.push(*fd as u8);
        }
        v.extend_from_slice(&u16::try_from(map.len()).map_err(|_| invalid("too many glyphs for FDSelect"))?.to_be_bytes());
        Ok(v)
    };
    match format {
        FdSelectFormat::Format0 => Ok(f0()),
        FdSelectFormat::Format3 => f3(),
        FdSelectFormat::Format4 => {
            let wide = |v: usize| u32::try_from(v).map_err(|_| invalid("too many glyphs for FDSelect"));
            let mut v = vec![4u8];
            v.extend_from_slice(&wide(ranges.len())?.to_be_bytes());
            for (first, fd) in &ranges {
                v.extend_from_slice(&wide(*first)?.to_be_bytes());
                v.extend_from_slice(&fd.to_be_bytes());
            }
            v.extend_from_slice(&wide(map.len())?.to_be_bytes());
            Ok(v)
        }
        FdSelectFormat::Smaller { zero_on_tie } => {
            let (a, b) = (f0(), f3()?);
            let zero = if zero_on_tie { a.len() <= b.len() } else { a.len() < b.len() };
            Ok(if zero { a } else { b })
        }
    }
}

/// The charset of a CID-keyed table whose glyph `g` has CID `g`, for `glyphs` glyphs: the smaller
/// of format 0 and one range, the range on a tie (`.notdef` alone gives an empty format 1), as
/// fontTools packs it.
pub fn cid_charset(glyphs: usize) -> Vec<u8> {
    let cids: Vec<u16> = (1..glyphs).map(|c| u16::try_from(c).unwrap_or(u16::MAX)).collect();
    let (format0, ranges) = (crate::charset::format0(&cids), crate::charset::ranges(&cids));
    if format0.len() < ranges.len() { format0 } else { ranges }
}

// ---------- DICTs ----------

/// One DICT entry: its operator (one byte, or 1200 plus the byte after the escape 12), its
/// operand bytes as found, and the operands as integers (`None` for reals and `CFF2` blends).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictEntry {
    pub op: u16,
    pub raw: Vec<u8>,
    pub ints: Vec<Option<i64>>,
}

impl DictEntry {
    /// Operand `i` when it is an integer.
    pub fn int(&self, i: usize) -> Option<i64> {
        self.ints.get(i).copied().flatten()
    }
}

/// The entries of a DICT. In `CFF2`, `blend` (23) belongs to the operands of the entry it appears
/// in. Reals are skipped to their end nibble, not read.
pub fn parse_dict(bytes: &[u8], cff2: bool) -> Result<Vec<DictEntry>, SubrError> {
    let cut = || invalid("a DICT is cut short");
    let mut entries = Vec::new();
    let mut raw = Vec::new();
    let mut ints = Vec::new();
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
                    return Err(SubrError::TooLarge { what: "entries in a DICT", limit: MAX_DICT_ENTRIES });
                }
                entries.push(DictEntry { op, raw: std::mem::take(&mut raw), ints: std::mem::take(&mut ints) });
            }
            _ => return Err(invalid(format!("byte {b0} is reserved in a DICT"))),
        }
    }
    if !raw.is_empty() {
        return Err(invalid("a DICT ends with operands and no operator"));
    }
    Ok(entries)
}

/// The DICT number at `i`, reals included, and where the next token starts. A real longer than
/// [`MAX_REAL_BYTES`] or holding the reserved nibble is refused.
pub fn read_dict_number(bytes: &[u8], i: usize) -> Result<(f64, usize), SubrError> {
    let cut = || invalid("a DICT is cut short");
    let b0 = *bytes.get(i).ok_or_else(cut)?;
    Ok(match b0 {
        28 => (f64::from(read_u16(bytes, i + 1).ok_or_else(cut)? as i16), i + 3),
        29 => (f64::from(read_u32(bytes, i + 1).ok_or_else(cut)? as i32), i + 5),
        30 => {
            let mut text = String::new();
            let mut j = i + 1;
            'outer: loop {
                if j - i > MAX_REAL_BYTES {
                    return Err(invalid("a DICT real number is too long"));
                }
                let b = *bytes.get(j).ok_or_else(cut)?;
                j += 1;
                for nib in [b >> 4, b & 0xF] {
                    match nib {
                        0..=9 => text.push(char::from(b'0' + nib)),
                        0xA => text.push('.'),
                        0xB => text.push('E'),
                        0xC => text.push_str("E-"),
                        0xE => text.push('-'),
                        0xF => break 'outer,
                        _ => return Err(invalid("a DICT real number holds a reserved nibble")),
                    }
                }
            }
            let v: f64 = text.parse().map_err(|_| invalid(format!("the DICT real number {text} cannot be read")))?;
            if !v.is_finite() {
                return Err(invalid("a DICT real number is not finite"));
            }
            (v, j)
        }
        32..=246 => (f64::from(b0) - 139.0, i + 1),
        247..=250 => (f64::from((i32::from(b0) - 247) * 256 + i32::from(*bytes.get(i + 1).ok_or_else(cut)?) + 108), i + 2),
        251..=254 => (f64::from(-(i32::from(b0) - 251) * 256 - i32::from(*bytes.get(i + 1).ok_or_else(cut)?) - 108), i + 2),
        _ => return Err(invalid(format!("byte {b0} does not start a DICT number"))),
    })
}

// ---------- operators, offsets, charstring numbers ----------

/// The bytes of a DICT or charstring operator (1200 and up are escaped).
pub fn operator_len(op: u16) -> usize {
    if op >= 1200 { 2 } else { 1 }
}

/// Appends a DICT or charstring operator (1200 and up as 12 and the second byte).
pub fn push_operator(out: &mut Vec<u8>, op: u16) {
    if op >= 1200 {
        out.extend_from_slice(&[12, (op - 1200) as u8]);
    } else {
        out.push(op as u8);
    }
}

/// Appends a DICT offset as a five-byte integer, so its size is known before its value.
pub fn push_offset(out: &mut Vec<u8>, v: usize) -> Result<(), SubrError> {
    let v = i32::try_from(v).map_err(|_| invalid("a DICT offset does not fit in 32 bits"))?;
    out.push(29);
    out.extend_from_slice(&v.to_be_bytes());
    Ok(())
}

/// The charstring number at `i` (16.16 fixed values divided out), and where the next token
/// starts; `None` when `code[i]` is an operator byte or the number is cut short.
pub fn read_t2_number(code: &[u8], i: usize) -> Option<(f64, usize)> {
    let b0 = *code.get(i)?;
    Some(match b0 {
        28 => (f64::from(read_u16(code, i + 1)? as i16), i + 3),
        32..=246 => (f64::from(b0) - 139.0, i + 1),
        247..=254 => {
            let b1 = f64::from(*code.get(i + 1)?);
            let v = if b0 <= 250 { (f64::from(b0) - 247.0) * 256.0 + b1 + 108.0 } else { -(f64::from(b0) - 251.0) * 256.0 - b1 - 108.0 };
            (v, i + 2)
        }
        255 => (f64::from(read_u32(code, i + 1)? as i32) / 65536.0, i + 5),
        _ => return None,
    })
}

/// Whether path operator `op` (the operator byte, or 1200 plus the byte after the escape) can
/// take `n` operands by the charstring specification; any other operator passes.
pub fn operands_fit(op: u16, n: usize) -> bool {
    match op {
        21 => n == 2,
        4 | 22 => n == 1,
        5 => n >= 2 && n.is_multiple_of(2),
        6 | 7 => n >= 1,
        8 => n >= 6 && n.is_multiple_of(6),
        24 => n >= 8 && (n - 2).is_multiple_of(6),
        // One curve with no line before it is accepted, as fontTools' pen is.
        25 => n >= 6 && n.is_multiple_of(2),
        26 | 27 | 30 | 31 => n >= 4 && n % 4 <= 1,
        1234 => n == 7,
        1235 => n == 13,
        1236 => n == 9,
        1237 => n == 11,
        _ => true,
    }
}

// ---------- generic segments ----------

/// A relative path segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RelSeg {
    Line(f64, f64),
    Curve([f64; 6]),
}

/// The relative segments of path operator `op` with operands `a`; `None` for an operator that
/// draws no segments (moves, hints, flex).
pub fn generalize(op: u16, a: &[f64]) -> Result<Option<Vec<RelSeg>>, SubrError> {
    let g = |i: usize| a.get(i).copied().unwrap_or(0.0);
    let wrong = || invalid(format!("operator {op} has {} operands", a.len()));
    let mut out = Vec::new();
    match op {
        5 => {
            if !a.len().is_multiple_of(2) {
                return Err(wrong());
            }
            out.extend(a.chunks(2).map(|c| RelSeg::Line(c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0))));
        }
        6 | 7 => {
            let mut horizontal = op == 6;
            for v in a {
                out.push(if horizontal { RelSeg::Line(*v, 0.0) } else { RelSeg::Line(0.0, *v) });
                horizontal = !horizontal;
            }
        }
        8 => {
            if !a.len().is_multiple_of(6) {
                return Err(wrong());
            }
            out.extend(a.chunks(6).map(|c| RelSeg::Curve(std::array::from_fn(|i| c.get(i).copied().unwrap_or(0.0)))));
        }
        24 => {
            if a.len() < 8 || !(a.len() - 2).is_multiple_of(6) {
                return Err(wrong());
            }
            let curves = (a.len() - 2) / 6;
            for k in 0..curves {
                out.push(RelSeg::Curve(std::array::from_fn(|i| g(6 * k + i))));
            }
            out.push(RelSeg::Line(g(a.len() - 2), g(a.len() - 1)));
        }
        25 => {
            if a.len() < 6 || !(a.len() - 6).is_multiple_of(2) {
                return Err(wrong());
            }
            let lines = (a.len() - 6) / 2;
            for k in 0..lines {
                out.push(RelSeg::Line(g(2 * k), g(2 * k + 1)));
            }
            out.push(RelSeg::Curve(std::array::from_fn(|i| g(2 * lines + i))));
        }
        26 | 27 => {
            let (first, rest) = if a.len() % 4 == 1 { (g(0), 1) } else { (0.0, 0) };
            if !(a.len() - rest).is_multiple_of(4) || a.len() < 4 {
                return Err(wrong());
            }
            for (k, c) in a.get(rest..).unwrap_or_default().chunks(4).enumerate() {
                let c = |i: usize| c.get(i).copied().unwrap_or(0.0);
                let f = if k == 0 { first } else { 0.0 };
                out.push(if op == 26 { RelSeg::Curve([f, c(0), c(1), c(2), 0.0, c(3)]) } else { RelSeg::Curve([c(0), f, c(1), c(2), c(3), 0.0]) });
            }
        }
        30 | 31 => {
            if a.len() < 4 || (!a.len().is_multiple_of(4) && a.len() % 4 != 1) {
                return Err(wrong());
            }
            let m = a.len() / 4;
            let tail = if a.len() % 4 == 1 { g(a.len() - 1) } else { 0.0 };
            for k in 0..m {
                let c = |i: usize| g(4 * k + i);
                let extra = if k + 1 == m { tail } else { 0.0 };
                let horizontal = (k % 2 == 0) == (op == 31);
                out.push(if horizontal {
                    RelSeg::Curve([c(0), 0.0, c(1), c(2), extra, c(3)])
                } else {
                    RelSeg::Curve([0.0, c(0), c(1), c(2), c(3), extra])
                });
            }
        }
        _ => return Ok(None),
    }
    Ok(Some(out))
}

/// Appends segments as `rlineto` and `rrcurveto` runs of at most `max_stack` operands, numbers
/// written by [`encode_t2_number_exact`].
pub fn write_segments(out: &mut Vec<u8>, segs: &[RelSeg], max_stack: usize) -> Result<(), crate::Error> {
    let what = "a path segment";
    let mut i = 0usize;
    while let Some(first) = segs.get(i) {
        let is_line = matches!(first, RelSeg::Line(..));
        let per = if is_line { 2 } else { 6 };
        let mut count = 0usize;
        while let Some(s) = segs.get(i) {
            if matches!(s, RelSeg::Line(..)) != is_line || (count + 1) * per > max_stack {
                break;
            }
            match s {
                RelSeg::Line(x, y) => {
                    encode_t2_number_exact(*x, what, out)?;
                    encode_t2_number_exact(*y, what, out)?;
                }
                RelSeg::Curve(c) => {
                    for v in c {
                        encode_t2_number_exact(*v, what, out)?;
                    }
                }
            }
            count += 1;
            i += 1;
        }
        if count == 0 {
            return Err(crate::Error::TooManyOperands { what: what.to_string(), count: per });
        }
        out.push(if is_line { 5 } else { 8 });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_charstring_numbers_keep_near_integers_fixed() {
        let enc = |v: f64| {
            let mut out = Vec::new();
            encode_t2_number_exact(v, "test", &mut out).unwrap();
            out
        };
        // Whole values take the short integer forms.
        assert_eq!(enc(0.0), [139]);
        assert_eq!(enc(-1131.0), [254, 255]);
        assert_eq!(enc(32767.0), [28, 0x7F, 0xFF]);
        // A value a quarter of a 16.16 unit off a whole number stays fixed; the builder's encoding
        // rounds it to an integer.
        let near = 3.0 + 1.0 / 262_144.0;
        assert_eq!(enc(near), [255, 0, 3, 0, 0]);
        let mut built = Vec::new();
        crate::number::encode_t2_number(near, "test", &mut built).unwrap();
        assert_eq!(built, [142]);
        // A negative half in 16.16 units rounds away from zero.
        assert_eq!(enc(-0.5 / 65_536.0), [255, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(encode_t2_number_exact(f64::NAN, "test", &mut Vec::new()).is_err());
        assert!(encode_t2_number_exact(40_000.0, "test", &mut Vec::new()).is_err());
    }

    #[test]
    fn blend_runs_fill_the_stack_when_asked() {
        use crate::cff2::{Arg, OperandStyle, write_operands};
        let args: Vec<Arg> = [(1.0, 10.0), (2.0, 20.0), (3.0, 30.0)].into_iter().map(|(d, x)| Arg::Blend(d, vec![x])).collect();
        let write = |fill_stack: bool| {
            let mut out = Vec::new();
            let style = OperandStyle { dict: false, max_stack: 7, fill_stack, exact_numbers: true };
            write_operands(&args, 1, style, "test", &mut out).unwrap();
            out
        };
        // Three values need 3 * 2 + 1 = 7 slots: one blend when the run may fill the stack, two
        // when it must leave a slot free.
        assert_eq!(write(true), [140, 141, 142, 149, 159, 169, 142, 16]);
        assert_eq!(write(false), [140, 141, 149, 159, 141, 16, 142, 169, 140, 16]);
    }

    #[test]
    fn fd_select_ties_follow_the_option() {
        // Seven glyphs in one Font DICT: format 0 takes 1 + 7 bytes, format 3 takes 1 + 2 + 3 + 2.
        let map = vec![0u16; 7];
        assert_eq!(write_fd_select(&map, FdSelectFormat::Smaller { zero_on_tie: true }, false).unwrap().first(), Some(&0));
        assert_eq!(write_fd_select(&map, FdSelectFormat::Smaller { zero_on_tie: false }, false).unwrap().first(), Some(&3));
        let wide = write_fd_select(&[0, 300], FdSelectFormat::Format4, true).unwrap();
        let read = read_fd_select(&wide, 0, 2, true).unwrap();
        assert_eq!((read.map, read.format, read.len), (vec![0, 300], 4, wide.len()));
        assert!(write_fd_select(&[0, 300], FdSelectFormat::Format3, false).is_err());
    }
}
