//! Rewriting a `CFF ` table with a new glyph order or new glyph names.
//!
//! The table is parsed into its parts (header, Name INDEX, Top DICT, String INDEX, global
//! subroutines, charset, CharStrings, Private DICTs with their local subroutines, and for
//! CID-keyed fonts FDArray and FDSelect), then laid out again: the charstrings, charset and
//! FDSelect follow the new order, every offset in the Top DICT, the font DICTs and the Private
//! DICTs is written as a five-byte integer and recomputed. Charstrings and subroutines are
//! copied byte for byte, so no outline or hint changes. Strings are kept and new glyph names
//! are appended, so every SID in the dictionaries stays valid.

use std::collections::HashMap;

use read_fonts::ps::string::STANDARD_STRINGS;

use super::be::{Data, put_u16, to_u16};
use crate::limits::{MAX_CFF2_STACK, MAX_SFNT};
use crate::{Result, TablesError};

const TAG: [u8; 4] = *b"CFF ";
const CHARSET: u16 = 15;
const ENCODING: u16 = 16;
const CHARSTRINGS: u16 = 17;
const PRIVATE: u16 = 18;
const SUBRS: u16 = 19;
const ROS: u16 = 1230;
const CID_COUNT: u16 = 1234;
const FONT_NAME: u16 = 1238;
const FDARRAY: u16 = 1236;
const FDSELECT: u16 = 1237;
/// Most font DICTs: FDSelect stores a font DICT index in one byte.
const MAX_FONT_DICTS: usize = 256;
/// Longest glyph name a CFF charset should hold (the CFF specification's advice).
pub(crate) const MAX_CFF_NAME: usize = 63;

/// One DICT entry: its operator (escaped operators as 1200 + the second byte) and operands as
/// they were written, with the integer value of each integer operand.
#[derive(Clone, Debug)]
struct Entry {
    op: u16,
    operands: Vec<Vec<u8>>,
    ints: Vec<Option<i64>>,
}

impl Entry {
    fn int(&self, i: usize) -> Option<i64> {
        self.ints.get(i).copied().flatten()
    }
}

#[derive(Clone, Debug)]
struct Private<'a> {
    dict: Vec<Entry>,
    subrs: Option<&'a [u8]>,
    /// Where `subrs` starts in the table.
    subrs_at: Option<usize>,
    /// The slot of `subrs` in [`Cff::local_subrs`].
    local: Option<usize>,
}

/// The local and global subroutines of a glyph, item by item.
pub(crate) type Subrs<'s, 'a> = (&'s [&'a [u8]], &'s [&'a [u8]]);

/// A parsed `CFF ` table.
#[derive(Clone)]
pub(crate) struct Cff<'a> {
    header: &'a [u8],
    name_index: &'a [u8],
    top: Vec<Entry>,
    strings: Vec<&'a [u8]>,
    gsubrs: &'a [u8],
    /// The items of `gsubrs`, parsed once.
    global_subrs: Vec<&'a [u8]>,
    /// The items of every distinct local Subrs INDEX, parsed once; Private DICTs that share one
    /// share its slot.
    local_subrs: Vec<Vec<&'a [u8]>>,
    charstrings: Vec<&'a [u8]>,
    /// SID (or CID) per glyph; glyph 0 has 0.
    charset: Vec<u16>,
    cid: bool,
    encoding: Option<&'a [u8]>,
    privates: Vec<Private<'a>>,
    font_dicts: Vec<Vec<Entry>>,
    fd_select: Vec<u8>,
}

/// The items of an INDEX at `at` and the offset just past it.
fn index<'a>(d: Data<'a>, at: usize) -> Result<(Vec<&'a [u8]>, usize)> {
    let count = usize::from(d.u16(at)?);
    if count == 0 {
        return Ok((Vec::new(), at + 2));
    }
    let off_size = usize::from(d.u8(at + 2)?);
    if !(1..=4).contains(&off_size) {
        return Err(d.bad(format!("has an INDEX with offset size {off_size}")));
    }
    let offsets_at = at + 3;
    let read = |i: usize| -> Result<usize> {
        let s = d.slice(offsets_at + i * off_size, off_size)?;
        Ok(s.iter().fold(0usize, |acc, b| acc << 8 | usize::from(*b)))
    };
    // The data starts one byte before the first offset's origin (offsets count from 1).
    let base = offsets_at + (count + 1) * off_size - 1;
    let mut items = Vec::with_capacity(count);
    let mut prev = read(0)?;
    if prev != 1 {
        return Err(d.bad("has an INDEX whose first offset is not 1"));
    }
    for i in 1..=count {
        let next = read(i)?;
        if next < prev {
            return Err(d.bad("has an INDEX with decreasing offsets"));
        }
        items.push(d.slice(base + prev, next - prev)?);
        prev = next;
    }
    Ok((items, base + prev))
}

