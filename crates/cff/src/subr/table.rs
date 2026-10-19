//! Whole `CFF ` and `CFF2` tables: reading their charstrings and subroutines into a
//! [`CharstringSet`], and laying a table out again with new ones.
//!
//! The name, string, charset, encoding, FDSelect and variation store data are copied byte for
//! byte, and every DICT keeps its entries except the offsets and `Subrs`, which are written as
//! five-byte integers so that the DICT sizes are known before the offsets.

use super::{CFF_STACK, CFF2_DEFAULT_STACK, CharstringSet, MAX_FONT_DICTS, MAX_GLYPHS, MAX_STACK_LIMIT, SubrError, SubrOptions};

mod subset;
mod widths;
pub use subset::{SubrMode, TableSubset, subset_table};
pub use widths::{WidthsReport, optimize_widths_table};

/// Most entries one DICT may hold.
const MAX_DICT_ENTRIES: usize = 4_096;
/// Largest table read or written.
const MAX_TABLE: usize = crate::MAX_TABLE_SIZE;

// DICT operators: one byte, or 1200 + the byte after the escape 12.
const OP_CHARSET: u16 = 15;
const OP_ENCODING: u16 = 16;
const OP_CHARSTRINGS: u16 = 17;
const OP_PRIVATE: u16 = 18;
const OP_SUBRS: u16 = 19;
const OP_VSINDEX: u16 = 22;
const OP_VSTORE: u16 = 24;
const OP_MAXSTACK: u16 = 25;
const OP_FDARRAY: u16 = 1236;
const OP_FDSELECT: u16 = 1237;

/// The charstrings and subroutines of a `CFF ` table (`cff2` false) or a `CFF2` table.
pub fn charstrings_of_table(data: &[u8], cff2: bool) -> Result<CharstringSet, SubrError> {
    Ok(Table::read(data, cff2)?.set)
}

/// The table with its charstrings subroutinised ([`super::subroutinize`]).
pub fn subroutinize_table(data: &[u8], cff2: bool, options: &SubrOptions) -> Result<Vec<u8>, SubrError> {
    let table = Table::read(data, cff2)?;
    let set = super::subroutinize(&table.set, options)?;
    table.write(&set)
}

/// The table with every subroutine call expanded and no subroutines left.
pub fn desubroutinize_table(data: &[u8], cff2: bool) -> Result<Vec<u8>, SubrError> {
    let table = Table::read(data, cff2)?;
    let set = super::desubroutinize(&table.set)?;
    table.write(&set)
}

/// [`desubroutinize_table`], refused once the expanded charstrings hold more than `max_bytes`
/// together ([`super::desubroutinize_within`]).
pub fn desubroutinize_table_within(data: &[u8], cff2: bool, max_bytes: usize) -> Result<Vec<u8>, SubrError> {
    let table = Table::read(data, cff2)?;
    let set = super::desubroutinize_within(&table.set, max_bytes)?;
    table.write(&set)
}

fn invalid(what: impl Into<String>) -> SubrError {
    SubrError::Invalid(what.into())
}

/// One DICT entry: its operator, its operand bytes as found, and the operands as integers
/// (`None` for reals and `CFF2` blends).
#[derive(Clone, Debug)]
struct Entry {
    op: u16,
    raw: Vec<u8>,
    ints: Vec<Option<i64>>,
}

impl Entry {
    fn int(&self, i: usize) -> Option<i64> {
        self.ints.get(i).copied().flatten()
    }
}

struct Table<'a> {
    cff2: bool,
    header: &'a [u8],
    /// `CFF `: the Name INDEX and the String INDEX, as found.
    names: &'a [u8],
    strings: &'a [u8],
    top: Vec<Entry>,
    charset: Option<&'a [u8]>,
    encoding: Option<&'a [u8]>,
    fd_select: Option<&'a [u8]>,
    vstore: Option<&'a [u8]>,
    /// The Font DICTs of a CID-keyed or `CFF2` font; `None` for a name-keyed `CFF ` font, whose
    /// one Private DICT hangs off the Top DICT.
    font_dicts: Option<Vec<Vec<Entry>>>,
    /// One per Font DICT, without their `Subrs` entries.
    privates: Vec<Vec<Entry>>,
    set: CharstringSet,
}

