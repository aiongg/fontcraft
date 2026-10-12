//! CFF and CFF2 tables without hints (M12.27).
//!
//! Every charstring is run with its subroutine calls followed and written out flat, with the
//! stems, hint masks, counter masks and `dotsection` of the selected glyphs left out and their
//! flex turned into plain curves. The width stays where the format expects it: when the first
//! stack-clearing operator was a removed stem or mask, its width operand moves to the next
//! operator written. The table is laid out again without subroutines; the name, string, charset,
//! encoding, FDSelect and variation store data are copied byte for byte, and every DICT keeps its
//! entries except the offsets, `Subrs` and (when font-wide data goes) the Private DICT hint values
//! pyftsubset deletes.

use super::DehintOptions;
use crate::limits::{MAX_CFF_STACK, MAX_CFF2_STACK, MAX_CHARSTRING, MAX_SFNT, MAX_SUBR_NESTING};
use crate::{Result, TablesError};

/// Most operators one glyph may run, subroutine calls followed.
const MAX_OPS_PER_GLYPH: usize = 1 << 20;
/// Most entries one DICT may hold.
const MAX_DICT_ENTRIES: usize = 4_096;
/// Most Font DICTs (an FDSelect index of CFF is one byte; CFF2 allows more but no font needs it).
const MAX_FONT_DICTS: usize = 256;
/// Most glyphs (CharStrings INDEX entries).
const MAX_GLYPHS: usize = 65_535;

// DICT operators: one byte, or 1200 + the byte after the escape 12.
const OP_CHARSET: u16 = 15;
const OP_ENCODING: u16 = 16;
const OP_CHARSTRINGS: u16 = 17;
const OP_PRIVATE: u16 = 18;
const OP_SUBRS: u16 = 19;
const OP_VSINDEX: u16 = 22;
const OP_VSTORE: u16 = 24;
const OP_FDARRAY: u16 = 1236;
const OP_FDSELECT: u16 = 1237;

/// The Private DICT values pyftsubset deletes with the hints: BlueValues, OtherBlues,
/// FamilyBlues, FamilyOtherBlues, StdHW, StdVW, BlueScale, BlueShift, BlueFuzz, StemSnapH,
/// StemSnapV, ForceBold, LanguageGroup, ExpansionFactor.
const PRIVATE_HINT_OPS: [u16; 14] = [6, 7, 8, 9, 10, 11, 1209, 1210, 1211, 1212, 1213, 1214, 1217, 1218];

/// The CFF or CFF2 table without hints, and the number of glyphs that lost some; `None` when
/// nothing would change.
pub(super) fn dehint(data: &[u8], cff2: bool, options: &DehintOptions) -> Result<Option<(Vec<u8>, usize)>> {
    let table = Table::read(data, cff2)?;
    let strip_private = options.font_wide() && !options.keep_font_hints;

    let mut charstrings = Vec::with_capacity(table.charstrings.len());
    let mut glyphs_changed = 0usize;
    for (gid, code) in table.charstrings.iter().enumerate() {
        let fd = table.fd_select.as_ref().and_then(|s| s.get(gid).copied()).unwrap_or(0);
        let private =
            table.privates.get(usize::from(fd)).ok_or_else(|| invalid(format!("glyph {gid} names Font DICT {fd}, which does not exist")))?;
        let mut flat = Flat::new(&table, private, options.selected(gid), options.keep_flex);
        flat.run(code, 0).map_err(|e| in_glyph(e, gid))?;
        flat.finish().map_err(|e| in_glyph(e, gid))?;
        if flat.removed {
            glyphs_changed += 1;
        }
        charstrings.push(flat.out);
    }
    let private_changed = strip_private && table.privates.iter().any(|p| p.entries.iter().any(|e| PRIVATE_HINT_OPS.contains(&e.op)));
    if glyphs_changed == 0 && !private_changed {
        return Ok(None);
    }

    let privates: Vec<Vec<u8>> = table
        .privates
        .iter()
        .map(|p| {
            let kept: Vec<&Entry> = p.entries.iter().filter(|e| e.op != OP_SUBRS && !(strip_private && PRIVATE_HINT_OPS.contains(&e.op))).collect();
            encode_dict(kept.into_iter(), &[])
        })
        .collect::<Result<_>>()?;
    let bytes = table.write(&charstrings, &privates)?;
    Ok(Some((bytes, glyphs_changed)))
}

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