/// Writes an INDEX with the smallest offset size.
fn write_index(items: &[&[u8]]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    put_u16(&mut out, to_u16(items.len(), "an INDEX's item count")?);
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
    let mut offset = 1usize;
    let put = |out: &mut Vec<u8>, v: usize| {
        let bytes = (v as u32).to_be_bytes();
        out.extend_from_slice(bytes.get(4 - off_size..).unwrap_or_default());
    };
    put(&mut out, offset);
    for item in items {
        offset += item.len();
        put(&mut out, offset);
    }
    for item in items {
        out.extend_from_slice(item);
    }
    Ok(out)
}

/// The entries of a DICT.
fn dict(d: Data<'_>, bytes: &[u8]) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let mut operands: Vec<Vec<u8>> = Vec::new();
    let mut ints: Vec<Option<i64>> = Vec::new();
    let mut i = 0usize;
    let byte = |i: usize| bytes.get(i).copied().ok_or_else(|| d.bad("has a DICT that is cut short"));
    while i < bytes.len() {
        let b0 = byte(i)?;
        let (len, value): (usize, Option<i64>) = match b0 {
            0..=21 => {
                let op = if b0 == 12 { 1200 + u16::from(byte(i + 1)?) } else { u16::from(b0) };
                i += if b0 == 12 { 2 } else { 1 };
                out.push(Entry { op, operands: std::mem::take(&mut operands), ints: std::mem::take(&mut ints) });
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
        if operands.len() >= MAX_CFF2_STACK {
            return Err(TablesError::TooLarge { what: "the operands of a DICT entry", limit: MAX_CFF2_STACK });
        }
        let raw = bytes.get(i..i + len).ok_or_else(|| d.bad("has a DICT that is cut short"))?;
        operands.push(raw.to_vec());
        ints.push(value);
        i += len;
    }
    Ok(out)
}

fn find(entries: &[Entry], op: u16) -> Option<&Entry> {
    entries.iter().find(|e| e.op == op)
}

fn offset_of(d: Data<'_>, e: &Entry, i: usize) -> Result<usize> {
    let v = e.int(i).ok_or_else(|| d.bad(format!("has a DICT operator {} without an integer offset", e.op)))?;
    usize::try_from(v).map_err(|_| d.bad("has a negative offset"))
}

fn int29(v: usize) -> Result<Vec<u8>> {
    let v = i32::try_from(v).map_err(|_| TablesError::TooLarge { what: "a CFF offset", limit: i32::MAX as usize })?;
    let mut out = vec![29];
    out.extend_from_slice(&v.to_be_bytes());
    Ok(out)
}

/// A DICT integer in its shortest encoding.
fn dict_int(v: i64) -> Result<Vec<u8>> {
    Ok(match v {
        -107..=107 => vec![(v + 139) as u8],
        108..=1131 => vec![(((v - 108) >> 8) + 247) as u8, ((v - 108) & 0xFF) as u8],
        -1131..=-108 => vec![(((-v - 108) >> 8) + 251) as u8, ((-v - 108) & 0xFF) as u8],
        -32768..=32767 => {
            let mut out = vec![28];
            out.extend_from_slice(&(v as i16).to_be_bytes());
            out
        }
        _ => {
            let v = i32::try_from(v).map_err(|_| TablesError::invalid("a CFF DICT number is out of range"))?;
            let mut out = vec![29];
            out.extend_from_slice(&v.to_be_bytes());
            out
        }
    })
}

/// Encodes `entries`, giving the operators in `replace` the operands listed there (as
/// five-byte integers); replaced operators the entries lack are appended.
fn encode(entries: &[Entry], replace: &[(u16, Vec<usize>)]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut done = Vec::new();
    let put_op = |out: &mut Vec<u8>, op: u16| {
        if op >= 1200 {
            out.push(12);
            out.push((op - 1200) as u8);
        } else {
            out.push(op as u8);
        }
    };
    for e in entries {
        if let Some((_, values)) = replace.iter().find(|(op, _)| *op == e.op) {
            for v in values {
                out.extend(int29(*v)?);
            }
            done.push(e.op);
        } else {
            for o in &e.operands {
                out.extend_from_slice(o);
            }
        }
        put_op(&mut out, e.op);
    }
    for (op, values) in replace {
        if !done.contains(op) {
            for v in values {
                out.extend(int29(*v)?);
            }
            put_op(&mut out, *op);
        }
    }
    Ok(out)
}

fn private<'a>(d: Data<'a>, e: &Entry) -> Result<Private<'a>> {
    let size = offset_of(d, e, 0)?;
    let at = offset_of(d, e, 1)?;
    let bytes = d.slice(at, size)?;
    let dict = dict(d, bytes)?;
    let (subrs, subrs_at) = match find(&dict, SUBRS) {
        Some(s) => {
            let start = at.checked_add(offset_of(d, s, 0)?).ok_or_else(|| d.bad("has a Subrs offset past the end"))?;
            let (_, end) = index(d, start)?;
            (Some(d.slice(start, end - start)?), Some(start))
        }
        None => (None, None),
    };
    Ok(Private { dict, subrs, subrs_at, local: None })
}