impl<'a> Table<'a> {
    fn read(data: &'a [u8], cff2: bool) -> Result<Self, SubrError> {
        if data.len() > MAX_TABLE {
            return Err(SubrError::TooLarge { what: "bytes in a CFF table", limit: MAX_TABLE });
        }
        let major = *data.first().ok_or_else(|| invalid("the CFF table is empty"))?;
        let hdr_size = usize::from(*data.get(2).ok_or_else(|| invalid("the CFF header is cut short"))?);
        if major != if cff2 { 2 } else { 1 } {
            return Err(SubrError::Unsupported(format!("CFF major version {major}")));
        }
        let header = data.get(..hdr_size).filter(|h| h.len() >= if cff2 { 5 } else { 4 }).ok_or_else(|| invalid("the CFF header is cut short"))?;
        let (names, strings, top_bytes, gsubr_at) = if cff2 {
            let top_len = usize::from(read_u16(data, 3).ok_or_else(|| invalid("the CFF2 header is cut short"))?);
            let top = data.get(hdr_size..hdr_size + top_len).ok_or_else(|| invalid("the CFF2 Top DICT lies outside the table"))?;
            (<&[u8]>::default(), <&[u8]>::default(), top, hdr_size + top_len)
        } else {
            let names = read_index(data, hdr_size, false)?;
            let tops = read_index(data, names.end, false)?;
            if tops.items.len() != 1 {
                return Err(SubrError::Unsupported(format!("a CFF table with {} fonts", tops.items.len())));
            }
            let strings = read_index(data, tops.end, false)?;
            let top = tops.items.first().copied().unwrap_or_default();
            (data.get(hdr_size..names.end).unwrap_or_default(), data.get(tops.end..strings.end).unwrap_or_default(), top, strings.end)
        };
        let global_subrs = read_index(data, gsubr_at, cff2)?.items;
        let top = parse_dict(top_bytes, cff2)?;
        let find = |op: u16| top.iter().find(|e| e.op == op);
        let offset = |op: u16| -> Result<Option<usize>, SubrError> {
            match find(op) {
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
        let charset = match offset(OP_CHARSET)? {
            Some(at) if !cff2 && at > 2 => Some(blob(data, at, charset_len(data, at, glyphs)?)?),
            _ => None,
        };
        let encoding = match offset(OP_ENCODING)? {
            Some(at) if !cff2 && at > 1 => Some(blob(data, at, encoding_len(data, at)?)?),
            _ => None,
        };
        let (vstore, regions) = match offset(OP_VSTORE)? {
            Some(at) if cff2 => {
                let len = usize::from(read_u16(data, at).ok_or_else(|| invalid("the variation store is cut short"))?);
                let raw = blob(data, at, len + 2)?;
                (Some(raw), region_counts(raw.get(2..).unwrap_or_default())?)
            }
            _ => (None, Vec::new()),
        };
        let max_stack = if cff2 {
            let v = find(OP_MAXSTACK).and_then(|e| e.int(0)).unwrap_or(CFF2_DEFAULT_STACK as i64);
            usize::try_from(v).ok().filter(|v| (1..=MAX_STACK_LIMIT).contains(v)).ok_or_else(|| invalid(format!("a maxstack of {v}")))?
        } else {
            CFF_STACK
        };

        let (font_dicts, private_refs) = match offset(OP_FDARRAY)? {
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
        let (fd_map, fd_select) = match offset(OP_FDSELECT)? {
            Some(at) => {
                let (map, len) = read_fd_select(data, at, glyphs, cff2)?;
                if map.iter().any(|&fd| usize::from(fd) >= private_refs.len()) {
                    return Err(invalid("the FDSelect names a Font DICT that does not exist"));
                }
                (map, Some(blob(data, at, len)?))
            }
            None => (Vec::new(), None),
        };

        let mut privates = Vec::with_capacity(private_refs.len());
        let mut local_subrs = Vec::with_capacity(private_refs.len());
        let mut fd_vsindex = Vec::with_capacity(private_refs.len());
        for r in private_refs {
            let (size, at) = r.ok_or_else(|| SubrError::Unsupported("a Font DICT without a Private DICT".into()))?;
            let mut entries = parse_dict(blob(data, at, size)?, cff2)?;
            let subrs = match entries.iter().find(|e| e.op == OP_SUBRS).and_then(|e| e.int(0)) {
                Some(rel) => {
                    let rel = usize::try_from(rel).map_err(|_| invalid("a Subrs offset is negative"))?;
                    let subrs_at = at.checked_add(rel).ok_or_else(|| invalid("a Subrs offset overflows"))?;
                    read_index(data, subrs_at, cff2)?.items.into_iter().map(<[u8]>::to_vec).collect()
                }
                None => Vec::new(),
            };
            let vsindex = entries.iter().find(|e| e.op == OP_VSINDEX).and_then(|e| e.int(0)).unwrap_or(0);
            fd_vsindex.push(u16::try_from(vsindex).map_err(|_| invalid(format!("a vsindex of {vsindex}")))?);
            entries.retain(|e| e.op != OP_SUBRS);
            privates.push(entries);
            local_subrs.push(subrs);
        }

        let set = CharstringSet {
            cff2,
            charstrings: charstrings.into_iter().map(<[u8]>::to_vec).collect(),
            global_subrs: global_subrs.into_iter().map(<[u8]>::to_vec).collect(),
            local_subrs,
            fd_select: fd_map,
            fd_vsindex,
            regions,
            max_stack,
        };
        Ok(Table { cff2, header, names, strings, top, charset, encoding, fd_select, vstore, font_dicts, privates, set })
    }

    /// Lays the table out: header, (Name INDEX, Top DICT INDEX, String INDEX) or the `CFF2` Top
    /// DICT, Global Subr INDEX, charset, encoding, variation store, FDSelect, CharStrings,
    /// FDArray, then each Private DICT followed by its Subrs INDEX.
    fn write(&self, set: &CharstringSet) -> Result<Vec<u8>, SubrError> {
        if set.charstrings.len() != self.set.charstrings.len() || set.fd_count() != self.privates.len().max(1) {
            return Err(invalid("the charstrings do not belong to this table"));
        }
        let cff2 = self.cff2;
        let empty = Vec::new();
        let locals: Vec<&Vec<Vec<u8>>> = (0..self.privates.len()).map(|fd| set.local_subrs.get(fd).unwrap_or(&empty)).collect();
        let mut private_bytes = Vec::with_capacity(self.privates.len());
        for (entries, subrs) in self.privates.iter().zip(&locals) {
            let mut bytes = encode_dict(entries.iter(), &[])?;
            if !subrs.is_empty() {
                // The Subrs INDEX follows the Private DICT; its offset counts from the DICT start.
                let rel = bytes.len() + 6;
                push_int29(rel as i64, &mut bytes)?;
                bytes.push(OP_SUBRS as u8);
            }
            private_bytes.push(bytes);
        }
        let local_lens: Vec<usize> = locals.iter().map(|l| if l.is_empty() { 0 } else { index_len(l.iter().map(Vec::len), cff2) }).collect();
        let fdarray_len = match &self.font_dicts {
            Some(dicts) => {
                let lens =
                    dicts.iter().map(|d| encode_dict(d.iter(), &[(OP_PRIVATE, vec![0, 0])]).map(|b| b.len())).collect::<Result<Vec<_>, _>>()?;
                index_len(lens.into_iter(), cff2)
            }
            None => 0,
        };

        let top_len = encode_dict(self.top.iter(), &self.top_overrides(&Layout::default(), &private_bytes))?.len();
        let mut pos = self.header.len();
        if cff2 {
            pos += top_len;
        } else {
            pos += self.names.len() + index_len([top_len].into_iter(), false) + self.strings.len();
        }
        pos += index_len(set.global_subrs.iter().map(Vec::len), cff2);
        let mut at = Layout::default();
        if let Some(c) = self.charset {
            at.charset = pos;
            pos += c.len();
        }
        if let Some(e) = self.encoding {
            at.encoding = pos;
            pos += e.len();
        }
        if let Some(v) = self.vstore {
            at.vstore = pos;
            pos += v.len();
        }
        if let Some(f) = self.fd_select {
            at.fd_select = pos;
            pos += f.len();
        }
        at.charstrings = pos;
        pos += index_len(set.charstrings.iter().map(Vec::len), cff2);
        at.fdarray = pos;
        pos += fdarray_len;
        for (p, l) in private_bytes.iter().zip(&local_lens) {
            at.privates.push(pos);
            pos += p.len() + l;
        }
        if pos > MAX_TABLE {
            return Err(SubrError::TooLarge { what: "bytes in a CFF table", limit: MAX_TABLE });
        }

        let top = encode_dict(self.top.iter(), &self.top_overrides(&at, &private_bytes))?;
        let mut out = Vec::with_capacity(pos);
        out.extend_from_slice(self.header);
        if cff2 {
            let len = u16::try_from(top.len()).map_err(|_| SubrError::TooLarge { what: "bytes in the CFF2 Top DICT", limit: 65_535 })?;
            if let Some(slot) = out.get_mut(3..5) {
                slot.copy_from_slice(&len.to_be_bytes());
            }
            out.extend_from_slice(&top);
        } else {
            if let Some(off_size) = out.get_mut(3) {
                *off_size = 4;
            }
            out.extend_from_slice(self.names);
            write_index(&mut out, std::slice::from_ref(&top), false)?;
            out.extend_from_slice(self.strings);
        }
        write_index(&mut out, &set.global_subrs, cff2)?;
        for part in [self.charset, self.encoding, self.vstore, self.fd_select].into_iter().flatten() {
            out.extend_from_slice(part);
        }
        write_index(&mut out, &set.charstrings, cff2)?;
        if let Some(dicts) = &self.font_dicts {
            let mut encoded = Vec::with_capacity(dicts.len());
            for (i, d) in dicts.iter().enumerate() {
                let size = private_bytes.get(i).map_or(0, Vec::len) as i64;
                let offset = at.privates.get(i).copied().unwrap_or(0) as i64;
                encoded.push(encode_dict(d.iter(), &[(OP_PRIVATE, vec![size, offset])])?);
            }
            write_index(&mut out, &encoded, cff2)?;
        }
        for (p, l) in private_bytes.iter().zip(&locals) {
            out.extend_from_slice(p);
            if !l.is_empty() {
                write_index(&mut out, l, cff2)?;
            }
        }
        if out.len() != pos {
            return Err(invalid("the CFF layout did not come out as planned"));
        }
        Ok(out)
    }

    fn top_overrides(&self, at: &Layout, privates: &[Vec<u8>]) -> Vec<(u16, Vec<i64>)> {
        let mut o = vec![(OP_CHARSTRINGS, vec![at.charstrings as i64])];
        if self.charset.is_some() {
            o.push((OP_CHARSET, vec![at.charset as i64]));
        }
        if self.encoding.is_some() {
            o.push((OP_ENCODING, vec![at.encoding as i64]));
        }
        if self.vstore.is_some() {
            o.push((OP_VSTORE, vec![at.vstore as i64]));
        }
        if self.fd_select.is_some() {
            o.push((OP_FDSELECT, vec![at.fd_select as i64]));
        }
        if self.font_dicts.is_some() {
            o.push((OP_FDARRAY, vec![at.fdarray as i64]));
        } else {
            let size = privates.first().map_or(0, Vec::len) as i64;
            o.push((OP_PRIVATE, vec![size, at.privates.first().copied().unwrap_or(0) as i64]));
        }
        o
    }
}

#[derive(Default)]
struct Layout {
    charset: usize,
    encoding: usize,
    vstore: usize,
    fd_select: usize,
    charstrings: usize,
    fdarray: usize,
    privates: Vec<usize>,
}

/// The Private DICT a Top or Font DICT points at: `(size, offset)`.
fn private_ref(entries: &[Entry]) -> Result<Option<(usize, usize)>, SubrError> {
    let Some(e) = entries.iter().find(|e| e.op == OP_PRIVATE) else { return Ok(None) };
    let (Some(size), Some(at)) = (e.int(0), e.int(1)) else { return Err(invalid("a Private entry needs a size and an offset")) };
    let size = usize::try_from(size).map_err(|_| invalid("a negative Private DICT size"))?;
    let at = usize::try_from(at).map_err(|_| invalid("a negative Private DICT offset"))?;
    Ok(Some((size, at)))
}

fn blob(data: &[u8], at: usize, len: usize) -> Result<&[u8], SubrError> {
    at.checked_add(len).and_then(|end| data.get(at..end)).ok_or_else(|| invalid("a CFF structure lies outside the table"))
}

struct Index<'a> {
    items: Vec<&'a [u8]>,
    end: usize,
}

/// The INDEX at `at`: a 16-bit count in `CFF `, a 32-bit count in `CFF2`.
fn read_index(data: &[u8], at: usize, cff2: bool) -> Result<Index<'_>, SubrError> {
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

fn index_len(lens: impl Iterator<Item = usize>, cff2: bool) -> usize {
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

fn off_size(last: usize) -> usize {
    match last {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    }
}

fn write_index<T: AsRef<[u8]>>(out: &mut Vec<u8>, items: &[T], cff2: bool) -> Result<(), SubrError> {
    let count = items.len();
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
    let total: usize = items.iter().map(|i| i.as_ref().len()).sum();
    let size = off_size(total + 1);
    out.push(size as u8);
    let put = |out: &mut Vec<u8>, v: usize| {
        let bytes = (v as u32).to_be_bytes();
        out.extend_from_slice(bytes.get(4 - size..).unwrap_or_default());
    };
    let mut offset = 1usize;
    put(out, offset);
    for item in items {
        offset += item.as_ref().len();
        put(out, offset);
    }
    for item in items {
        out.extend_from_slice(item.as_ref());
    }
    Ok(())
}

/// The DICT entries of `bytes`. In `CFF2`, `blend` (23) belongs to the operands of the entry it
/// appears in.
fn parse_dict(bytes: &[u8], cff2: bool) -> Result<Vec<Entry>, SubrError> {
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
                entries.push(Entry { op, raw: std::mem::take(&mut raw), ints: std::mem::take(&mut ints) });
            }
            _ => return Err(invalid(format!("byte {b0} is reserved in a DICT"))),
        }
    }
    if !raw.is_empty() {
        return Err(invalid("a DICT ends with operands and no operator"));
    }
    Ok(entries)
}