// ---------- reading ----------

/// One DICT entry: its operator, its operand bytes as found, and the operands as integers
/// (`None` for reals and CFF2 blends).
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

struct Private<'a> {
    entries: Vec<Entry>,
    subrs: Vec<&'a [u8]>,
    vsindex: usize,
}

struct Table<'a> {
    cff2: bool,
    /// The header bytes (`hdrSize` of them).
    header: &'a [u8],
    /// CFF: the Name INDEX and the String INDEX, as found.
    names: &'a [u8],
    strings: &'a [u8],
    top: Vec<Entry>,
    charset: Option<&'a [u8]>,
    encoding: Option<&'a [u8]>,
    fd_select_raw: Option<&'a [u8]>,
    vstore: Option<&'a [u8]>,
    /// Region counts of the variation store's item variation data, by `vsindex`.
    regions: Vec<usize>,
    global_subrs: Vec<&'a [u8]>,
    charstrings: Vec<&'a [u8]>,
    /// The Font DICTs of a CID-keyed or CFF2 font; `None` for a name-keyed CFF font, whose one
    /// Private DICT hangs off the Top DICT.
    font_dicts: Option<Vec<Vec<Entry>>>,
    fd_select: Option<Vec<u16>>,
    privates: Vec<Private<'a>>,
}

impl<'a> Table<'a> {
    fn read(data: &'a [u8], cff2: bool) -> Result<Self> {
        let major = *data.first().ok_or_else(|| invalid("the CFF table is empty"))?;
        let hdr_size = usize::from(*data.get(2).ok_or_else(|| invalid("the CFF header is cut short"))?);
        let expected = if cff2 { 2 } else { 1 };
        if major != expected {
            return Err(TablesError::Unsupported(format!("CFF major version {major}")));
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
                return Err(TablesError::Unsupported(format!("a CFF table with {} fonts", tops.items.len())));
            }
            let strings = read_index(data, tops.end, false)?;
            let top = tops.items.first().copied().unwrap_or_default();
            let names_raw = data.get(hdr_size..names.end).unwrap_or_default();
            let strings_raw = data.get(tops.end..strings.end).unwrap_or_default();
            (names_raw, strings_raw, top, strings.end)
        };
        let global_subrs = read_index(data, gsubr_at, cff2)?.items;
        let top = parse_dict(top_bytes, cff2)?;
        let find = |op: u16| top.iter().find(|e| e.op == op);
        let offset = |op: u16| -> Result<Option<usize>> {
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
        let (fd_select, fd_select_raw) = match offset(OP_FDSELECT)? {
            Some(at) => {
                let (map, len) = read_fd_select(data, at, glyphs, cff2)?;
                (Some(map), Some(blob(data, at, len)?))
            }
            None => (None, None),
        };

        let mut privates = Vec::with_capacity(private_refs.len());
        for r in private_refs {
            let (entries, subrs, vsindex) = match r {
                None => (Vec::new(), Vec::new(), 0),
                Some((size, at)) => {
                    let bytes = blob(data, at, size)?;
                    let entries = parse_dict(bytes, cff2)?;
                    let subrs = match entries.iter().find(|e| e.op == OP_SUBRS).and_then(|e| e.int(0)) {
                        Some(rel) => {
                            let rel = usize::try_from(rel).map_err(|_| invalid("a Subrs offset is negative"))?;
                            read_index(data, at.checked_add(rel).ok_or_else(|| invalid("a Subrs offset overflows"))?, cff2)?.items
                        }
                        None => Vec::new(),
                    };
                    let vsindex = entries.iter().find(|e| e.op == OP_VSINDEX).and_then(|e| e.int(0)).unwrap_or(0);
                    let vsindex = usize::try_from(vsindex).map_err(|_| invalid("a negative vsindex"))?;
                    (entries, subrs, vsindex)
                }
            };
            privates.push(Private { entries, subrs, vsindex });
        }

        Ok(Table {
            cff2,
            header,
            names,
            strings,
            top,
            charset,
            encoding,
            fd_select_raw,
            vstore,
            regions,
            global_subrs,
            charstrings,
            font_dicts,
            fd_select,
            privates,
        })
    }