/// Parses `bytes` as a `CFF ` table of a font with `num_glyphs` glyphs.
pub(crate) fn parse(bytes: &[u8], num_glyphs: usize) -> Result<Cff<'_>> {
    let d = Data::new(TAG, bytes);
    if d.u8(0)? != 1 {
        return Err(TablesError::Unsupported(format!("CFF major version {}", d.u8(0)?)));
    }
    let hdr = usize::from(d.u8(2)?);
    if hdr < 4 {
        return Err(d.bad("has a header shorter than 4 bytes"));
    }
    let header = d.slice(0, hdr)?;
    let (_, name_end) = index(d, hdr)?;
    let name_index = d.slice(hdr, name_end - hdr)?;
    let (tops, top_end) = index(d, name_end)?;
    let [top] = tops.as_slice() else {
        return Err(TablesError::Unsupported(format!("a CFF table with {} fonts", tops.len())));
    };
    let top = dict(d, top)?;
    let (strings, strings_end) = index(d, top_end)?;
    let (global_subrs, gsubrs_end) = index(d, strings_end)?;
    let gsubrs = d.slice(strings_end, gsubrs_end - strings_end)?;

    let cs = find(&top, CHARSTRINGS).ok_or_else(|| d.bad("has no CharStrings"))?;
    let (charstrings, _) = index(d, offset_of(d, cs, 0)?)?;
    if charstrings.len() != num_glyphs {
        return Err(d.bad(format!("has {} charstrings for {num_glyphs} glyphs", charstrings.len())));
    }
    let cid = find(&top, ROS).is_some();
    let charset_at = match find(&top, CHARSET) {
        Some(e) => offset_of(d, e, 0)?,
        None => 0,
    };
    let charset = match charset_at {
        0 if num_glyphs <= 229 && !cid => (0..num_glyphs as u16).collect(),
        0..=2 => return Err(TablesError::Unsupported("a CFF table with a predefined charset other than ISOAdobe".into())),
        at => read_charset(d, at, num_glyphs)?,
    };
    let encoding = match find(&top, ENCODING) {
        Some(e) if offset_of(d, e, 0)? > 1 => {
            let at = offset_of(d, e, 0)?;
            let format = d.u8(at)?;
            let mut len = match format & 0x7F {
                0 => 2 + usize::from(d.u8(at + 1)?),
                1 => 2 + 2 * usize::from(d.u8(at + 1)?),
                _ => return Err(d.bad(format!("has an Encoding of format {format}"))),
            };
            if format & 0x80 != 0 {
                len += 1 + 3 * usize::from(d.u8(at + len)?);
            }
            Some(d.slice(at, len)?)
        }
        _ => None,
    };

    let mut privates = Vec::new();
    let mut font_dicts = Vec::new();
    let mut fd_select = Vec::new();
    if cid {
        let fda = find(&top, FDARRAY).ok_or_else(|| d.bad("is CID-keyed but has no FDArray"))?;
        let (dicts, _) = index(d, offset_of(d, fda, 0)?)?;
        if dicts.len() > MAX_FONT_DICTS {
            return Err(TablesError::TooLarge { what: "the font DICTs of a CFF table", limit: MAX_FONT_DICTS });
        }
        for fd in dicts {
            let entries = dict(d, fd)?;
            let p = find(&entries, PRIVATE).ok_or_else(|| d.bad("has a font DICT without a Private DICT"))?;
            privates.push(private(d, p)?);
            font_dicts.push(entries);
        }
        let fds = find(&top, FDSELECT).ok_or_else(|| d.bad("is CID-keyed but has no FDSelect"))?;
        fd_select = read_fd_select(d, offset_of(d, fds, 0)?, num_glyphs)?;
        if fd_select.iter().any(|fd| usize::from(*fd) >= font_dicts.len()) {
            return Err(d.bad("has an FDSelect entry past the FDArray"));
        }
    } else if let Some(p) = find(&top, PRIVATE) {
        privates.push(private(d, p)?);
    }
    let mut local_subrs: Vec<Vec<&[u8]>> = Vec::new();
    let mut slots: HashMap<usize, usize> = HashMap::new();
    for p in &mut privates {
        let Some(start) = p.subrs_at else { continue };
        let slot = match slots.get(&start) {
            Some(slot) => *slot,
            None => {
                local_subrs.push(index(d, start)?.0);
                slots.insert(start, local_subrs.len() - 1);
                local_subrs.len() - 1
            }
        };
        p.local = Some(slot);
    }
    Ok(Cff {
        header,
        name_index,
        top,
        strings,
        gsubrs,
        global_subrs,
        local_subrs,
        charstrings,
        charset,
        cid,
        encoding,
        privates,
        font_dicts,
        fd_select,
    })
}