fn push_int29(v: i64, out: &mut Vec<u8>) -> Result<(), SubrError> {
    let v = i32::try_from(v).map_err(|_| invalid("a DICT offset does not fit in 32 bits"))?;
    out.push(29);
    out.extend_from_slice(&v.to_be_bytes());
    Ok(())
}

/// The entries as DICT bytes; an operator in `overrides` gets those operands, each a five-byte
/// integer, in place of its own.
fn encode_dict<'e>(entries: impl Iterator<Item = &'e Entry>, overrides: &[(u16, Vec<i64>)]) -> Result<Vec<u8>, SubrError> {
    let mut out = Vec::new();
    for e in entries {
        match overrides.iter().find(|(op, _)| *op == e.op) {
            Some((_, values)) => {
                for &v in values {
                    push_int29(v, &mut out)?;
                }
            }
            None => out.extend_from_slice(&e.raw),
        }
        if e.op >= 1200 {
            out.extend_from_slice(&[12, (e.op - 1200) as u8]);
        } else {
            out.push(e.op as u8);
        }
    }
    Ok(out)
}

/// Bytes of a custom charset for `glyphs` glyphs (`.notdef` has no entry).
fn charset_len(data: &[u8], at: usize, glyphs: usize) -> Result<usize, SubrError> {
    let cut = || invalid("the charset is cut short");
    let format = *data.get(at).ok_or_else(cut)?;
    let wanted = glyphs.saturating_sub(1);
    match format {
        0 => Ok(1 + 2 * wanted),
        1 | 2 => {
            let range_len = if format == 1 { 3 } else { 4 };
            let mut covered = 0usize;
            let mut len = 1usize;
            while covered < wanted {
                let p = at + len;
                let left =
                    if format == 1 { usize::from(*data.get(p + 2).ok_or_else(cut)?) } else { usize::from(read_u16(data, p + 2).ok_or_else(cut)?) };
                covered += left + 1;
                len += range_len;
            }
            Ok(len)
        }
        other => Err(invalid(format!("charset format {other}"))),
    }
}