    /// Lays the table out again: header, (Name INDEX, Top DICT INDEX, String INDEX) or the CFF2
    /// Top DICT, an empty Global Subr INDEX, charset, encoding, variation store, FDSelect,
    /// CharStrings, FDArray, Private DICTs. Every offset is a five-byte integer, so the DICT
    /// sizes are known before the offsets are.
    fn write(&self, charstrings: &[Vec<u8>], privates: &[Vec<u8>]) -> Result<Vec<u8>> {
        let empty_index_len = if self.cff2 { 4 } else { 2 };
        let cs_len = index_len(charstrings.iter().map(Vec::len), self.cff2)?;
        let fd_lens: Option<Vec<usize>> = match &self.font_dicts {
            Some(dicts) => Some(
                dicts
                    .iter()
                    .zip(privates)
                    .map(|(d, p)| encode_dict(d.iter(), &[(OP_PRIVATE, vec![p.len() as i64, 0])]).map(|b| b.len()))
                    .collect::<Result<_>>()?,
            ),
            None => None,
        };
        let fdarray_len = match &fd_lens {
            Some(lens) => index_len(lens.iter().copied(), self.cff2)?,
            None => 0,
        };

        // First pass with zero offsets gives the Top DICT size.
        let zero = Layout::default();
        let top_len = encode_dict(self.top.iter(), &self.top_overrides(&zero, privates))?.len();
        let mut pos = self.header.len();
        if self.cff2 {
            pos += top_len;
        } else {
            pos += self.names.len() + index_len([top_len].into_iter(), false)? + self.strings.len();
        }
        pos += empty_index_len;
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
        if let Some(f) = self.fd_select_raw {
            at.fd_select = pos;
            pos += f.len();
        }
        at.charstrings = pos;
        pos += cs_len;
        at.fdarray = pos;
        pos += fdarray_len;
        for p in privates {
            at.privates.push(pos);
            pos += p.len();
        }
        if pos > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the CFF table", limit: MAX_SFNT });
        }