fn read_charset(d: Data<'_>, at: usize, n: usize) -> Result<Vec<u16>> {
    let mut out = Vec::with_capacity(n);
    out.push(0);
    let format = d.u8(at)?;
    let mut pos = at + 1;
    match format {
        0 => {
            for _ in 1..n {
                out.push(d.u16(pos)?);
                pos += 2;
            }
        }
        1 | 2 => {
            while out.len() < n {
                let first = d.u16(pos)?;
                let left = if format == 1 { usize::from(d.u8(pos + 2)?) } else { usize::from(d.u16(pos + 2)?) };
                pos += if format == 1 { 3 } else { 4 };
                for k in 0..=left {
                    if out.len() >= n {
                        break;
                    }
                    out.push(first.wrapping_add(k as u16));
                }
            }
        }
        _ => return Err(d.bad(format!("has a charset of format {format}"))),
    }
    Ok(out)
}

fn read_fd_select(d: Data<'_>, at: usize, n: usize) -> Result<Vec<u8>> {
    match d.u8(at)? {
        0 => Ok(d.slice(at + 1, n)?.to_vec()),
        3 => {
            let ranges = usize::from(d.u16(at + 1)?);
            let mut out = vec![0u8; n];
            for r in 0..ranges {
                let first = usize::from(d.u16(at + 3 + r * 3)?);
                let fd = d.u8(at + 5 + r * 3)?;
                let end = usize::from(d.u16(at + 6 + r * 3)?).min(n);
                for slot in out.iter_mut().take(end).skip(first) {
                    *slot = fd;
                }
            }
            Ok(out)
        }
        f => Err(d.bad(format!("has an FDSelect of format {f}"))),
    }
}

impl<'a> Cff<'a> {
    /// Glyph names by glyph id: from the charset, or `cidNNNNN` for a CID-keyed font as
    /// fontTools names them.
    pub(crate) fn glyph_names(&self) -> Vec<String> {
        self.charset
            .iter()
            .enumerate()
            .map(|(gid, sid)| {
                if gid == 0 {
                    ".notdef".to_string()
                } else if self.cid {
                    format!("cid{sid:05}")
                } else {
                    self.sid_name(*sid)
                }
            })
            .collect()
    }

    fn sid_name(&self, sid: u16) -> String {
        let sid = usize::from(sid);
        match STANDARD_STRINGS.get(sid) {
            Some(s) => s.to_string(),
            None => {
                self.strings.get(sid - STANDARD_STRINGS.len()).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_else(|| format!("sid{sid}"))
            }
        }
    }