/// Bytes of a custom encoding.
fn encoding_len(data: &[u8], at: usize) -> Result<usize, SubrError> {
    let cut = || invalid("the encoding is cut short");
    let format = *data.get(at).ok_or_else(cut)?;
    let n = usize::from(*data.get(at + 1).ok_or_else(cut)?);
    let mut len = match format & 0x7F {
        0 => 2 + n,
        1 => 2 + 2 * n,
        other => return Err(invalid(format!("encoding format {other}"))),
    };
    if format & 0x80 != 0 {
        let sups = usize::from(*data.get(at + len).ok_or_else(cut)?);
        len += 1 + 3 * sups;
    }
    Ok(len)
}

/// The Font DICT of every glyph, and the FDSelect's length in bytes.
fn read_fd_select(data: &[u8], at: usize, glyphs: usize, cff2: bool) -> Result<(Vec<u16>, usize), SubrError> {
    let cut = || invalid("the FDSelect is cut short");
    let format = *data.get(at).ok_or_else(cut)?;
    match format {
        0 => {
            let map = data.get(at + 1..at + 1 + glyphs).ok_or_else(cut)?.iter().map(|b| u16::from(*b)).collect();
            Ok((map, 1 + glyphs))
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
            Ok((map, p + sentinel - at))
        }
        other => Err(invalid(format!("FDSelect format {other}"))),
    }
}

/// The region count of each item variation data of a variation store.
fn region_counts(store: &[u8]) -> Result<Vec<usize>, SubrError> {
    let cut = || invalid("the variation store is cut short");
    let count = usize::from(read_u16(store, 6).ok_or_else(cut)?);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = read_u32(store, 8 + 4 * i).ok_or_else(cut)? as usize;
        out.push(usize::from(read_u16(store, at.checked_add(4).ok_or_else(cut)?).ok_or_else(cut)?));
    }
    Ok(out)
}

fn read_u16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?]))
}