        let top = encode_dict(self.top.iter(), &self.top_overrides(&at, privates))?;
        let mut out = Vec::with_capacity(pos);
        out.extend_from_slice(self.header);
        if self.cff2 {
            let len = u16::try_from(top.len()).map_err(|_| TablesError::TooLarge { what: "the CFF2 Top DICT", limit: 65_535 })?;
            if let Some(slot) = out.get_mut(3..5) {
                slot.copy_from_slice(&len.to_be_bytes());
            }
            out.extend_from_slice(&top);
        } else {
            if let Some(off_size) = out.get_mut(3) {
                *off_size = 4;
            }
            out.extend_from_slice(self.names);
            write_index(&mut out, [top.as_slice()].into_iter(), false)?;
            out.extend_from_slice(self.strings);
        }
        write_index(&mut out, std::iter::empty::<&[u8]>(), self.cff2)?;
        for part in [self.charset, self.encoding, self.vstore, self.fd_select_raw].into_iter().flatten() {
            out.extend_from_slice(part);
        }
        write_index(&mut out, charstrings.iter().map(Vec::as_slice), self.cff2)?;
        if let Some(dicts) = &self.font_dicts {
            let mut encoded = Vec::with_capacity(dicts.len());
            for (i, d) in dicts.iter().enumerate() {
                let size = privates.get(i).map_or(0, Vec::len) as i64;
                let offset = at.privates.get(i).copied().unwrap_or(0) as i64;
                encoded.push(encode_dict(d.iter(), &[(OP_PRIVATE, vec![size, offset])])?);
            }
            write_index(&mut out, encoded.iter().map(Vec::as_slice), self.cff2)?;
        }
        for p in privates {
            out.extend_from_slice(p);
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
        if self.fd_select_raw.is_some() {
            o.push((OP_FDSELECT, vec![at.fd_select as i64]));
        }
        if self.font_dicts.is_some() {
            o.push((OP_FDARRAY, vec![at.fdarray as i64]));
        } else if self.top.iter().any(|e| e.op == OP_PRIVATE) {
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
    /// Where the INDEX ends.
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

fn index_len(lens: impl Iterator<Item = usize> + Clone, cff2: bool) -> Result<usize> {
    let head = if cff2 { 4 } else { 2 };
    let count = lens.clone().count();
    if count == 0 {
        return Ok(head);
    }
    let total: usize = lens.sum();
    let off_size = off_size(total + 1);
    Ok(head + 1 + (count + 1) * off_size + total)
}

fn off_size(last: usize) -> usize {
    match last {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    }
}

fn write_index<'b>(out: &mut Vec<u8>, items: impl Iterator<Item = &'b [u8]> + Clone, cff2: bool) -> Result<()> {
    let count = items.clone().count();
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
    let total: usize = items.clone().map(<[u8]>::len).sum();
    let size = off_size(total + 1);
    out.push(size as u8);
    let mut offset = 1usize;
    let put = |out: &mut Vec<u8>, v: usize| {
        let bytes = (v as u32).to_be_bytes();
        out.extend_from_slice(bytes.get(4 - size..).unwrap_or_default());
    };
    put(out, offset);
    for item in items.clone() {
        offset += item.len();
        put(out, offset);
    }
    for item in items {
        out.extend_from_slice(item);
    }
    Ok(())
}

/// The DICT entries of `bytes`. In CFF2, `blend` (23) belongs to the operands of the entry it
/// appears in.
fn parse_dict(bytes: &[u8], cff2: bool) -> Result<Vec<Entry>> {
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
                    return Err(TablesError::TooLarge { what: "a DICT's entry count", limit: MAX_DICT_ENTRIES });
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

/// The entries as DICT bytes; an operator in `overrides` gets those operands, each a five-byte
/// integer, in place of its own.
fn encode_dict<'e>(entries: impl Iterator<Item = &'e Entry>, overrides: &[(u16, Vec<i64>)]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for e in entries {
        match overrides.iter().find(|(op, _)| *op == e.op) {
            Some((_, values)) => {
                for v in values {
                    let v = i32::try_from(*v).map_err(|_| invalid("a DICT offset does not fit in 32 bits"))?;
                    out.push(29);
                    out.extend_from_slice(&v.to_be_bytes());
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
fn charset_len(data: &[u8], at: usize, glyphs: usize) -> Result<usize> {
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
fn encoding_len(data: &[u8], at: usize) -> Result<usize> {
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
fn read_fd_select(data: &[u8], at: usize, glyphs: usize, cff2: bool) -> Result<(Vec<u16>, usize)> {
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
            let len = p + sentinel - at;
            Ok((map, len))
        }
        other => Err(invalid(format!("FDSelect format {other}"))),
    }
}

/// The region count of each item variation data of a variation store.
fn region_counts(store: &[u8]) -> Result<Vec<usize>> {
    let cut = || invalid("the variation store is cut short");
    let count = usize::from(read_u16(store, 6).ok_or_else(cut)?);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = read_u32(store, 8 + 4 * i).ok_or_else(cut)? as usize;
        out.push(usize::from(read_u16(store, at.checked_add(4).ok_or_else(cut)?).ok_or_else(cut)?));
    }
    Ok(out)
}

// ---------- charstrings ----------

/// An operand on the charstring stack: a number in 16.16 fixed point, or the `blend` operator of
/// CFF2, which stays among the operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    Num(i64),
    Blend,
}

const ONE: i64 = 1 << 16;

/// Runs a charstring with its subroutine calls followed and writes it out flat.
struct Flat<'t> {
    cff2: bool,
    strip: bool,
    keep_flex: bool,
    global: &'t [&'t [u8]],
    local: &'t [&'t [u8]],
    regions: &'t [usize],
    vsindex: usize,
    /// The operands since the last operator, `blend` included.
    pending: Vec<Tok>,
    /// The number of values `pending` leaves on the stack.
    depth: usize,
    stems: usize,
    /// CFF: the first stack-clearing operator has been seen, so the width is settled.
    width_settled: bool,
    /// CFF: the width of a removed first stem or mask, written before the next operator.
    width: Option<Tok>,
    out: Vec<u8>,
    ops: usize,
    /// Something was removed: a stem, a mask, `dotsection` or flex.
    removed: bool,
    ended: bool,
}

enum Flow {
    Go,
    Return,
}

impl<'t> Flat<'t> {
    fn new(table: &'t Table<'t>, private: &'t Private<'t>, strip: bool, keep_flex: bool) -> Self {
        Flat {
            cff2: table.cff2,
            strip,
            keep_flex,
            global: &table.global_subrs,
            local: &private.subrs,
            regions: &table.regions,
            vsindex: private.vsindex,
            pending: Vec::new(),
            depth: 0,
            stems: 0,
            width_settled: false,
            width: None,
            out: Vec::new(),
            ops: 0,
            removed: false,
            ended: false,
        }
    }

    fn run(&mut self, code: &[u8], level: usize) -> Result<()> {
        if level > MAX_SUBR_NESTING {
            return Err(invalid(format!("subroutines nest deeper than {MAX_SUBR_NESTING}")));
        }
        let cut = || invalid("a charstring is cut short");
        let mut i = 0usize;
        while let Some(&b0) = code.get(i) {
            if self.ended {
                break;
            }
            self.ops += 1;
            if self.ops > MAX_OPS_PER_GLYPH {
                return Err(TablesError::TooLarge { what: "the operators one glyph runs", limit: MAX_OPS_PER_GLYPH });
            }
            match b0 {
                28 => {
                    let v = read_u16(code, i + 1).ok_or_else(cut)? as i16;
                    self.push(Tok::Num(i64::from(v) * ONE))?;
                    i += 3;
                }
                32..=246 => {
                    self.push(Tok::Num((i64::from(b0) - 139) * ONE))?;
                    i += 1;
                }
                247..=254 => {
                    let b1 = i64::from(*code.get(i + 1).ok_or_else(cut)?);
                    let v = if b0 <= 250 { (i64::from(b0) - 247) * 256 + b1 + 108 } else { -(i64::from(b0) - 251) * 256 - b1 - 108 };
                    self.push(Tok::Num(v * ONE))?;
                    i += 2;
                }
                255 => {
                    let v = read_u32(code, i + 1).ok_or_else(cut)? as i32;
                    self.push(Tok::Num(i64::from(v)))?;
                    i += 5;
                }
                12 => {
                    let b1 = *code.get(i + 1).ok_or_else(cut)?;
                    i += 2;
                    self.escape(b1)?;
                }
                _ => {
                    i += 1;
                    if let Flow::Return = self.operator(b0, code, &mut i, level)? {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if !self.pending.is_empty() {
            return Err(invalid("operands are left at the end of the charstring"));
        }
        if !self.cff2 && !self.ended {
            return Err(invalid("the charstring has no endchar"));
        }
        Ok(())
    }

    fn push(&mut self, t: Tok) -> Result<()> {
        self.pending.push(t);
        self.depth += 1;
        let limit = if self.cff2 { MAX_CFF2_STACK } else { MAX_CFF_STACK };
        if self.depth > limit || self.pending.len() > MAX_CFF2_STACK * 2 {
            return Err(invalid(format!("more than {limit} operands on the stack")));
        }
        Ok(())
    }

    fn pop_int(&mut self) -> Result<i64> {
        match self.pending.pop() {
            Some(Tok::Num(v)) if v % ONE == 0 => {
                self.depth = self.depth.saturating_sub(1);
                Ok(v / ONE)
            }
            _ => Err(invalid("an operator needs an integer operand")),
        }
    }

    /// CFF: when this is the first stack-clearing operator and `has_width` says the operand
    /// count carries a width, sets the width aside if the operator is removed.
    fn settle_width(&mut self, has_width: bool, removing: bool) {
        if self.cff2 || self.width_settled {
            return;
        }
        self.width_settled = true;
        if has_width && removing && !self.pending.is_empty() {
            self.width = Some(self.pending.remove(0));
            self.depth = self.depth.saturating_sub(1);
        }
    }

    fn emit(&mut self, op: &[u8]) -> Result<()> {
        if let Some(w) = self.width.take() {
            encode_tok(w, &mut self.out)?;
        }
        for t in std::mem::take(&mut self.pending) {
            encode_tok(t, &mut self.out)?;
        }
        self.depth = 0;
        self.out.extend_from_slice(op);
        if self.out.len() > MAX_CHARSTRING {
            return Err(TablesError::TooLarge { what: "a charstring without subroutines", limit: MAX_CHARSTRING });
        }
        Ok(())
    }

    fn drop_operands(&mut self) {
        self.pending.clear();
        self.depth = 0;
        self.removed = true;
    }

    fn operator(&mut self, op: u8, code: &[u8], i: &mut usize, level: usize) -> Result<Flow> {
        match op {
            // hstem, vstem, hstemhm, vstemhm
            1 | 3 | 18 | 23 => {
                self.settle_width(self.depth % 2 == 1, self.strip);
                self.stems += self.depth / 2;
                if self.strip { self.drop_operands() } else { self.emit(&[op])? }
            }
            // hintmask, cntrmask: operands before them are an implicit vstem.
            19 | 20 => {
                self.settle_width(self.depth % 2 == 1, self.strip);
                self.stems += self.depth / 2;
                let n = self.stems.div_ceil(8);
                let mask = code.get(*i..*i + n).ok_or_else(|| invalid("a hint mask is cut short"))?;
                *i += n;
                if self.strip {
                    self.drop_operands();
                } else {
                    let mut bytes = Vec::with_capacity(n + 1);
                    bytes.push(op);
                    bytes.extend_from_slice(mask);
                    self.emit(&bytes)?;
                }
            }
            10 | 29 => {
                let v = self.pop_int()?;
                let subrs = if op == 10 { self.local } else { self.global };
                let bias: i64 = match subrs.len() {
                    0..1240 => 107,
                    1240..33900 => 1131,
                    _ => 32768,
                };
                let sub = usize::try_from(v + bias)
                    .ok()
                    .and_then(|k| subrs.get(k).copied())
                    .ok_or_else(|| invalid(format!("subroutine {v} does not exist")))?;
                self.run(sub, level + 1)?;
            }
            11 if !self.cff2 => return Ok(Flow::Return),
            14 if !self.cff2 => {
                self.settle_width(self.depth == 1 || self.depth == 5, self.strip);
                self.emit(&[14])?;
                self.ended = true;
                return Ok(Flow::Return);
            }
            // vmoveto, hmoveto
            4 | 22 => {
                self.settle_width(self.depth == 2, self.strip);
                self.emit(&[op])?;
            }
            21 => {
                self.settle_width(self.depth == 3, self.strip);
                self.emit(&[op])?;
            }
            5 | 6 | 7 | 8 | 24 | 25 | 26 | 27 | 30 | 31 => self.emit(&[op])?,
            15 if self.cff2 => {
                let v = self.pop_int()?;
                self.vsindex = usize::try_from(v).map_err(|_| invalid("a negative vsindex"))?;
                self.pending.push(Tok::Num(v * ONE));
                self.emit(&[15])?;
            }
            16 if self.cff2 => {
                let n = usize::try_from(self.pop_int()?).map_err(|_| invalid("a negative blend count"))?;
                let k = *self.regions.get(self.vsindex).ok_or_else(|| invalid(format!("vsindex {} does not exist", self.vsindex)))?;
                let used = n.checked_mul(k + 1).filter(|u| *u <= self.depth).ok_or_else(|| invalid("blend has too few operands"))?;
                self.pending.push(Tok::Num(i64::try_from(n).unwrap_or(0) * ONE));
                self.pending.push(Tok::Blend);
                self.depth = self.depth - used + n;
            }
            other => return Err(invalid(format!("operator {other} is reserved"))),
        }
        Ok(Flow::Go)
    }

    fn escape(&mut self, op: u8) -> Result<()> {
        match op {
            // dotsection (an old hint)
            0 => {
                if self.strip {
                    self.drop_operands();
                } else {
                    self.emit(&[12, 0])?;
                }
            }
            34..=37 => {
                let all_numbers: Option<Vec<i64>> = self.pending.iter().map(|t| if let Tok::Num(v) = t { Some(*v) } else { None }).collect();
                match all_numbers {
                    Some(args) if self.strip && !self.keep_flex => {
                        let curves = flex_curves(op, &args)?;
                        self.pending = curves.into_iter().map(Tok::Num).collect();
                        self.depth = self.pending.len();
                        self.removed = true;
                        self.emit(&[8])?;
                    }
                    _ => self.emit(&[12, op])?,
                }
            }
            3..=5 | 9..=12 | 14 | 15 | 18 | 20..=24 | 26..=30 => {
                return Err(TablesError::Unsupported(format!("the charstring arithmetic operator 12 {op}")));
            }
            other => return Err(invalid(format!("operator 12 {other} is reserved"))),
        }
        Ok(())
    }
}

/// The twelve `rrcurveto` operands of a flex operator's two curves.
fn flex_curves(op: u8, a: &[i64]) -> Result<Vec<i64>> {
    let wrong = || invalid(format!("flex operator 12 {op} has {} operands", a.len()));
    let g = |i: usize| a.get(i).copied().ok_or_else(wrong);
    match op {
        // hflex: dx1 dx2 dy2 dx3 dx4 dx5 dx6
        34 if a.len() == 7 => Ok(vec![g(0)?, 0, g(1)?, g(2)?, g(3)?, 0, g(4)?, 0, g(5)?, -g(2)?, g(6)?, 0]),
        // flex: dx1 dy1 … dx6 dy6 fd
        35 if a.len() == 13 => Ok(a.get(..12).ok_or_else(wrong)?.to_vec()),
        // hflex1: dx1 dy1 dx2 dy2 dx3 dx4 dx5 dy5 dx6
        36 if a.len() == 9 => {
            let dy6 = -(g(1)? + g(3)? + g(7)?);
            Ok(vec![g(0)?, g(1)?, g(2)?, g(3)?, g(4)?, 0, g(5)?, 0, g(6)?, g(7)?, g(8)?, dy6])
        }
        // flex1: dx1 dy1 … dx5 dy5 d6; d6 is the last x or y, the other returns to the start.
        37 if a.len() == 11 => {
            let dx: i64 = [0, 2, 4, 6, 8].iter().map(|i| a.get(*i).copied().unwrap_or(0)).sum();
            let dy: i64 = [1, 3, 5, 7, 9].iter().map(|i| a.get(*i).copied().unwrap_or(0)).sum();
            let (dx6, dy6) = if dx.abs() > dy.abs() { (g(10)?, -dy) } else { (-dx, g(10)?) };
            let mut v = a.get(..10).ok_or_else(wrong)?.to_vec();
            v.extend([dx6, dy6]);
            Ok(v)
        }
        _ => Err(wrong()),
    }
}

/// Writes one operand: an integer in the shortest form, anything else as 16.16 fixed.
fn encode_tok(t: Tok, out: &mut Vec<u8>) -> Result<()> {
    let v = match t {
        Tok::Blend => {
            out.push(16);
            return Ok(());
        }
        Tok::Num(v) => v,
    };
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

fn read_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

#[cfg(test)]
pub(super) mod build {
    //! Small CFF helpers for the tests: an assembler, a disassembler and readers.

    const OPS: [(&str, &[u8]); 21] = [
        ("hstem", &[1]),
        ("vstem", &[3]),
        ("vmoveto", &[4]),
        ("rlineto", &[5]),
        ("hlineto", &[6]),
        ("vlineto", &[7]),
        ("rrcurveto", &[8]),
        ("callsubr", &[10]),
        ("return", &[11]),
        ("endchar", &[14]),
        ("hstemhm", &[18]),
        ("hintmask", &[19]),
        ("cntrmask", &[20]),
        ("rmoveto", &[21]),
        ("hmoveto", &[22]),
        ("vstemhm", &[23]),
        ("callgsubr", &[29]),
        ("hflex", &[12, 34]),
        ("flex", &[12, 35]),
        ("hflex1", &[12, 36]),
        ("flex1", &[12, 37]),
    ];

    /// An INDEX of `items`.
    pub fn index(items: &[Vec<u8>], cff2: bool) -> Vec<u8> {
        let mut out = Vec::new();
        super::write_index(&mut out, items.iter().map(Vec::as_slice), cff2).unwrap();
        out
    }

    /// A charstring from text: integers, operator names and `<hex>` mask bytes.
    pub fn charstring(program: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for word in program.split_whitespace() {
            if let Ok(n) = word.parse::<i64>() {
                super::encode_tok(super::Tok::Num(n * super::ONE), &mut out).unwrap();
            } else if let Some(hex) = word.strip_prefix('<').and_then(|w| w.strip_suffix('>')) {
                for k in (0..hex.len()).step_by(2) {
                    out.push(u8::from_str_radix(&hex[k..k + 2], 16).unwrap());
                }
            } else {
                let (_, bytes) = OPS.iter().find(|(name, _)| *name == word).unwrap_or_else(|| panic!("unknown operator {word}"));
                out.extend_from_slice(bytes);
            }
        }
        out
    }

    /// A flat charstring as text, integers only; hint masks print as `<hex>`.
    pub fn disassemble(code: &[u8]) -> String {
        let mut words: Vec<String> = Vec::new();
        let (mut i, mut operands, mut stems) = (0usize, 0usize, 0usize);
        while i < code.len() {
            let b0 = code[i];
            let tok = match b0 {
                28 | 32..=255 => {
                    let (v, len) = match b0 {
                        28 => (i64::from(i16::from_be_bytes([code[i + 1], code[i + 2]])), 3),
                        32..=246 => (i64::from(b0) - 139, 1),
                        247..=250 => ((i64::from(b0) - 247) * 256 + i64::from(code[i + 1]) + 108, 2),
                        251..=254 => (-(i64::from(b0) - 251) * 256 - i64::from(code[i + 1]) - 108, 2),
                        _ => (i64::from(i32::from_be_bytes([code[i + 1], code[i + 2], code[i + 3], code[i + 4]])) / super::ONE, 5),
                    };
                    i += len;
                    operands += 1;
                    v.to_string()
                }
                _ => {
                    let len = if b0 == 12 { 2 } else { 1 };
                    let bytes = &code[i..i + len];
                    i += len;
                    let name = OPS.iter().find(|(_, b)| *b == bytes).map(|(n, _)| *n).unwrap_or("?");
                    if matches!(name, "hstem" | "vstem" | "hstemhm" | "vstemhm" | "hintmask" | "cntrmask") {
                        stems += operands / 2;
                    }
                    operands = 0;
                    if matches!(name, "hintmask" | "cntrmask") {
                        let n = stems.div_ceil(8);
                        let mask: String = code[i..i + n].iter().map(|b| format!("{b:02x}")).collect();
                        i += n;
                        words.push(name.to_string());
                        format!("<{mask}>")
                    } else {
                        name.to_string()
                    }
                }
            };
            words.push(tok);
        }
        words.join(" ")
    }

    /// The charstrings of a CFF table, as text.
    pub fn programs(table: &[u8]) -> Vec<String> {
        let t = super::Table::read(table, false).unwrap();
        t.charstrings.iter().map(|c| disassemble(c)).collect()
    }

    /// The operators of each Private DICT of a CFF table, and whether any Subrs remain.
    pub fn private_ops(table: &[u8]) -> Vec<Vec<u16>> {
        let t = super::Table::read(table, false).unwrap();
        assert!(t.global_subrs.is_empty() && t.privates.iter().all(|p| p.subrs.is_empty()), "no subroutines are left");
        t.privates.iter().map(|p| p.entries.iter().map(|e| e.op).collect()).collect()
    }
}