    pub(crate) fn is_cid(&self) -> bool {
        self.cid
    }

    /// The Font DICT of every glyph (all 0 in a name-keyed font) and the number of Font DICTs.
    pub(crate) fn font_dict_of_glyphs(&self) -> (Vec<u16>, usize) {
        if self.cid {
            (self.fd_select.iter().map(|fd| u16::from(*fd)).collect(), self.font_dicts.len())
        } else {
            (vec![0; self.charstrings.len()], 1)
        }
    }

    /// Registry, ordering and supplement of a CID-keyed font.
    pub(crate) fn ros(&self) -> Option<(String, String, i64)> {
        let ints = self.top_ints(ROS)?;
        let sid = |v: i64| u16::try_from(v).ok().map(|s| self.sid_name(s));
        Some((sid(*ints.first()?)?, sid(*ints.get(1)?)?, *ints.get(2)?))
    }

    /// The table made CID-keyed: CID n is glyph n, one Font DICT named `fd_name` takes the
    /// Private DICT and its subroutines, the Top DICT gets `ROS` (first, as the specification
    /// requires) and `CIDCount`, and the Encoding goes. Charstrings and subroutines are copied
    /// byte for byte.
    pub(crate) fn to_cid_keyed(&self, registry: &str, ordering: &str, supplement: i64, fd_name: &str) -> Result<Vec<u8>> {
        if self.cid {
            return Err(TablesError::Unsupported("the CFF table is already CID-keyed".into()));
        }
        let n = self.charstrings.len();
        let mut extra: Vec<Vec<u8>> = Vec::new();
        let mut sid_of = |text: &str| -> Result<u16> {
            let bytes = text.as_bytes();
            if let Some(sid) = STANDARD_STRINGS.iter().position(|s| s.as_bytes() == bytes) {
                return to_u16(sid, "a standard string id");
            }
            if let Some(i) = self.strings.iter().position(|s| *s == bytes) {
                return to_u16(STANDARD_STRINGS.len() + i, "the CFF string count");
            }
            if let Some(i) = extra.iter().position(|s| s.as_slice() == bytes) {
                return to_u16(STANDARD_STRINGS.len() + self.strings.len() + i, "the CFF string count");
            }
            extra.push(bytes.to_vec());
            to_u16(STANDARD_STRINGS.len() + self.strings.len() + extra.len() - 1, "the CFF string count")
        };
        let ros = [i64::from(sid_of(registry)?), i64::from(sid_of(ordering)?), supplement];
        let fd_name_sid = i64::from(sid_of(fd_name)?);
        let ints_entry = |op: u16, values: &[i64]| -> Result<Entry> {
            Ok(Entry { op, operands: values.iter().map(|v| dict_int(*v)).collect::<Result<_>>()?, ints: values.iter().map(|v| Some(*v)).collect() })
        };
        let mut top = vec![ints_entry(ROS, &ros)?];
        top.extend(self.top.iter().filter(|e| !matches!(e.op, ENCODING | PRIVATE | CHARSET | ROS | CID_COUNT)).cloned());
        top.push(ints_entry(CID_COUNT, &[i64::try_from(n).unwrap_or(0)])?);
        let font_dict = vec![ints_entry(FONT_NAME, &[fd_name_sid])?];
        let mut cid: Cff<'_> = self.clone();
        cid.strings.extend(extra.iter().map(Vec::as_slice));
        cid.top = top;
        cid.cid = true;
        cid.encoding = None;
        cid.charset = (0..n).map(|g| to_u16(g, "a CID")).collect::<Result<_>>()?;
        cid.font_dicts = vec![font_dict];
        cid.fd_select = vec![0; n];
        if cid.privates.is_empty() {
            cid.privates.push(Private { dict: Vec::new(), subrs: None, subrs_at: None, local: None });
        }
        let identity: Vec<u16> = (0..n).map(|g| to_u16(g, "a glyph id")).collect::<Result<_>>()?;
        cid.rebuild(&identity, None)
    }

