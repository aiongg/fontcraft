//! Rewriting a `CFF2` table with a new glyph order, or with some charstrings replaced.
//!
//! The table is cut into its parts (header, Top DICT, global subroutines, CharStrings, FDSelect,
//! FDArray, the Private DICTs with their local subroutines, and the variation store), then laid
//! out again in that order: the charstrings and FDSelect follow the new order, and every offset
//! in the Top DICT, the Font DICTs and the Private DICTs is written as a five-byte integer and
//! recomputed. Everything else, the subroutines and the variation store included, is copied
//! byte for byte, so a variable table stays variable.

use std::collections::HashMap;

use super::be::{Data, put_u16, put_u32, to_u16, to_u32};
use crate::limits::{MAX_CFF2_STACK, MAX_GLYPHS, MAX_SFNT};
use crate::{Result, TablesError};

const TAG: [u8; 4] = *b"CFF2";
const CHARSTRINGS: u16 = 17;
const PRIVATE: u16 = 18;
const SUBRS: u16 = 19;
const VSTORE: u16 = 24;
const FDARRAY: u16 = 1236;
const FDSELECT: u16 = 1237;
/// Most Font DICTs: FDSelect formats 0 and 3 store a Font DICT's index in one byte.
const MAX_FONT_DICTS: usize = 256;
/// Most entries one DICT may hold.
const MAX_DICT_ENTRIES: usize = 4_096;

/// One DICT entry: its operator and its operand bytes as written (a `blend` among the operands
/// stays in them).
#[derive(Clone, Debug)]
struct Entry {
    op: u16,
    raw: Vec<u8>,
    /// The last operand, when it is an integer: the offset of an offset entry.
    last: Option<i64>,
    /// The operand before the last, when it is an integer: the size of `Private`.
    before_last: Option<i64>,
}

#[derive(Clone, Debug)]
struct Private<'a> {
    dict: Vec<Entry>,
    subrs: Option<&'a [u8]>,
}

/// A parsed `CFF2` table.
pub(crate) struct Cff2<'a> {
    header: &'a [u8],
    top: Vec<Entry>,
    gsubrs: &'a [u8],
    charstrings: Vec<&'a [u8]>,
    font_dicts: Vec<Vec<Entry>>,
    privates: Vec<Private<'a>>,
    /// The Font DICT of every glyph; `None` when the table has no FDSelect.
    fd_select: Option<Vec<u16>>,
    /// The variation store with its two-byte length in front.
    vstore: Option<&'a [u8]>,
}

/// A CFF2 INDEX at `at`: its items and the offset just past it.
fn index<'a>(d: Data<'a>, at: usize, max_count: usize) -> Result<(Vec<&'a [u8]>, usize)> {
    let count = d.u32_usize(at)?;
    if count == 0 {
        return Ok((Vec::new(), at.saturating_add(4)));
    }
    if count > max_count {
        return Err(TablesError::TooLarge { what: "the items of a CFF2 INDEX", limit: max_count });
    }
    let off_size = usize::from(d.u8(at.saturating_add(4))?);
    if !(1..=4).contains(&off_size) {
        return Err(d.bad(format!("has an INDEX with offset size {off_size}")));
    }
    let offsets_at = at.saturating_add(5);
    let read = |i: usize| -> Result<usize> {
        let s = d.slice(offsets_at.saturating_add(i.saturating_mul(off_size)), off_size)?;
        Ok(s.iter().fold(0usize, |acc, b| acc << 8 | usize::from(*b)))
    };
    // Offsets count from 1, from the byte before the data.
    let base = offsets_at.saturating_add(count.saturating_add(1).saturating_mul(off_size)).saturating_sub(1);
    let mut items = Vec::with_capacity(count.min(d.bytes.len()));
    let mut prev = read(0)?;
    if prev != 1 {
        return Err(d.bad("has an INDEX whose first offset is not 1"));
    }
    for i in 1..=count {
        let next = read(i)?;
        if next < prev {
            return Err(d.bad("has an INDEX with decreasing offsets"));
        }
        items.push(d.slice(base.saturating_add(prev), next - prev)?);
        prev = next;
    }
    Ok((items, base.saturating_add(prev)))
}