    /// Gives the Top DICT operator `op` the integer operands `values`, replacing its entry or
    /// adding one at the end.
    pub(crate) fn set_top_ints(&mut self, op: u16, values: &[i64]) -> Result<()> {
        let operands = values.iter().map(|v| dict_int(*v)).collect::<Result<Vec<_>>>()?;
        let entry = Entry { op, operands, ints: values.iter().map(|v| Some(*v)).collect() };
        match self.top.iter_mut().find(|e| e.op == op) {
            Some(e) => *e = entry,
            None => self.top.push(entry),
        }
        Ok(())
    }

    /// The integer operands of Top DICT operator `op`, when the entry is there and every operand
    /// is an integer.
    pub(crate) fn top_ints(&self, op: u16) -> Option<Vec<i64>> {
        self.top.iter().find(|e| e.op == op)?.ints.iter().copied().collect()
    }

    /// The SID (or CID) of every glyph.
    pub(crate) fn charset(&self) -> &[u16] {
        &self.charset
    }

    /// The byte length of every charstring.
    pub(crate) fn charstring_lengths(&self) -> Vec<usize> {
        self.charstrings.iter().map(|c| c.len()).collect()
    }

    /// The table with glyph `new_to_old[i]` at index `i`, and with `names` (by new glyph id)
    /// in the charset when given. A CID-keyed font keeps its CIDs.
    pub(crate) fn rebuild(&self, new_to_old: &[u16], names: Option<&[String]>) -> Result<Vec<u8>> {
        self.rebuild_with(new_to_old, names, &HashMap::new(), false)
    }

    /// The charstring of glyph `g`.
    pub(crate) fn charstring(&self, g: usize) -> Option<&[u8]> {
        self.charstrings.get(g).copied()
    }

    /// The table with `items` as its global subroutines and the charstrings of `replace`.
    #[cfg(test)]
    pub(crate) fn with_global_subrs(&self, items: &[&[u8]], replace: &HashMap<usize, Vec<u8>>) -> Result<Vec<u8>> {
        let index = write_index(items)?;
        let mut table = self.clone();
        table.gsubrs = &index;
        let identity: Vec<u16> = (0..self.charstrings.len()).map(|g| to_u16(g, "a glyph id")).collect::<Result<_>>()?;
        table.rebuild_with(&identity, None, replace, false)
    }

    /// The local and global subroutines glyph `g` calls into.
    pub(crate) fn subrs(&self, g: usize) -> Subrs<'_, 'a> {
        let local = self.private_of(g).and_then(|p| p.local).and_then(|slot| self.local_subrs.get(slot)).map_or(&[][..], Vec::as_slice);
        (local, &self.global_subrs)
    }

    /// `defaultWidthX` and `nominalWidthX` of the Private DICT glyph `g` uses, when they are
    /// integers (both default to 0).
    pub(crate) fn widths(&self, g: usize) -> (Option<i64>, Option<i64>) {
        let get = |op: u16| match self.private_of(g).and_then(|p| find(&p.dict, op)) {
            None => Some(0),
            Some(e) => e.int(0),
        };
        (get(20), get(21))
    }

    fn private_of(&self, g: usize) -> Option<&Private<'_>> {
        let fd = if self.cid { usize::from(self.fd_select.get(g).copied().unwrap_or(0)) } else { 0 };
        self.privates.get(fd)
    }

    /// [`Self::rebuild`] with the charstrings of `replace` (by new glyph id) and, when
    /// `standard_encoding`, without a custom or expert Encoding.
    pub(crate) fn rebuild_with(
        &self,
        new_to_old: &[u16],
        names: Option<&[String]>,
        replace: &HashMap<usize, Vec<u8>>,
        standard_encoding: bool,
    ) -> Result<Vec<u8>> {
        let old = |g: usize| usize::from(new_to_old.get(g).copied().unwrap_or(0));
        let encoding = if standard_encoding { None } else { self.encoding };
        let top: Vec<Entry> = if standard_encoding { self.top.iter().filter(|e| e.op != ENCODING).cloned().collect() } else { self.top.clone() };
        let n = new_to_old.len();
        let mut strings: Vec<Vec<u8>> = self.strings.iter().map(|s| s.to_vec()).collect();
        let mut charset: Vec<u16> = (0..n).map(|g| self.charset.get(old(g)).copied().unwrap_or(0)).collect();
        if let Some(names) = names {
            if self.cid {
                return Err(TablesError::Unsupported("renaming the glyphs of a CID-keyed font".into()));
            }
            let mut known: HashMap<Vec<u8>, u16> = HashMap::new();
            for (sid, s) in STANDARD_STRINGS.iter().enumerate() {
                known.entry(s.as_bytes().to_vec()).or_insert(sid as u16);
            }
            for (i, s) in strings.iter().enumerate() {
                known.entry(s.clone()).or_insert((STANDARD_STRINGS.len() + i) as u16);
            }
            for (g, name) in names.iter().enumerate().skip(1) {
                let sid = match known.get(name.as_bytes()) {
                    Some(sid) => *sid,
                    None => {
                        let sid = to_u16(STANDARD_STRINGS.len() + strings.len(), "the CFF string count")?;
                        strings.push(name.as_bytes().to_vec());
                        known.insert(name.as_bytes().to_vec(), sid);
                        sid
                    }
                };
                if let Some(slot) = charset.get_mut(g) {
                    *slot = sid;
                }
            }
        }
        let charstrings: Vec<&[u8]> =
            (0..n).map(|g| replace.get(&g).map(Vec::as_slice).unwrap_or_else(|| self.charstrings.get(old(g)).copied().unwrap_or_default())).collect();
        let fd_select: Vec<u8> = if self.cid { (0..n).map(|g| self.fd_select.get(old(g)).copied().unwrap_or(0)).collect() } else { Vec::new() };

        let string_refs: Vec<&[u8]> = strings.iter().map(Vec::as_slice).collect();
        let string_index = write_index(&string_refs)?;
        let charset_bytes = write_charset(&charset);
        let fd_select_bytes = if self.cid { write_fd_select(&fd_select)? } else { Vec::new() };
        let cs_index = write_index(&charstrings)?;

        // Every offset is a five-byte integer, so the sizes are known before the offsets.
        let privates_sized: Vec<Vec<u8>> = self.privates.iter().map(|p| encode_private(p)).collect::<Result<_>>()?;
        let private_spans: Vec<usize> = privates_sized.iter().zip(&self.privates).map(|(b, p)| b.len() + p.subrs.map_or(0, <[u8]>::len)).collect();
        let fd_dummy: Vec<Vec<u8>> = self.font_dicts.iter().map(|fd| encode(fd, &[(PRIVATE, vec![0, 0])])).collect::<Result<_>>()?;
        let fd_refs: Vec<&[u8]> = fd_dummy.iter().map(Vec::as_slice).collect();
        let fd_index_len = if self.cid { write_index(&fd_refs)?.len() } else { 0 };
        let top_replace = |charset: usize, fds: usize, cs: usize, fda: usize, private: (usize, usize), enc: usize| {
            let mut r = vec![(CHARSET, vec![charset]), (CHARSTRINGS, vec![cs])];
            if self.cid {
                r.push((FDSELECT, vec![fds]));
                r.push((FDARRAY, vec![fda]));
            } else if !self.privates.is_empty() {
                r.push((PRIVATE, vec![private.0, private.1]));
            }
            if encoding.is_some() {
                r.push((ENCODING, vec![enc]));
            }
            r
        };
        let top_len = write_index(&[encode(&top, &top_replace(0, 0, 0, 0, (0, 0), 0))?.as_slice()])?.len();

        let mut pos = self.header.len() + self.name_index.len() + top_len + string_index.len() + self.gsubrs.len();
        let charset_at = pos;
        pos += charset_bytes.len();
        let fd_select_at = pos;
        pos += fd_select_bytes.len();
        let cs_at = pos;
        pos += cs_index.len();
        let fd_array_at = pos;
        pos += fd_index_len;
        let mut private_at = Vec::with_capacity(private_spans.len());
        for span in &private_spans {
            private_at.push(pos);
            pos += span;
        }
        let encoding_at = pos;
        if pos > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the rewritten CFF table", limit: MAX_SFNT });
        }

        let first_private = (privates_sized.first().map_or(0, Vec::len), private_at.first().copied().unwrap_or(0));
        let top = encode(&top, &top_replace(charset_at, fd_select_at, cs_at, fd_array_at, first_private, encoding_at))?;
        let mut out = Vec::with_capacity(pos);
        out.extend_from_slice(self.header);
        out.extend_from_slice(self.name_index);
        out.extend(write_index(&[top.as_slice()])?);
        out.extend(string_index);
        out.extend_from_slice(self.gsubrs);
        out.extend(charset_bytes);
        out.extend(fd_select_bytes);
        out.extend(cs_index);
        if self.cid {
            let fds: Vec<Vec<u8>> = self
                .font_dicts
                .iter()
                .enumerate()
                .map(|(i, fd)| {
                    let size = privates_sized.get(i).map_or(0, Vec::len);
                    encode(fd, &[(PRIVATE, vec![size, private_at.get(i).copied().unwrap_or(0)])])
                })
                .collect::<Result<_>>()?;
            let refs: Vec<&[u8]> = fds.iter().map(Vec::as_slice).collect();
            out.extend(write_index(&refs)?);
        }
        for (bytes, p) in privates_sized.iter().zip(&self.privates) {
            out.extend_from_slice(bytes);
            if let Some(subrs) = p.subrs {
                out.extend_from_slice(subrs);
            }
        }
        if let Some(enc) = encoding {
            out.extend_from_slice(enc);
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

/// A charset in the smallest of the three formats.
fn write_charset(sids: &[u16]) -> Vec<u8> {
    let rest = sids.get(1..).unwrap_or_default();
    let ranges = |max_left: usize| {
        let mut out: Vec<(u16, usize)> = Vec::new();
        for sid in rest {
            match out.last_mut() {
                Some((first, left)) if *left < max_left && usize::from(*first) + *left + 1 == usize::from(*sid) => *left += 1,
                _ => out.push((*sid, 0)),
            }
        }
        out
    };
    let r1 = ranges(0xFF);
    let r2 = ranges(0xFFFF);
    let (f0, f1, f2) = (2 * rest.len(), 3 * r1.len(), 4 * r2.len());
    let mut out = Vec::new();
    if f0 <= f1 && f0 <= f2 {
        out.push(0);
        for sid in rest {
            put_u16(&mut out, *sid);
        }
    } else if f1 <= f2 {
        out.push(1);
        for (first, left) in r1 {
            put_u16(&mut out, first);
            out.push(left as u8);
        }
    } else {
        out.push(2);
        for (first, left) in r2 {
            put_u16(&mut out, first);
            put_u16(&mut out, left as u16);
        }
    }
    out
}

/// FDSelect in format 3 (ranges) or 0, whichever is smaller.
fn write_fd_select(fds: &[u8]) -> Result<Vec<u8>> {
    let mut ranges: Vec<(usize, u8)> = Vec::new();
    for (g, fd) in fds.iter().enumerate() {
        if ranges.last().is_none_or(|(_, last)| last != fd) {
            ranges.push((g, *fd));
        }
    }
    let mut out = Vec::new();
    if fds.len() < 5 + 3 * ranges.len() {
        out.push(0);
        out.extend_from_slice(fds);
    } else {
        out.push(3);
        put_u16(&mut out, to_u16(ranges.len(), "the FDSelect ranges")?);
        for (first, fd) in ranges {
            put_u16(&mut out, to_u16(first, "a glyph id")?);
            out.push(fd);
        }
        put_u16(&mut out, to_u16(fds.len(), "the glyph count")?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn charsets_take_the_smallest_format() {
        let identity: Vec<u16> = (0..300).collect();
        assert_eq!(write_charset(&identity)[0], 2, "one long range");
        let short: Vec<u16> = (0..100).collect();
        assert_eq!(write_charset(&short)[0], 1);
        let scattered = [0u16, 9, 3, 7];
        assert_eq!(write_charset(&scattered), [0, 0, 9, 0, 3, 0, 7]);
        let bytes = write_charset(&identity);
        assert_eq!(read_charset(Data::new(TAG, &bytes), 0, 300).unwrap(), identity);
    }

    #[test]
    fn index_round_trip_and_bad_offsets() {
        let items: [&[u8]; 3] = [b"a", b"", b"xyz"];
        let bytes = write_index(&items).unwrap();
        let (back, end) = index(Data::new(TAG, &bytes), 0).unwrap();
        assert_eq!(back, items);
        assert_eq!(end, bytes.len());
        // Offsets that go backwards.
        let bad = [0u8, 2, 1, 1, 3, 2, b'a', b'b'];
        assert!(index(Data::new(TAG, &bad), 0).is_err());
    }
}