/// Writes a CFF2 INDEX with the smallest offset size.
fn write_index(items: &[&[u8]]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    put_u32(&mut out, to_u32(items.len(), "a CFF2 INDEX's item count")?);
    if items.is_empty() {
        return Ok(out);
    }
    let total: usize = items.iter().map(|i| i.len()).sum::<usize>() + 1;
    let off_size: usize = match total {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    out.push(off_size as u8);
    let put = |out: &mut Vec<u8>, v: usize| -> Result<()> {
        let bytes = to_u32(v, "a CFF2 INDEX offset")?.to_be_bytes();
        out.extend_from_slice(bytes.get(4 - off_size..).unwrap_or_default());
        Ok(())
    };
    let mut offset = 1usize;
    put(&mut out, offset)?;
    for item in items {
        offset += item.len();
        put(&mut out, offset)?;
    }
    for item in items {
        out.extend_from_slice(item);
    }
    Ok(out)
}

/// The entries of a CFF2 DICT. `blend` (23) stays among the operands of the entry it ends in.
fn dict(d: Data<'_>, bytes: &[u8]) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let mut raw: Vec<u8> = Vec::new();
    let mut values: Vec<Option<i64>> = Vec::new();
    let mut i = 0usize;
    let byte = |i: usize| bytes.get(i).copied().ok_or_else(|| d.bad("has a DICT that is cut short"));
    while i < bytes.len() {
        let b0 = byte(i)?;
        let (len, value): (usize, Option<i64>) = match b0 {
            23 => (1, None),
            0..=24 => {
                let op = if b0 == 12 { 1200 + u16::from(byte(i + 1)?) } else { u16::from(b0) };
                i += if b0 == 12 { 2 } else { 1 };
                if out.len() >= MAX_DICT_ENTRIES {
                    return Err(TablesError::TooLarge { what: "the entries of a CFF2 DICT", limit: MAX_DICT_ENTRIES });
                }
                let last = values.last().copied().flatten();
                let before_last = values.len().checked_sub(2).and_then(|k| values.get(k).copied().flatten());
                out.push(Entry { op, raw: std::mem::take(&mut raw), last, before_last });
                values.clear();
                continue;
            }
            28 => (3, Some(i64::from(i16::from_be_bytes([byte(i + 1)?, byte(i + 2)?])))),
            29 => (5, Some(i64::from(i32::from_be_bytes([byte(i + 1)?, byte(i + 2)?, byte(i + 3)?, byte(i + 4)?])))),
            30 => {
                let mut n = 1;
                loop {
                    let b = byte(i + n)?;
                    n += 1;
                    if b & 0x0F == 0x0F || b >> 4 == 0x0F {
                        break;
                    }
                }
                (n, None)
            }
            32..=246 => (1, Some(i64::from(b0) - 139)),
            247..=250 => (2, Some((i64::from(b0) - 247) * 256 + i64::from(byte(i + 1)?) + 108)),
            251..=254 => (2, Some(-(i64::from(b0) - 251) * 256 - i64::from(byte(i + 1)?) - 108)),
            _ => return Err(d.bad(format!("has a DICT with the reserved byte {b0}"))),
        };
        if values.len() >= MAX_CFF2_STACK {
            return Err(TablesError::TooLarge { what: "the operands of a DICT entry", limit: MAX_CFF2_STACK });
        }
        raw.extend_from_slice(bytes.get(i..i.saturating_add(len)).ok_or_else(|| d.bad("has a DICT that is cut short"))?);
        values.push(value);
        i += len;
    }
    Ok(out)
}

fn find(entries: &[Entry], op: u16) -> Option<&Entry> {
    entries.iter().find(|e| e.op == op)
}

fn offset(d: Data<'_>, value: Option<i64>, op: u16) -> Result<usize> {
    let v = value.ok_or_else(|| d.bad(format!("has a DICT operator {op} without an integer offset")))?;
    usize::try_from(v).map_err(|_| d.bad("has a negative offset"))
}

fn int29(v: usize, out: &mut Vec<u8>) -> Result<()> {
    let v = i32::try_from(v).map_err(|_| TablesError::TooLarge { what: "a CFF2 offset", limit: i32::MAX as usize })?;
    out.push(29);
    out.extend_from_slice(&v.to_be_bytes());
    Ok(())
}

/// Encodes `entries`, giving the operators in `replace` the operands listed there (as
/// five-byte integers); replaced operators the entries lack are appended.
fn encode(entries: &[Entry], replace: &[(u16, Vec<usize>)]) -> Result<Vec<u8>> {
    let put_op = |out: &mut Vec<u8>, op: u16| {
        if op >= 1200 {
            out.extend_from_slice(&[12, (op - 1200) as u8]);
        } else {
            out.push(op as u8);
        }
    };
    let mut out = Vec::new();
    for e in entries {
        match replace.iter().find(|(op, _)| *op == e.op) {
            Some((_, values)) => {
                for v in values {
                    int29(*v, &mut out)?;
                }
            }
            None => out.extend_from_slice(&e.raw),
        }
        put_op(&mut out, e.op);
    }
    for (op, values) in replace {
        if find(entries, *op).is_none() {
            for v in values {
                int29(*v, &mut out)?;
            }
            put_op(&mut out, *op);
        }
    }
    Ok(out)
}

fn read_fd_select(d: Data<'_>, at: usize, n: usize) -> Result<Vec<u16>> {
    match d.u8(at)? {
        0 => Ok(d.slice(at.saturating_add(1), n)?.iter().map(|b| u16::from(*b)).collect()),
        format @ (3 | 4) => {
            // Format 3: u16 ranges, u16 first glyph, u8 Font DICT; format 4: u32 and u32, u16.
            let wide = format == 4;
            let (count_len, first_len, fd_len) = if wide { (4, 4, 2) } else { (2, 2, 1) };
            let ranges = if wide { d.u32_usize(at.saturating_add(1))? } else { usize::from(d.u16(at.saturating_add(1))?) };
            if ranges > n {
                return Err(d.bad("has more FDSelect ranges than glyphs"));
            }
            let record = first_len + fd_len;
            let start = at.saturating_add(1 + count_len);
            let first_of = |r: usize| -> Result<usize> {
                let p = start.saturating_add(r.saturating_mul(record));
                if wide { d.u32_usize(p) } else { Ok(usize::from(d.u16(p)?)) }
            };
            let mut out = vec![0u16; n];
            for r in 0..ranges {
                let first = first_of(r)?;
                let p = start.saturating_add(r * record).saturating_add(first_len);
                let fd = if wide { d.u16(p)? } else { u16::from(d.u8(p)?) };
                let end = first_of(r + 1)?.min(n);
                for slot in out.iter_mut().take(end).skip(first) {
                    *slot = fd;
                }
            }
            Ok(out)
        }
        f => Err(d.bad(format!("has an FDSelect of format {f}"))),
    }
}

/// FDSelect format 3 (ranges) or 0, whichever is smaller.
fn write_fd_select(fds: &[u16]) -> Result<Vec<u8>> {
    let mut ranges: Vec<(usize, u16)> = Vec::new();
    for (g, fd) in fds.iter().enumerate() {
        if ranges.last().is_none_or(|(_, last)| last != fd) {
            ranges.push((g, *fd));
        }
    }
    let byte = |fd: u16| u8::try_from(fd).map_err(|_| TablesError::TooLarge { what: "a Font DICT index", limit: MAX_FONT_DICTS - 1 });
    let mut out = Vec::new();
    if fds.len() < 5 + 3 * ranges.len() {
        out.push(0);
        for fd in fds {
            out.push(byte(*fd)?);
        }
    } else {
        out.push(3);
        put_u16(&mut out, to_u16(ranges.len(), "the FDSelect ranges")?);
        for (first, fd) in ranges {
            put_u16(&mut out, to_u16(first, "a glyph id")?);
            out.push(byte(fd)?);
        }
        put_u16(&mut out, to_u16(fds.len(), "the glyph count")?);
    }
    Ok(out)
}

/// Parses `bytes` as a `CFF2` table of a font with `num_glyphs` glyphs.
pub(crate) fn parse(bytes: &[u8], num_glyphs: usize) -> Result<Cff2<'_>> {
    let d = Data::new(TAG, bytes);
    if d.u8(0)? != 2 {
        return Err(TablesError::Unsupported(format!("CFF2 major version {}", d.u8(0)?)));
    }
    let hdr = usize::from(d.u8(2)?);
    if hdr < 5 {
        return Err(d.bad("has a header shorter than 5 bytes"));
    }
    let header = d.slice(0, hdr)?;
    let top_len = usize::from(d.u16(3)?);
    let top = dict(d, d.slice(hdr, top_len)?)?;
    let gsubrs_at = hdr + top_len;
    let (_, gsubrs_end) = index(d, gsubrs_at, MAX_SFNT)?;
    let gsubrs = d.slice(gsubrs_at, gsubrs_end.saturating_sub(gsubrs_at))?;

    let cs = find(&top, CHARSTRINGS).ok_or_else(|| d.bad("has no CharStrings"))?;
    let (charstrings, _) = index(d, offset(d, cs.last, CHARSTRINGS)?, MAX_GLYPHS)?;
    if charstrings.len() != num_glyphs {
        return Err(d.bad(format!("has {} charstrings for {num_glyphs} glyphs", charstrings.len())));
    }
    let fda = find(&top, FDARRAY).ok_or_else(|| d.bad("has no FDArray"))?;
    let (dicts, _) = index(d, offset(d, fda.last, FDARRAY)?, MAX_FONT_DICTS)?;
    if dicts.is_empty() {
        return Err(d.bad("has an empty FDArray"));
    }
    let mut font_dicts = Vec::with_capacity(dicts.len());
    let mut privates = Vec::with_capacity(dicts.len());
    for fd in dicts {
        let entries = dict(d, fd)?;
        let p = find(&entries, PRIVATE).ok_or_else(|| d.bad("has a Font DICT without a Private DICT"))?;
        let size = offset(d, p.before_last, PRIVATE)?;
        let at = offset(d, p.last, PRIVATE)?;
        let private = dict(d, d.slice(at, size)?)?;
        let subrs = match find(&private, SUBRS) {
            Some(s) => {
                let start = at.checked_add(offset(d, s.last, SUBRS)?).ok_or_else(|| d.bad("has a Subrs offset past the end"))?;
                let (_, end) = index(d, start, MAX_SFNT)?;
                Some(d.slice(start, end.saturating_sub(start))?)
            }
            None => None,
        };
        privates.push(Private { dict: private, subrs });
        font_dicts.push(entries);
    }
    let fd_select = match find(&top, FDSELECT) {
        Some(e) => {
            let fds = read_fd_select(d, offset(d, e.last, FDSELECT)?, num_glyphs)?;
            if fds.iter().any(|fd| usize::from(*fd) >= font_dicts.len()) {
                return Err(d.bad("has an FDSelect entry past the FDArray"));
            }
            Some(fds)
        }
        None if font_dicts.len() > 1 => return Err(d.bad("has several Font DICTs but no FDSelect")),
        None => None,
    };
    let vstore = match find(&top, VSTORE) {
        Some(e) => {
            let at = offset(d, e.last, VSTORE)?;
            let len = usize::from(d.u16(at)?);
            Some(d.slice(at, len.saturating_add(2))?)
        }
        None => None,
    };
    Ok(Cff2 { header, top, gsubrs, charstrings, font_dicts, privates, fd_select, vstore })
}

impl Cff2<'_> {
    /// The Font DICT of every glyph.
    pub(crate) fn font_dict_of_glyphs(&self) -> Vec<u16> {
        self.fd_select.clone().unwrap_or_else(|| vec![0; self.charstrings.len()])
    }

    /// The number of Font DICTs.
    pub(crate) fn font_dict_count(&self) -> usize {
        self.font_dicts.len()
    }

    /// The byte length of every charstring.
    pub(crate) fn charstring_lengths(&self) -> Vec<usize> {
        self.charstrings.iter().map(|c| c.len()).collect()
    }

    /// The table with glyph `new_to_old[i]` at index `i`, and the charstrings of `replace` (by
    /// new glyph id) in place of the old ones.
    pub(crate) fn rebuild(&self, new_to_old: &[u16], replace: &HashMap<usize, Vec<u8>>) -> Result<Vec<u8>> {
        let old = |g: usize| usize::from(new_to_old.get(g).copied().unwrap_or(0));
        let n = new_to_old.len();
        let charstrings: Vec<&[u8]> =
            (0..n).map(|g| replace.get(&g).map(Vec::as_slice).unwrap_or_else(|| self.charstrings.get(old(g)).copied().unwrap_or_default())).collect();
        let cs_index = write_index(&charstrings)?;
        let fd_select_bytes = match &self.fd_select {
            Some(fds) => write_fd_select(&(0..n).map(|g| fds.get(old(g)).copied().unwrap_or(0)).collect::<Vec<_>>())?,
            None => Vec::new(),
        };
        // Every offset is a five-byte integer, so the sizes are known before the offsets.
        let privates_sized: Vec<Vec<u8>> = self.privates.iter().map(encode_private).collect::<Result<_>>()?;
        let private_spans: Vec<usize> = privates_sized.iter().zip(&self.privates).map(|(b, p)| b.len() + p.subrs.map_or(0, <[u8]>::len)).collect();
        let fd_dummy: Vec<Vec<u8>> = self.font_dicts.iter().map(|fd| encode(fd, &[(PRIVATE, vec![0, 0])])).collect::<Result<_>>()?;
        let fd_index_len = write_index(&fd_dummy.iter().map(Vec::as_slice).collect::<Vec<_>>())?.len();
        let top_replace = |cs: usize, fds: usize, fda: usize, vstore: usize| {
            let mut r = vec![(CHARSTRINGS, vec![cs]), (FDARRAY, vec![fda])];
            if self.fd_select.is_some() {
                r.push((FDSELECT, vec![fds]));
            }
            if self.vstore.is_some() {
                r.push((VSTORE, vec![vstore]));
            }
            r
        };
        let top_len = encode(&self.top, &top_replace(0, 0, 0, 0))?.len();

        let mut pos = self.header.len() + top_len + self.gsubrs.len();
        let cs_at = pos;
        pos += cs_index.len();
        let fd_select_at = pos;
        pos += fd_select_bytes.len();
        let fd_array_at = pos;
        pos += fd_index_len;
        let mut private_at = Vec::with_capacity(private_spans.len());
        for span in &private_spans {
            private_at.push(pos);
            pos += span;
        }
        let vstore_at = pos;
        pos += self.vstore.map_or(0, <[u8]>::len);
        if pos > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the rewritten CFF2 table", limit: MAX_SFNT });
        }

        let top = encode(&self.top, &top_replace(cs_at, fd_select_at, fd_array_at, vstore_at))?;
        let mut header = self.header.to_vec();
        if let Some(slot) = header.get_mut(3..5) {
            slot.copy_from_slice(&to_u16(top.len(), "the CFF2 Top DICT")?.to_be_bytes());
        }
        let mut out = Vec::with_capacity(pos);
        out.extend_from_slice(&header);
        out.extend_from_slice(&top);
        out.extend_from_slice(self.gsubrs);
        out.extend(cs_index);
        out.extend(fd_select_bytes);
        let fds: Vec<Vec<u8>> = self
            .font_dicts
            .iter()
            .enumerate()
            .map(|(i, fd)| encode(fd, &[(PRIVATE, vec![privates_sized.get(i).map_or(0, Vec::len), private_at.get(i).copied().unwrap_or(0)])]))
            .collect::<Result<_>>()?;
        out.extend(write_index(&fds.iter().map(Vec::as_slice).collect::<Vec<_>>())?);
        for (bytes, p) in privates_sized.iter().zip(&self.privates) {
            out.extend_from_slice(bytes);
            if let Some(subrs) = p.subrs {
                out.extend_from_slice(subrs);
            }
        }
        if let Some(vstore) = self.vstore {
            out.extend_from_slice(vstore);
        }
        if out.len() != pos {
            return Err(TablesError::invalid("the CFF2 layout did not come out as planned"));
        }
        Ok(out)
    }
}

/// A Private DICT whose local subroutines follow it directly.
fn encode_private(p: &Private<'_>) -> Result<Vec<u8>> {
    if p.subrs.is_none() {
        return encode(&p.dict, &[]);
    }
    let len = encode(&p.dict, &[(SUBRS, vec![0])])?.len();
    encode(&p.dict, &[(SUBRS, vec![len])])
}
