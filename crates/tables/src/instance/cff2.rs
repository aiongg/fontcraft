//! `CFF2` outlines of an instance (M12.20b), as fontTools' `varLib.instancer.instantiateCFF2`
//! makes them, and the `CFF ` table a downgrade writes, as fontTools' `cffLib.CFF2ToCFF` does
//! (both MIT, Copyright (c) 2019 Google LLC, Behdad Esfahbod and the fontTools authors). Written
//! anew here.
//!
//! - Every charstring is run with its subroutine calls followed, so the result has no
//!   subroutines (fontTools desubroutinizes too). Each value a `blend` produces becomes an
//!   [`Item`]: its default and one delta per region of its `vsindex`.
//! - A full instance adds each item's delta at the location, rounded half to even as Python's
//!   `round` does, and the variation store goes. A partial instance rebases the regions of each
//!   variation-store subtable as any item variation store is rebased (see `partial.rs`), adds the
//!   parts that no longer vary the same way and keeps the rest as blends; a subtable left without
//!   regions goes and the `vsindex` values are renumbered.
//! - Operators, hints and hint masks are kept as they are. fontTools also respecialises the
//!   operators (`hlineto`, `hvcurveto` …), which changes the bytes and not the outline.
//! - Private DICT blends are resolved the same way. In the delta-encoded lists (`BlueValues`
//!   and the like) fontTools adds a blend's deltas to the absolute values, not to the stored
//!   differences, and so does this module.
//! - The downgrade writes a CID-keyed `CFF ` table (ROS Adobe-Identity-0, CID equal to glyph id,
//!   the Name INDEX entry `CFF2Font`), each glyph's width stored against a `defaultWidthX` and
//!   `nominalWidthX` chosen per Font DICT, `endchar` at the end of each charstring, and a path
//!   operator with more operands than CFF's 48-value stack holds split into generic segments.
//!
//! Counts read from the table are capped (glyphs, Font DICTs, DICT entries, operators per glyph
//! and per table, items and their deltas, subroutine nesting).

#[cfg(test)]
#[path = "cff2_tests.rs"]
mod tests;

use read_fonts::tables::variations::ItemVariationStore as ReadStore;
use read_fonts::{FontData, FontRead};

use crate::limits::{MAX_CFF_STACK, MAX_CFF2_STACK, MAX_CHARSTRING, MAX_SFNT, MAX_SUBR_NESTING};
use crate::metrics::{Contour, Pt, Seg, exact_bounds};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Most operators one glyph may run, subroutine calls followed.
const MAX_OPS_PER_GLYPH: usize = 1 << 20;
/// Most operators the charstrings of one table may run together.
const MAX_OPS_PER_TABLE: usize = 200_000_000;
/// Most blended values one table may hold, charstrings and Private DICTs together.
const MAX_ITEMS: usize = 8 << 20;
/// Most deltas those values may hold together.
const MAX_ITEM_DELTAS: usize = 64 << 20;
/// Most entries one DICT may hold.
const MAX_DICT_ENTRIES: usize = 4_096;
/// Most Font DICTs (a CFF FDSelect index is one byte).
const MAX_FONT_DICTS: usize = 256;
/// Most glyphs a CharStrings INDEX may hold.
const MAX_GLYPHS: usize = 65_535;
/// Most bytes one real number in a DICT may take.
const MAX_REAL_BYTES: usize = 32;
/// The CIDCount a CID-keyed CFF table implies when it states none.
const DEFAULT_CID_COUNT: usize = 8_720;

const OP_CHARSET: u16 = 15;
const OP_CHARSTRINGS: u16 = 17;
const OP_PRIVATE: u16 = 18;
const OP_SUBRS: u16 = 19;
const OP_DEFAULT_WIDTH: u16 = 20;
const OP_NOMINAL_WIDTH: u16 = 21;
const OP_VSINDEX: u16 = 22;
const OP_VSTORE: u16 = 24;
const OP_MAXSTACK: u16 = 25;
const OP_FONT_BBOX: u16 = 5;
const OP_ROS: u16 = 1230;
const OP_CID_COUNT: u16 = 1234;
const OP_FDARRAY: u16 = 1236;
const OP_FDSELECT: u16 = 1237;
/// The Private DICT lists stored as differences: BlueValues, OtherBlues, FamilyBlues,
/// FamilyOtherBlues, StemSnapH, StemSnapV.
const DELTA_OPS: [u16; 6] = [6, 7, 8, 9, 1212, 1213];
/// Top DICT entries a CFF table may not carry over from CFF2.
const CFF2_ONLY_TOP: [u16; 1] = [OP_MAXSTACK];

fn invalid(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

fn in_glyph(e: TablesError, gid: usize) -> TablesError {
    match e {
        TablesError::Invalid(m) => TablesError::Invalid(format!("glyph {gid}: {m}")),
        other => other,
    }
}

/// Python's `round`: halves go to the even neighbour.
pub(super) fn round_half_even(v: f64) -> f64 {
    v.round_ties_even()
}

/// A number of a charstring or DICT: its value at the default and, when it varies, the index of
/// the [`Item`] holding its deltas.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Val {
    v: f64,
    item: Option<usize>,
}

impl Val {
    fn plain(v: f64) -> Self {
        Val { v, item: None }
    }
}

/// A value a `blend` made: the variation-store subtable (`vsindex`) of its deltas and one delta
/// per region of that subtable.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Item {
    pub major: usize,
    pub deltas: Vec<f64>,
}

/// One charstring operator with its operands. `op` is the operator byte, or 1200 plus the second
/// byte of an escaped one; `mask` holds the bytes after `hintmask` and `cntrmask`.
#[derive(Clone, Debug, PartialEq)]
struct Cmd {
    op: u16,
    mask: Vec<u8>,
    args: Vec<Val>,
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    op: u16,
    raw: Vec<u8>,
    vals: Vec<Val>,
    blended: bool,
}

#[derive(Clone, Debug)]
struct Private {
    entries: Vec<Entry>,
    vsindex: usize,
}

/// A `CFF2` table read for instancing.
#[derive(Clone, Debug)]
pub(super) struct Table {
    top: Vec<Entry>,
    font_dicts: Vec<Vec<Entry>>,
    /// The Font DICT of each glyph and the format it was stored in; `None` without FDSelect.
    fd_select: Option<(Vec<u16>, u8)>,
    privates: Vec<Private>,
    glyphs: Vec<Vec<Cmd>>,
    stack_limit: usize,
    /// The variation store's regions, each one (start, peak, end) per axis.
    pub regions: Vec<Vec<(f64, f64, f64)>>,
    /// The region indexes of each variation-store subtable.
    pub datas: Vec<Vec<u16>>,
    pub items: Vec<Item>,
}

/// What an instance makes of the items: the value each adds to its default (rounded half to even
/// here), for a partial instance their new deltas, and the new index of each variation-store
/// subtable (`None` for one left without regions, whose values stop varying).
pub(super) struct Resolved<'a> {
    pub add: &'a [f64],
    pub deltas: Option<&'a [Vec<f64>]>,
    pub majors: &'a [Option<usize>],
}

impl Table {
    /// Reads a `CFF2` table whose variation store has `axis_count` axes.
    pub fn read(data: &[u8], axis_count: usize) -> Result<Self> {
        let major = *data.first().ok_or_else(|| invalid("the CFF2 table is empty"))?;
        if major != 2 {
            return Err(TablesError::Unsupported(format!("CFF2 major version {major}")));
        }
        let hdr_size = usize::from(*data.get(2).ok_or_else(|| invalid("the CFF2 header is cut short"))?);
        let top_len = usize::from(read_u16(data, 3).ok_or_else(|| invalid("the CFF2 header is cut short"))?);
        if hdr_size < 5 {
            return Err(invalid("the CFF2 header is cut short"));
        }
        let top_bytes = blob(data, hdr_size, top_len)?;
        let mut items: Vec<Item> = Vec::new();
        let mut budget = ItemBudget::default();
        let top = parse_dict(top_bytes, None, &mut items, &mut budget)?;
        if top.iter().any(|e| e.blended) {
            return Err(invalid("the CFF2 Top DICT holds a blend"));
        }
        let global = read_index(data, hdr_size + top_len, true)?.items;
        let offset = |op: u16| -> Result<Option<usize>> {
            match top.iter().find(|e| e.op == op) {
                None => Ok(None),
                Some(e) => {
                    let v = e.vals.last().map(|v| v.v).ok_or_else(|| invalid("a Top DICT offset has no operand"))?;
                    if v < 0.0 || v.fract() != 0.0 || v > MAX_SFNT as f64 {
                        return Err(invalid("a Top DICT offset is not a positive integer"));
                    }
                    Ok(Some(v as usize))
                }
            }
        };
        let stack_limit = match top.iter().find(|e| e.op == OP_MAXSTACK).and_then(|e| e.vals.first()) {
            Some(v) if v.v.is_finite() && v.v > MAX_CFF2_STACK as f64 => (v.v as usize).min(u16::MAX as usize),
            _ => MAX_CFF2_STACK,
        };

        let (regions, datas) = match offset(OP_VSTORE)? {
            Some(at) => {
                let len = usize::from(read_u16(data, at).ok_or_else(|| invalid("the CFF2 variation store is cut short"))?);
                read_store(blob(data, at + 2, len)?, axis_count)?
            }
            None => (Vec::new(), Vec::new()),
        };
        let region_counts: Vec<usize> = datas.iter().map(Vec::len).collect();

        let cs_at = offset(OP_CHARSTRINGS)?.ok_or_else(|| invalid("the CFF2 Top DICT has no CharStrings"))?;
        let charstrings = read_index(data, cs_at, true)?.items;
        if charstrings.is_empty() || charstrings.len() > MAX_GLYPHS {
            return Err(invalid(format!("the CharStrings INDEX holds {} glyphs", charstrings.len())));
        }
        let fd_at = offset(OP_FDARRAY)?.ok_or_else(|| invalid("the CFF2 Top DICT has no FDArray"))?;
        let fds = read_index(data, fd_at, true)?.items;
        if fds.is_empty() || fds.len() > MAX_FONT_DICTS {
            return Err(invalid(format!("the FDArray holds {} Font DICTs", fds.len())));
        }
        let mut font_dicts = Vec::with_capacity(fds.len());
        let mut privates = Vec::with_capacity(fds.len());
        let mut local_subrs: Vec<Vec<&[u8]>> = Vec::with_capacity(fds.len());
        for fd in fds {
            let entries = parse_dict(fd, None, &mut items, &mut budget)?;
            if entries.iter().any(|e| e.blended) {
                return Err(invalid("a CFF2 Font DICT holds a blend"));
            }
            let (size, at) = match entries.iter().find(|e| e.op == OP_PRIVATE) {
                Some(e) => match (e.vals.first(), e.vals.get(1)) {
                    (Some(s), Some(a)) if s.v >= 0.0 && a.v >= 0.0 && s.v <= MAX_SFNT as f64 && a.v <= MAX_SFNT as f64 => {
                        (s.v as usize, a.v as usize)
                    }
                    _ => return Err(invalid("a Private entry needs a size and an offset")),
                },
                None => (0, 0),
            };
            let bytes = blob(data, at, size)?;
            let vsindex = private_vsindex(bytes)?;
            let blend_with = if datas.is_empty() {
                None
            } else {
                Some((*region_counts.get(vsindex).ok_or_else(|| invalid(format!("vsindex {vsindex} does not exist")))?, vsindex))
            };
            let entries_p = parse_dict(bytes, blend_with, &mut items, &mut budget)?;
            let mut subrs = Vec::new();
            if let Some(rel) = entries_p.iter().find(|e| e.op == OP_SUBRS).and_then(|e| e.vals.first()) {
                if rel.v < 0.0 || rel.v > MAX_SFNT as f64 {
                    return Err(invalid("a Subrs offset is out of range"));
                }
                subrs = read_index(data, at + rel.v as usize, true)?.items;
            }
            privates.push(Private { entries: entries_p, vsindex });
            local_subrs.push(subrs);
            font_dicts.push(entries);
        }
        let fd_select = match offset(OP_FDSELECT)? {
            Some(at) => Some(read_fd_select(data, at, charstrings.len())?),
            None => None,
        };

        let mut glyphs = Vec::with_capacity(charstrings.len());
        let mut ops_total = 0usize;
        for (gid, cs) in charstrings.iter().enumerate() {
            let fd = fd_select.as_ref().and_then(|(m, _)| m.get(gid).copied()).map_or(0, usize::from);
            let private = privates.get(fd).ok_or_else(|| invalid(format!("glyph {gid} names Font DICT {fd}, which does not exist")))?;
            let local = local_subrs.get(fd).map(Vec::as_slice).unwrap_or_default();
            let mut run = Run {
                global: &global,
                local,
                region_counts: &region_counts,
                vsindex: private.vsindex,
                stack: Vec::new(),
                stems: 0,
                cmds: Vec::new(),
                ops: 0,
                stack_limit,
                items: &mut items,
                budget: &mut budget,
            };
            run.run(cs, 0).map_err(|e| in_glyph(e, gid))?;
            if !run.stack.is_empty() {
                return Err(in_glyph(invalid("operands are left at the end of the charstring"), gid));
            }
            ops_total = ops_total.saturating_add(run.ops);
            if ops_total > MAX_OPS_PER_TABLE {
                return Err(TablesError::TooLarge { what: "the operators of the CFF2 charstrings", limit: MAX_OPS_PER_TABLE });
            }
            glyphs.push(run.cmds);
        }
        Ok(Table { top, font_dicts, fd_select, privates, glyphs, stack_limit, regions, datas, items })
    }

    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// Applies an instance to every blended value.
    pub fn resolve(&mut self, r: &Resolved<'_>) -> Result<()> {
        let items = &self.items;
        let apply = |val: &mut Val| {
            let Some(i) = val.item else { return };
            val.v += round_half_even(r.add.get(i).copied().unwrap_or(0.0));
            let major = items.get(i).and_then(|it| r.majors.get(it.major).copied().flatten());
            let kept = r.deltas.is_some() && major.is_some();
            if !kept {
                val.item = None;
            }
        };
        for g in &mut self.glyphs {
            for c in g.iter_mut() {
                for v in &mut c.args {
                    apply(v);
                }
            }
        }
        for p in &mut self.privates {
            for e in p.entries.iter_mut().filter(|e| e.blended) {
                let delta_list = DELTA_OPS.contains(&e.op);
                if delta_list {
                    let mut acc = 0.0;
                    for v in &mut e.vals {
                        acc += v.v;
                        v.v = acc;
                    }
                }
                for v in &mut e.vals {
                    apply(v);
                }
                if delta_list {
                    let mut prev = 0.0;
                    for v in &mut e.vals {
                        let abs = v.v;
                        v.v = abs - prev;
                        prev = abs;
                    }
                }
            }
        }
        let mut new_items = Vec::with_capacity(self.items.len());
        for (i, it) in self.items.iter().enumerate() {
            let major = r.majors.get(it.major).copied().flatten();
            let deltas = match (major, r.deltas) {
                (Some(_), Some(d)) => d.get(i).cloned().unwrap_or_default(),
                _ => Vec::new(),
            };
            new_items.push(Item { major: major.unwrap_or(0), deltas });
        }
        self.items = new_items;
        for p in &mut self.privates {
            let old = p.vsindex;
            p.vsindex = self.datas.get(old).and(r.majors.get(old).copied().flatten()).unwrap_or(0);
            let still = r.majors.get(old).copied().flatten().is_some() && r.deltas.is_some();
            for e in &mut p.entries {
                if e.op == OP_VSINDEX {
                    if still {
                        e.raw = dict_int(p.vsindex as i64);
                        e.vals = vec![Val::plain(p.vsindex as f64)];
                    } else {
                        e.op = u16::MAX;
                    }
                }
            }
            p.entries.retain(|e| e.op != u16::MAX);
        }
        Ok(())
    }

    /// The `CFF2` table: no subroutines, the variation store `store` (item variation store bytes;
    /// `None` drops it), every offset a five-byte integer.
    pub fn write_cff2(&self, store: Option<&[u8]>) -> Result<Vec<u8>> {
        let mut charstrings = Vec::with_capacity(self.glyphs.len());
        for (gid, g) in self.glyphs.iter().enumerate() {
            let vsindex = self.glyph_private(gid)?.vsindex;
            charstrings.push(self.encode_cff2_glyph(g, vsindex).map_err(|e| in_glyph(e, gid))?);
        }
        let privates: Vec<Vec<u8>> =
            self.privates.iter().map(|p| encode_private(&p.entries, &self.items, &[OP_SUBRS], &[])).collect::<Result<_>>()?;
        let vstore = match store {
            Some(s) => {
                let len = u16::try_from(s.len()).map_err(|_| TablesError::TooLarge { what: "the CFF2 variation store", limit: 65_535 })?;
                let mut v = len.to_be_bytes().to_vec();
                v.extend_from_slice(s);
                Some(v)
            }
            None => None,
        };
        let fd_select = self.fd_select.as_ref().map(|(map, format)| write_fd_select(map, *format, true)).transpose()?;

        let other_top: Vec<&Entry> = self.top.iter().filter(|e| ![OP_CHARSTRINGS, OP_FDARRAY, OP_FDSELECT, OP_VSTORE].contains(&e.op)).collect();
        let mut top_len = other_top.iter().map(|e| e.raw.len() + op_len(e.op)).sum::<usize>() + 6 + 7;
        if fd_select.is_some() {
            top_len += 7;
        }
        if vstore.is_some() {
            top_len += 6;
        }
        let fd_dicts_len: Vec<usize> = self.font_dicts.iter().map(|d| fd_dict_len(d)).collect();
        let fdarray_len = index_len(fd_dicts_len.iter().copied(), true);
        let cs_len = index_len(charstrings.iter().map(Vec::len), true);

        let mut pos = 5 + top_len + 4;
        let at_vstore = pos;
        pos += vstore.as_ref().map_or(0, Vec::len);
        let at_fdselect = pos;
        pos += fd_select.as_ref().map_or(0, Vec::len);
        let at_cs = pos;
        pos += cs_len;
        let at_fdarray = pos;
        pos += fdarray_len;
        let mut at_privates = Vec::with_capacity(privates.len());
        for p in &privates {
            at_privates.push(pos);
            pos += p.len();
        }
        if pos > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the CFF2 table", limit: MAX_SFNT });
        }

        let mut top = Vec::with_capacity(top_len);
        for e in &other_top {
            top.extend_from_slice(&e.raw);
            put_op(&mut top, e.op);
        }
        put_offset(&mut top, at_cs)?;
        put_op(&mut top, OP_CHARSTRINGS);
        if fd_select.is_some() {
            put_offset(&mut top, at_fdselect)?;
            put_op(&mut top, OP_FDSELECT);
        }
        put_offset(&mut top, at_fdarray)?;
        put_op(&mut top, OP_FDARRAY);
        if vstore.is_some() {
            put_offset(&mut top, at_vstore)?;
            put_op(&mut top, OP_VSTORE);
        }
        let top16 = u16::try_from(top.len()).map_err(|_| TablesError::TooLarge { what: "the CFF2 Top DICT", limit: 65_535 })?;
        let mut out = Vec::with_capacity(pos);
        out.extend_from_slice(&[2, 0, 5]);
        out.extend_from_slice(&top16.to_be_bytes());
        out.extend_from_slice(&top);
        write_index(&mut out, std::iter::empty::<&[u8]>(), true)?;
        if let Some(v) = &vstore {
            out.extend_from_slice(v);
        }
        if let Some(f) = &fd_select {
            out.extend_from_slice(f);
        }
        write_index(&mut out, charstrings.iter().map(Vec::as_slice), true)?;
        let fd_bytes: Vec<Vec<u8>> = self
            .font_dicts
            .iter()
            .enumerate()
            .map(|(i, d)| fd_dict(d, privates.get(i).map_or(0, Vec::len), at_privates.get(i).copied().unwrap_or(0)))
            .collect::<Result<_>>()?;
        write_index(&mut out, fd_bytes.iter().map(Vec::as_slice), true)?;
        for p in &privates {
            out.extend_from_slice(p);
        }
        if out.len() != pos {
            return Err(invalid("the CFF2 layout did not come out as planned"));
        }
        Ok(out)
    }

    /// The CID-keyed `CFF ` table of a table with nothing left to vary. `advances` are the
    /// `hmtx` advances by glyph id; `font_bbox` goes into the Top DICT.
    pub fn write_cff(&self, advances: &[u16], font_bbox: [i64; 4]) -> Result<Vec<u8>> {
        let varies = |v: &Val| v.item.and_then(|i| self.items.get(i)).is_some_and(|it| !it.deltas.is_empty());
        if self.glyphs.iter().flatten().any(|c| c.args.iter().any(varies))
            || self.privates.iter().any(|p| p.entries.iter().any(|e| e.vals.iter().any(varies)))
        {
            return Err(invalid("a CFF table cannot hold values that still vary"));
        }
        let n = self.glyphs.len();
        let fd_of = |gid: usize| self.fd_select.as_ref().and_then(|(m, _)| m.get(gid).copied()).map_or(0usize, usize::from);
        // Each Font DICT's width pair, from the advances of its glyphs.
        let mut widths: Vec<(i64, i64)> = Vec::with_capacity(self.privates.len());
        for fd in 0..self.privates.len() {
            let list: Vec<i64> = (0..n).filter(|g| fd_of(*g) == fd).map(|g| i64::from(advances.get(g).copied().unwrap_or(0))).collect();
            widths.push(if list.is_empty() { (0, 0) } else { fontcraft_cff::optimize_widths(&list).unwrap_or((0, 0)) });
        }
        let mut charstrings = Vec::with_capacity(n);
        for (gid, g) in self.glyphs.iter().enumerate() {
            let (default, nominal) = widths.get(fd_of(gid)).copied().unwrap_or((0, 0));
            let advance = i64::from(advances.get(gid).copied().unwrap_or(0));
            let width = (advance != default).then_some((advance - nominal) as f64);
            charstrings.push(encode_cff_glyph(g, width).map_err(|e| in_glyph(e, gid))?);
        }
        let privates: Vec<Vec<u8>> = self
            .privates
            .iter()
            .zip(&widths)
            .map(|(p, (d, nom))| {
                let mut extra = Vec::new();
                for (op, v) in [(OP_DEFAULT_WIDTH, *d), (OP_NOMINAL_WIDTH, *nom)] {
                    if v != 0 {
                        extra.push((op, v));
                    }
                }
                encode_private(&p.entries, &self.items, &[OP_SUBRS, OP_VSINDEX, OP_DEFAULT_WIDTH, OP_NOMINAL_WIDTH], &extra)
            })
            .collect::<Result<_>>()?;
        let fd_map: Vec<u16> = match &self.fd_select {
            Some((m, _)) => m.clone(),
            None => vec![0; n],
        };
        let format = match &self.fd_select {
            Some((_, f @ (0 | 3))) => *f,
            _ => 255,
        };
        let fd_select = write_fd_select(&fd_map, format, false)?;
        let charset = cid_charset(n);
        let strings: [&[u8]; 2] = [b"Adobe", b"Identity"]; // brand-ok: the CFF registry value
        let name: &[u8] = b"CFF2Font";

        let mut top_fixed: Vec<u8> = Vec::new();
        // ROS first, as a CID-keyed font needs.
        top_fixed.extend(dict_int(391));
        top_fixed.extend(dict_int(392));
        top_fixed.extend(dict_int(0));
        put_op(&mut top_fixed, OP_ROS);
        for e in self.top.iter().filter(|e| ![OP_CHARSTRINGS, OP_FDARRAY, OP_FDSELECT, OP_VSTORE].contains(&e.op) && !CFF2_ONLY_TOP.contains(&e.op)) {
            top_fixed.extend_from_slice(&e.raw);
            put_op(&mut top_fixed, e.op);
        }
        if font_bbox != [0; 4] {
            for v in font_bbox {
                top_fixed.extend(dict_int(v));
            }
            put_op(&mut top_fixed, OP_FONT_BBOX);
        }
        if n > DEFAULT_CID_COUNT {
            top_fixed.extend(dict_int(n as i64));
            put_op(&mut top_fixed, OP_CID_COUNT);
        }
        // charset, FDSelect, CharStrings and FDArray, each a five-byte offset and its operator.
        let top_len = top_fixed.len() + 6 + 7 + 6 + 7;

        let fd_dicts_len: Vec<usize> = self.font_dicts.iter().map(|d| fd_dict_len(d)).collect();
        let fdarray_len = index_len(fd_dicts_len.iter().copied(), false);
        let cs_len = index_len(charstrings.iter().map(Vec::len), false);
        let mut pos = 4 + index_len([name.len()].into_iter(), false) + index_len([top_len].into_iter(), false);
        pos += index_len(strings.iter().map(|s| s.len()), false) + 2;
        let at_charset = pos;
        pos += charset.len();
        let at_fdselect = pos;
        pos += fd_select.len();
        let at_cs = pos;
        pos += cs_len;
        let at_fdarray = pos;
        pos += fdarray_len;
        let mut at_privates = Vec::with_capacity(privates.len());
        for p in &privates {
            at_privates.push(pos);
            pos += p.len();
        }
        if pos > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the CFF table", limit: MAX_SFNT });
        }
        let mut top = top_fixed;
        put_offset(&mut top, at_charset)?;
        put_op(&mut top, OP_CHARSET);
        put_offset(&mut top, at_fdselect)?;
        put_op(&mut top, OP_FDSELECT);
        put_offset(&mut top, at_cs)?;
        put_op(&mut top, OP_CHARSTRINGS);
        put_offset(&mut top, at_fdarray)?;
        put_op(&mut top, OP_FDARRAY);

        let mut out = Vec::with_capacity(pos);
        out.extend_from_slice(&[1, 0, 4, 4]);
        write_index(&mut out, [name].into_iter(), false)?;
        write_index(&mut out, [top.as_slice()].into_iter(), false)?;
        write_index(&mut out, strings.into_iter(), false)?;
        write_index(&mut out, std::iter::empty::<&[u8]>(), false)?;
        out.extend_from_slice(&charset);
        out.extend_from_slice(&fd_select);
        write_index(&mut out, charstrings.iter().map(Vec::as_slice), false)?;
        let fd_bytes: Vec<Vec<u8>> = self
            .font_dicts
            .iter()
            .enumerate()
            .map(|(i, d)| fd_dict(d, privates.get(i).map_or(0, Vec::len), at_privates.get(i).copied().unwrap_or(0)))
            .collect::<Result<_>>()?;
        write_index(&mut out, fd_bytes.iter().map(Vec::as_slice), false)?;
        for p in &privates {
            out.extend_from_slice(p);
        }
        if out.len() != pos {
            return Err(invalid("the CFF layout did not come out as planned"));
        }
        Ok(out)
    }

    fn glyph_private(&self, gid: usize) -> Result<&Private> {
        let fd = self.fd_select.as_ref().and_then(|(m, _)| m.get(gid).copied()).map_or(0, usize::from);
        self.privates.get(fd).ok_or_else(|| invalid(format!("glyph {gid} names Font DICT {fd}, which does not exist")))
    }

    /// A CFF2 charstring: `vsindex` first when the glyph's blends use another subtable than its
    /// Private DICT's, runs of blended operands written as `blend`s that fit the stack.
    fn encode_cff2_glyph(&self, cmds: &[Cmd], private_vsindex: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut major: Option<usize> = None;
        for c in cmds {
            for a in c.args.iter().filter_map(|a| a.item) {
                let m = self.items.get(a).map_or(0, |i| i.major);
                match major {
                    None => major = Some(m),
                    Some(x) if x != m => return Err(TablesError::Unsupported("a charstring whose blends use two vsindex values".into())),
                    Some(_) => {}
                }
            }
        }
        if let Some(m) = major
            && m != private_vsindex
        {
            put_cs_number(&mut out, m as f64)?;
            out.push(15);
        }
        for c in cmds {
            let mut depth = 0usize;
            let mut i = 0usize;
            while let Some(a) = c.args.get(i) {
                let Some(item) = a.item else {
                    put_cs_number(&mut out, a.v)?;
                    depth += 1;
                    i += 1;
                    continue;
                };
                let k = self.items.get(item).map_or(0, |it| it.deltas.len());
                let mut run: Vec<&Val> = vec![a];
                while let Some(b) = c.args.get(i + run.len()).filter(|b| b.item.is_some()) {
                    if depth + (run.len() + 1) * (k + 1) + 1 > self.stack_limit {
                        break;
                    }
                    run.push(b);
                }
                if depth + run.len() * (k + 1) + 1 > self.stack_limit {
                    return Err(TablesError::TooLarge { what: "the operands of one blend", limit: self.stack_limit });
                }
                for v in &run {
                    put_cs_number(&mut out, v.v)?;
                }
                for v in &run {
                    let deltas = v.item.and_then(|x| self.items.get(x)).map(|it| it.deltas.as_slice()).unwrap_or_default();
                    for j in 0..k {
                        put_cs_number(&mut out, deltas.get(j).copied().unwrap_or(0.0))?;
                    }
                }
                put_cs_number(&mut out, run.len() as f64)?;
                out.push(16);
                depth += run.len();
                i += run.len();
            }
            put_op(&mut out, c.op);
            out.extend_from_slice(&c.mask);
            if out.len() > MAX_CHARSTRING {
                return Err(TablesError::TooLarge { what: "a charstring without subroutines", limit: MAX_CHARSTRING });
            }
        }
        Ok(out)
    }
}

// ---------- charstrings ----------

#[derive(Default)]
struct ItemBudget {
    deltas: usize,
}

struct Run<'a, 'b> {
    global: &'a [&'a [u8]],
    local: &'a [&'a [u8]],
    region_counts: &'a [usize],
    vsindex: usize,
    stack: Vec<Val>,
    stems: usize,
    cmds: Vec<Cmd>,
    ops: usize,
    stack_limit: usize,
    items: &'b mut Vec<Item>,
    budget: &'b mut ItemBudget,
}

impl Run<'_, '_> {
    fn run(&mut self, code: &[u8], level: usize) -> Result<()> {
        if level > MAX_SUBR_NESTING {
            return Err(invalid(format!("subroutines nest deeper than {MAX_SUBR_NESTING}")));
        }
        let cut = || invalid("a charstring is cut short");
        let mut i = 0usize;
        while let Some(&b0) = code.get(i) {
            self.ops += 1;
            if self.ops > MAX_OPS_PER_GLYPH {
                return Err(TablesError::TooLarge { what: "the operators one glyph runs", limit: MAX_OPS_PER_GLYPH });
            }
            match b0 {
                28 => {
                    let v = read_u16(code, i + 1).ok_or_else(cut)? as i16;
                    self.push(f64::from(v))?;
                    i += 3;
                }
                32..=246 => {
                    self.push(f64::from(b0) - 139.0)?;
                    i += 1;
                }
                247..=254 => {
                    let b1 = f64::from(*code.get(i + 1).ok_or_else(cut)?);
                    let v = if b0 <= 250 { (f64::from(b0) - 247.0) * 256.0 + b1 + 108.0 } else { -(f64::from(b0) - 251.0) * 256.0 - b1 - 108.0 };
                    self.push(v)?;
                    i += 2;
                }
                255 => {
                    let v = read_u32(code, i + 1).ok_or_else(cut)? as i32;
                    self.push(f64::from(v) / 65536.0)?;
                    i += 5;
                }
                12 => {
                    let b1 = *code.get(i + 1).ok_or_else(cut)?;
                    i += 2;
                    match b1 {
                        // dotsection and the flex operators
                        0 | 34..=37 => self.emit(1200 + u16::from(b1), Vec::new()),
                        3..=5 | 9..=12 | 14 | 15 | 18 | 20..=24 | 26..=30 => {
                            return Err(TablesError::Unsupported(format!("the charstring arithmetic operator 12 {b1}")));
                        }
                        other => return Err(invalid(format!("operator 12 {other} is reserved"))),
                    }
                }
                1 | 3 | 18 | 23 => {
                    self.stems += self.stack.len() / 2;
                    self.emit(u16::from(b0), Vec::new());
                    i += 1;
                }
                19 | 20 => {
                    self.stems += self.stack.len() / 2;
                    let n = self.stems.div_ceil(8);
                    let mask = code.get(i + 1..i + 1 + n).ok_or_else(|| invalid("a hint mask is cut short"))?.to_vec();
                    self.emit(u16::from(b0), mask);
                    i += 1 + n;
                }
                10 | 29 => {
                    i += 1;
                    let v = self.pop_int()?;
                    let subrs = if b0 == 10 { self.local } else { self.global };
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
                15 => {
                    i += 1;
                    let v = self.pop_int()?;
                    let v = usize::try_from(v).map_err(|_| invalid("a negative vsindex"))?;
                    if v >= self.region_counts.len() {
                        return Err(invalid(format!("vsindex {v} does not exist")));
                    }
                    self.vsindex = v;
                }
                16 => {
                    i += 1;
                    let n = usize::try_from(self.pop_int()?).map_err(|_| invalid("a negative blend count"))?;
                    let k = *self.region_counts.get(self.vsindex).ok_or_else(|| invalid(format!("vsindex {} does not exist", self.vsindex)))?;
                    blend(&mut self.stack, n, k, self.vsindex, self.items, self.budget)?;
                }
                4 | 5 | 6 | 7 | 8 | 21 | 22 | 24 | 25 | 26 | 27 | 30 | 31 => {
                    self.emit(u16::from(b0), Vec::new());
                    i += 1;
                }
                other => return Err(invalid(format!("operator {other} is reserved in CFF2"))),
            }
        }
        Ok(())
    }

    fn push(&mut self, v: f64) -> Result<()> {
        if self.stack.len() >= self.stack_limit {
            return Err(invalid(format!("more than {} operands on the stack", self.stack_limit)));
        }
        self.stack.push(Val::plain(v));
        Ok(())
    }

    fn pop_int(&mut self) -> Result<i64> {
        match self.stack.pop() {
            Some(Val { v, item: None }) if v.fract() == 0.0 && v.abs() < 1e9 => Ok(v as i64),
            _ => Err(invalid("an operator needs an integer operand")),
        }
    }

    fn emit(&mut self, op: u16, mask: Vec<u8>) {
        let args = std::mem::take(&mut self.stack);
        self.cmds.push(Cmd { op, mask, args });
    }
}

/// Replaces the top `n * (k + 1)` operands (n defaults, then k deltas for each) by n blended
/// values.
fn blend(stack: &mut Vec<Val>, n: usize, k: usize, major: usize, items: &mut Vec<Item>, budget: &mut ItemBudget) -> Result<()> {
    let used = n.checked_mul(k + 1).filter(|u| *u <= stack.len()).ok_or_else(|| invalid("blend has too few operands"))?;
    let start = stack.len() - used;
    let args: Vec<Val> = stack.split_off(start);
    if args.iter().any(|a| a.item.is_some()) {
        return Err(TablesError::Unsupported("a blend of blended values".into()));
    }
    if items.len().saturating_add(n) > MAX_ITEMS {
        return Err(TablesError::TooLarge { what: "the blended values of the CFF2 table", limit: MAX_ITEMS });
    }
    budget.deltas = budget.deltas.saturating_add(n.saturating_mul(k));
    if budget.deltas > MAX_ITEM_DELTAS {
        return Err(TablesError::TooLarge { what: "the deltas of the CFF2 table", limit: MAX_ITEM_DELTAS });
    }
    for j in 0..n {
        let default = args.get(j).map_or(0.0, |a| a.v);
        let deltas: Vec<f64> = (0..k).map(|r| args.get(n + j * k + r).map_or(0.0, |a| a.v)).collect();
        stack.push(Val { v: default, item: Some(items.len()) });
        items.push(Item { major, deltas });
    }
    Ok(())
}

/// A CFF charstring: the width first when given, operators with more operands than the stack
/// holds (the width included) split into generic segments, `endchar` last.
fn encode_cff_glyph(cmds: &[Cmd], width: Option<f64>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut width = width;
    for c in cmds {
        let extra = usize::from(width.is_some());
        if c.args.len() + extra > MAX_CFF_STACK {
            let segs = generalize(c.op, &c.args)?.ok_or_else(|| {
                TablesError::Unsupported(format!("a CFF charstring operator with {} operands, more than the stack holds", c.args.len()))
            })?;
            if let Some(w) = width.take() {
                put_cs_number(&mut out, w)?;
            }
            write_segments(&mut out, &segs)?;
        } else {
            if let Some(w) = width.take() {
                put_cs_number(&mut out, w)?;
            }
            for a in &c.args {
                put_cs_number(&mut out, a.v)?;
            }
            put_op(&mut out, c.op);
            out.extend_from_slice(&c.mask);
        }
        if out.len() > MAX_CHARSTRING {
            return Err(TablesError::TooLarge { what: "a charstring without subroutines", limit: MAX_CHARSTRING });
        }
    }
    if let Some(w) = width {
        put_cs_number(&mut out, w)?;
    }
    out.push(14);
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RelSeg {
    Line(f64, f64),
    Curve([f64; 6]),
}

/// The relative segments of a path operator; `None` for an operator that is not one.
fn generalize(op: u16, args: &[Val]) -> Result<Option<Vec<RelSeg>>> {
    let a: Vec<f64> = args.iter().map(|v| v.v).collect();
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
            for v in &a {
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
            if a.len() < 8 || !(a.len() - 6).is_multiple_of(2) {
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

/// Writes segments as `rlineto` and `rrcurveto` runs of at most 48 operands.
fn write_segments(out: &mut Vec<u8>, segs: &[RelSeg]) -> Result<()> {
    let mut i = 0usize;
    while let Some(first) = segs.get(i) {
        let is_line = matches!(first, RelSeg::Line(..));
        let per = if is_line { 2 } else { 6 };
        let mut count = 0usize;
        while let Some(s) = segs.get(i) {
            if matches!(s, RelSeg::Line(..)) != is_line || (count + 1) * per > MAX_CFF_STACK {
                break;
            }
            match s {
                RelSeg::Line(x, y) => {
                    put_cs_number(out, *x)?;
                    put_cs_number(out, *y)?;
                }
                RelSeg::Curve(c) => {
                    for v in c {
                        put_cs_number(out, *v)?;
                    }
                }
            }
            count += 1;
            i += 1;
        }
        out.push(if is_line { 5 } else { 8 });
    }
    Ok(())
}

// ---------- numbers and DICTs ----------

fn put_cs_number(out: &mut Vec<u8>, v: f64) -> Result<()> {
    if !v.is_finite() {
        return Err(invalid("a charstring number is not finite"));
    }
    if v.fract() == 0.0 && (-32768.0..=32767.0).contains(&v) {
        let n = v as i64;
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
            _ => {
                out.push(28);
                out.extend_from_slice(&(n as i16).to_be_bytes());
            }
        }
        return Ok(());
    }
    let fixed = (v * 65536.0).round();
    if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&fixed) {
        return Err(invalid(format!("the number {v} does not fit in a charstring")));
    }
    out.push(255);
    out.extend_from_slice(&(fixed as i32).to_be_bytes());
    Ok(())
}

fn dict_int(v: i64) -> Vec<u8> {
    match v {
        -107..=107 => vec![(v + 139) as u8],
        108..=1131 => {
            let m = v - 108;
            vec![(m / 256 + 247) as u8, (m % 256) as u8]
        }
        -1131..=-108 => {
            let m = -v - 108;
            vec![(m / 256 + 251) as u8, (m % 256) as u8]
        }
        -32768..=32767 => {
            let mut b = vec![28];
            b.extend_from_slice(&(v as i16).to_be_bytes());
            b
        }
        _ => {
            let mut b = vec![29];
            b.extend_from_slice(&(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32).to_be_bytes());
            b
        }
    }
}

/// A DICT number: an integer in the shortest form, anything else as a real.
fn dict_number(v: f64) -> Result<Vec<u8>> {
    if !v.is_finite() {
        return Err(invalid("a DICT number is not finite"));
    }
    if v.fract() == 0.0 && v.abs() <= f64::from(i32::MAX) {
        return Ok(dict_int(v as i64));
    }
    let text = format!("{v}");
    let mut nibbles: Vec<u8> = Vec::with_capacity(text.len() + 1);
    for ch in text.chars() {
        nibbles.push(match ch {
            '0'..='9' => ch as u8 - b'0',
            '.' => 0xA,
            '-' => 0xE,
            _ => return Err(invalid(format!("the number {v} cannot be written in a DICT"))),
        });
    }
    nibbles.push(0xF);
    if nibbles.len() % 2 == 1 {
        nibbles.push(0xF);
    }
    let mut out = vec![30];
    out.extend(nibbles.chunks(2).map(|p| (p.first().copied().unwrap_or(0xF) << 4) | p.get(1).copied().unwrap_or(0xF)));
    Ok(out)
}

fn op_len(op: u16) -> usize {
    if op >= 1200 { 2 } else { 1 }
}

fn put_op(out: &mut Vec<u8>, op: u16) {
    if op >= 1200 {
        out.extend_from_slice(&[12, (op - 1200) as u8]);
    } else {
        out.push(op as u8);
    }
}

fn put_offset(out: &mut Vec<u8>, v: usize) -> Result<()> {
    let v = i32::try_from(v).map_err(|_| invalid("a CFF offset does not fit in 32 bits"))?;
    out.push(29);
    out.extend_from_slice(&v.to_be_bytes());
    Ok(())
}

/// The `vsindex` of a Private DICT (0 when it states none).
fn private_vsindex(bytes: &[u8]) -> Result<usize> {
    let entries = parse_dict_skipping_blends(bytes)?;
    match entries.iter().find(|e| e.op == OP_VSINDEX).and_then(|e| e.vals.first()) {
        Some(v) if v.v >= 0.0 && v.v.fract() == 0.0 && v.v < 65_536.0 => Ok(v.v as usize),
        Some(_) => Err(invalid("a Private DICT vsindex is not a small positive integer")),
        None => Ok(0),
    }
}

fn parse_dict_skipping_blends(bytes: &[u8]) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut vals: Vec<Val> = Vec::new();
    while let Some(&b0) = bytes.get(i) {
        let (v, next) = match b0 {
            28 | 29 | 30 | 32..=254 => read_dict_number(bytes, i)?,
            23 => {
                vals.clear();
                i += 1;
                continue;
            }
            0..=27 => {
                let (op, next) = if b0 == 12 {
                    (1200 + u16::from(*bytes.get(i + 1).ok_or_else(|| invalid("a DICT is cut short"))?), i + 2)
                } else {
                    (u16::from(b0), i + 1)
                };
                if out.len() >= MAX_DICT_ENTRIES {
                    return Err(TablesError::TooLarge { what: "a DICT's entry count", limit: MAX_DICT_ENTRIES });
                }
                out.push(Entry { op, raw: Vec::new(), vals: std::mem::take(&mut vals), blended: false });
                i = next;
                continue;
            }
            _ => return Err(invalid(format!("byte {b0} is reserved in a DICT"))),
        };
        vals.push(Val::plain(v));
        if vals.len() > MAX_CFF2_STACK {
            return Err(invalid("a DICT entry has too many operands"));
        }
        i = next;
    }
    Ok(out)
}

/// A DICT number at `i`, and where the next token starts.
fn read_dict_number(bytes: &[u8], i: usize) -> Result<(f64, usize)> {
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

/// The entries of a DICT. `blend_with` is the region count and the subtable a `blend` uses;
/// `None` refuses `blend`.
fn parse_dict(bytes: &[u8], blend_with: Option<(usize, usize)>, items: &mut Vec<Item>, budget: &mut ItemBudget) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut vals: Vec<Val> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut blended = false;
    while let Some(&b0) = bytes.get(i) {
        match b0 {
            28 | 29 | 30 | 32..=254 => {
                let (v, next) = read_dict_number(bytes, i)?;
                vals.push(Val::plain(v));
                if vals.len() > MAX_CFF2_STACK {
                    return Err(invalid("a DICT entry has too many operands"));
                }
                i = next;
            }
            23 => {
                let (k, major) = blend_with.ok_or_else(|| invalid("a blend outside a Private DICT of a font with a variation store"))?;
                let n = match vals.pop() {
                    Some(Val { v, item: None }) if v >= 0.0 && v.fract() == 0.0 && v <= MAX_CFF2_STACK as f64 => v as usize,
                    _ => return Err(invalid("a DICT blend needs a count")),
                };
                blend(&mut vals, n, k, major, items, budget)?;
                blended = true;
                i += 1;
            }
            0..=27 => {
                let (op, next) = if b0 == 12 {
                    (1200 + u16::from(*bytes.get(i + 1).ok_or_else(|| invalid("a DICT is cut short"))?), i + 2)
                } else {
                    (u16::from(b0), i + 1)
                };
                if entries.len() >= MAX_DICT_ENTRIES {
                    return Err(TablesError::TooLarge { what: "a DICT's entry count", limit: MAX_DICT_ENTRIES });
                }
                let raw = bytes.get(start..i).unwrap_or_default().to_vec();
                entries.push(Entry { op, raw, vals: std::mem::take(&mut vals), blended });
                blended = false;
                i = next;
                start = i;
            }
            _ => return Err(invalid(format!("byte {b0} is reserved in a DICT"))),
        }
    }
    if !vals.is_empty() {
        return Err(invalid("a DICT ends with operands and no operator"));
    }
    Ok(entries)
}

/// A Private DICT: entries in `drop` left out, entries that held a blend written again (their
/// values resolved, and one `blend` for each run that still varies), then `extra` integer
/// entries.
fn encode_private(entries: &[Entry], items: &[Item], drop: &[u16], extra: &[(u16, i64)]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for e in entries.iter().filter(|e| !drop.contains(&e.op)) {
        if e.blended {
            out.extend(encode_dict_vals(&e.vals, items)?);
        } else {
            out.extend_from_slice(&e.raw);
        }
        put_op(&mut out, e.op);
    }
    for (op, v) in extra {
        out.extend(dict_int(*v));
        put_op(&mut out, *op);
    }
    Ok(out)
}

/// DICT operands. When any value still varies, every value of the entry goes into `blend`s (as
/// fontTools requires of a Private DICT entry), each holding as many values as the stack allows.
fn encode_dict_vals(vals: &[Val], items: &[Item]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let deltas_of = |v: &Val| v.item.and_then(|i| items.get(i)).map(|it| it.deltas.as_slice()).unwrap_or_default();
    let k = vals.iter().map(|v| deltas_of(v).len()).max().unwrap_or(0);
    if k == 0 {
        for v in vals {
            out.extend(dict_number(v.v)?);
        }
        return Ok(out);
    }
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < vals.len() {
        let room = MAX_CFF2_STACK.saturating_sub(depth + 1) / (k + 1);
        if room == 0 {
            return Err(TablesError::TooLarge { what: "the operands of one Private DICT blend", limit: MAX_CFF2_STACK });
        }
        let run = vals.get(i..(i + room).min(vals.len())).unwrap_or_default();
        for v in run {
            out.extend(dict_number(v.v)?);
        }
        for v in run {
            let d = deltas_of(v);
            for j in 0..k {
                out.extend(dict_number(d.get(j).copied().unwrap_or(0.0))?);
            }
        }
        out.extend(dict_int(run.len() as i64));
        out.push(23);
        depth += run.len();
        i += run.len();
    }
    Ok(out)
}

fn fd_dict_len(entries: &[Entry]) -> usize {
    entries.iter().filter(|e| e.op != OP_PRIVATE).map(|e| e.raw.len() + op_len(e.op)).sum::<usize>() + 11
}

fn fd_dict(entries: &[Entry], size: usize, at: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for e in entries.iter().filter(|e| e.op != OP_PRIVATE) {
        out.extend_from_slice(&e.raw);
        put_op(&mut out, e.op);
    }
    put_offset(&mut out, size)?;
    put_offset(&mut out, at)?;
    put_op(&mut out, OP_PRIVATE);
    Ok(out)
}

// ---------- structures ----------

fn blob(data: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    at.checked_add(len).and_then(|end| data.get(at..end)).ok_or_else(|| invalid("a CFF structure lies outside the table"))
}

struct Index<'a> {
    items: Vec<&'a [u8]>,
}

fn read_index(data: &[u8], at: usize, cff2: bool) -> Result<Index<'_>> {
    let cut = || invalid("an INDEX is cut short");
    let (count, head) = if cff2 { (read_u32(data, at).ok_or_else(cut)? as usize, 4) } else { (usize::from(read_u16(data, at).ok_or_else(cut)?), 2) };
    let start = at.checked_add(head).ok_or_else(cut)?;
    if count == 0 {
        return Ok(Index { items: Vec::new() });
    }
    let off_size = usize::from(*data.get(start).ok_or_else(cut)?);
    if !(1..=4).contains(&off_size) {
        return Err(invalid(format!("an INDEX has offset size {off_size}")));
    }
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
    Ok(Index { items })
}

fn off_size(last: usize) -> usize {
    match last {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    }
}

fn index_len(lens: impl Iterator<Item = usize> + Clone, cff2: bool) -> usize {
    let head = if cff2 { 4 } else { 2 };
    let count = lens.clone().count();
    if count == 0 {
        return head;
    }
    let total: usize = lens.sum();
    head + 1 + (count + 1) * off_size(total + 1) + total
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
    let put = |out: &mut Vec<u8>, v: usize| {
        let bytes = (v as u32).to_be_bytes();
        out.extend_from_slice(bytes.get(4 - size..).unwrap_or_default());
    };
    let mut offset = 1usize;
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

/// The Font DICT of every glyph and the FDSelect format.
fn read_fd_select(data: &[u8], at: usize, glyphs: usize) -> Result<(Vec<u16>, u8)> {
    let cut = || invalid("the FDSelect is cut short");
    let format = *data.get(at).ok_or_else(cut)?;
    match format {
        0 => Ok((data.get(at + 1..at + 1 + glyphs).ok_or_else(cut)?.iter().map(|b| u16::from(*b)).collect(), 0)),
        3 | 4 => {
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
            Ok((map, format))
        }
        other => Err(invalid(format!("FDSelect format {other}"))),
    }
}

/// An FDSelect in `format` (0, 3 or, in CFF2, 4); any other value picks the smaller of 0 and 3
/// (3 on a tie), as fontTools does for a table it builds.
fn write_fd_select(map: &[u16], format: u8, cff2: bool) -> Result<Vec<u8>> {
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
    if map.iter().any(|f| *f > 255) && (!cff2 || format != 4) {
        return Err(TablesError::Unsupported("more than 256 Font DICTs in a CFF table".into()));
    }
    let f3 = || -> Result<Vec<u8>> {
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
        0 => Ok(f0()),
        3 => f3(),
        4 if cff2 => {
            let mut v = vec![4u8];
            v.extend_from_slice(&(ranges.len() as u32).to_be_bytes());
            for (first, fd) in &ranges {
                v.extend_from_slice(&(*first as u32).to_be_bytes());
                v.extend_from_slice(&fd.to_be_bytes());
            }
            v.extend_from_slice(&(map.len() as u32).to_be_bytes());
            Ok(v)
        }
        _ => {
            let (a, b) = (f0(), f3()?);
            Ok(if a.len() < b.len() { a } else { b })
        }
    }
}

/// The charset of CIDs 1 to n−1 in order, the smaller of format 0 and one range, as fontTools
/// packs it.
fn cid_charset(n: usize) -> Vec<u8> {
    let mut ranged = Vec::new();
    if n >= 2 {
        let left = n - 2;
        let fmt = if left > 255 { 2u8 } else { 1 };
        ranged.push(fmt);
        ranged.extend_from_slice(&1u16.to_be_bytes());
        if fmt == 1 {
            ranged.push(left as u8);
        } else {
            ranged.extend_from_slice(&(left as u16).to_be_bytes());
        }
    } else {
        ranged.push(1);
    }
    let format0_len = 1 + 2 * n.saturating_sub(1);
    if format0_len < ranged.len() {
        let mut v = vec![0u8];
        for cid in 1..n {
            v.extend_from_slice(&(cid as u16).to_be_bytes());
        }
        v
    } else {
        ranged
    }
}

/// One region of a variation store: start, peak and end per axis.
type Region = Vec<(f64, f64, f64)>;

/// The regions (per axis start, peak, end) and each subtable's region indexes of the CFF2
/// variation store.
fn read_store(data: &[u8], axis_count: usize) -> Result<(Vec<Region>, Vec<Vec<u16>>)> {
    let store = ReadStore::read(FontData::new(data)).map_err(|e| invalid(format!("the CFF2 variation store cannot be read: {e}")))?;
    let list = store.variation_region_list().map_err(|e| invalid(format!("the CFF2 variation regions cannot be read: {e}")))?;
    if usize::from(list.axis_count()) != axis_count {
        return Err(invalid(format!("the CFF2 variation store has {} axes and the fvar table {axis_count}", list.axis_count())));
    }
    let f2 = |v: read_fonts::types::F2Dot14| f64::from(v.to_bits()) / 16384.0;
    let mut regions = Vec::new();
    for r in list.variation_regions().iter() {
        let r = r.map_err(|e| invalid(format!("a CFF2 variation region cannot be read: {e}")))?;
        regions.push(r.region_axes().iter().map(|a| (f2(a.start_coord()), f2(a.peak_coord()), f2(a.end_coord()))).collect());
    }
    let mut datas = Vec::new();
    for i in 0..store.item_variation_data_count() {
        match store.item_variation_data().get(usize::from(i)) {
            Some(Ok(sub)) => {
                let idx: Vec<u16> = sub.region_indexes().iter().map(|r| r.get()).collect();
                if idx.iter().any(|r| usize::from(*r) >= regions.len()) {
                    return Err(invalid(format!("CFF2 variation subtable {i} names a region the store lacks")));
                }
                datas.push(idx);
            }
            Some(Err(e)) => return Err(invalid(format!("CFF2 variation subtable {i} cannot be read: {e}"))),
            None => datas.push(Vec::new()),
        }
    }
    Ok((regions, datas))
}

fn read_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

// ---------- bounds ----------

#[derive(Default)]
struct BoundsSink {
    contours: Vec<Contour>,
    open: Option<Contour>,
    pos: Pt,
}

impl BoundsSink {
    fn begin(&mut self) -> &mut Contour {
        let start = self.pos;
        self.open.get_or_insert_with(|| Contour { start, segs: Vec::new() })
    }
}

fn fixed(v: read_fonts::types::Fixed) -> f64 {
    f64::from(v.to_bits()) / 65536.0
}

impl read_fonts::ps::cs::CommandSink for BoundsSink {
    fn move_to(&mut self, x: read_fonts::types::Fixed, y: read_fonts::types::Fixed) {
        self.close();
        self.pos = Pt { x: fixed(x), y: fixed(y) };
    }

    fn line_to(&mut self, x: read_fonts::types::Fixed, y: read_fonts::types::Fixed) {
        let (from, to) = (self.pos, Pt { x: fixed(x), y: fixed(y) });
        self.begin().segs.push(Seg::Line(from, to));
        self.pos = to;
    }

    fn curve_to(
        &mut self,
        cx0: read_fonts::types::Fixed,
        cy0: read_fonts::types::Fixed,
        cx1: read_fonts::types::Fixed,
        cy1: read_fonts::types::Fixed,
        x: read_fonts::types::Fixed,
        y: read_fonts::types::Fixed,
    ) {
        let from = self.pos;
        let (c0, c1, to) = (Pt { x: fixed(cx0), y: fixed(cy0) }, Pt { x: fixed(cx1), y: fixed(cy1) }, Pt { x: fixed(x), y: fixed(y) });
        self.begin().segs.push(Seg::Cubic(from, c0, c1, to));
        self.pos = to;
    }

    fn close(&mut self) {
        if let Some(mut c) = self.open.take() {
            if self.pos != c.start {
                c.segs.push(Seg::Line(self.pos, c.start));
            }
            self.pos = c.start;
            self.contours.push(c);
        }
    }
}

/// Each glyph's exact bounds (curve extremes, not control points) at the default, as
/// fontTools' `BoundsPen` gives them; `None` for a glyph that draws nothing.
pub(super) fn glyph_bounds(table: &[u8]) -> Result<Vec<Option<[f64; 4]>>> {
    use read_fonts::ps::cff::CffFontRef;
    use read_fonts::ps::cs::CommandSink;
    use read_fonts::types::GlyphId;
    let cff = CffFontRef::new(table, 0, None).map_err(|e| invalid(format!("the CFF data cannot be read: {e}")))?;
    let n = cff.num_glyphs() as usize;
    let mut out = Vec::with_capacity(n.min(MAX_GLYPHS));
    for gid in 0..n.min(MAX_GLYPHS) {
        let id = GlyphId::new(gid as u32);
        let fd = cff.subfont_index(id).unwrap_or(0);
        let sub = cff.subfont(fd, &[]).map_err(|e| invalid(format!("the font dictionary of glyph {gid} cannot be read: {e}")))?;
        let mut sink = BoundsSink::default();
        cff.evaluate_charstring(&sub, id, &[], &mut sink).map_err(|e| invalid(format!("the charstring of glyph {gid} cannot be read: {e}")))?;
        sink.close();
        out.push(exact_bounds(&sink.contours));
    }
    Ok(out)
}

/// fontTools' `intRect`: minima floored, maxima ceiled; zeros when nothing is drawn.
pub(super) fn font_bbox(bounds: &[Option<[f64; 4]>]) -> [i64; 4] {
    let mut u: Option<[f64; 4]> = None;
    for &[bx0, by0, bx1, by1] in bounds.iter().flatten() {
        u = Some(match u {
            None => [bx0, by0, bx1, by1],
            Some([x0, y0, x1, y1]) => [x0.min(bx0), y0.min(by0), x1.max(bx1), y1.max(by1)],
        });
    }
    match u {
        None => [0; 4],
        Some([x0, y0, x1, y1]) => [x0.floor() as i64, y0.floor() as i64, x1.ceil() as i64, y1.ceil() as i64],
    }
}

// ---------- the instance steps ----------

/// A full instance of the `CFF2` table at `coords` (normalised, one per `fvar` axis): every
/// blend resolved and the variation store dropped; with `downgrade`, a `CFF ` table instead.
/// A table without a variation store is left as it is unless it is downgraded, as fontTools
/// leaves it. `head`, `hhea` and `vhea` bounds follow when `recalc_bounds`.
pub(super) fn instance_full(font: &mut SfntFont, coords: &[f64], downgrade: bool, recalc_bounds: bool) -> Result<()> {
    let Some(data) = font.get(*b"CFF2") else { return Ok(()) };
    let mut table = Table::read(data, coords.len())?;
    if table.datas.is_empty() && !downgrade {
        if recalc_bounds {
            let bounds = glyph_bounds(data)?;
            recalc_font_bounds(font, &bounds)?;
        }
        return Ok(());
    }
    let scalars: Vec<f64> = table.regions.iter().map(|r| super::tuple_scalar(coords, r.iter().copied())).collect();
    let add: Vec<f64> = table
        .items
        .iter()
        .map(|it| {
            let regions = table.datas.get(it.major).map(Vec::as_slice).unwrap_or_default();
            it.deltas.iter().zip(regions).map(|(d, r)| d * scalars.get(usize::from(*r)).copied().unwrap_or(0.0)).sum()
        })
        .collect();
    let majors = vec![None; table.datas.len()];
    table.resolve(&Resolved { add: &add, deltas: None, majors: &majors })?;
    finish(font, &table, None, downgrade, recalc_bounds)
}

/// Writes the instanced table: `CFF2` with `store` (the item variation store's bytes), or, with
/// `downgrade` and no store left, `CFF `; then the font bounds when `recalc_bounds`.
pub(super) fn finish(font: &mut SfntFont, table: &Table, store: Option<&[u8]>, downgrade: bool, recalc_bounds: bool) -> Result<()> {
    let cff2 = table.write_cff2(store)?;
    let downgrade = downgrade && store.is_none();
    let bounds = if recalc_bounds || downgrade { Some(glyph_bounds(&cff2)?) } else { None };
    if downgrade {
        let n = table.glyph_count();
        let advances = match (font.get(*b"hmtx"), font.get(*b"hhea")) {
            (Some(m), Some(h)) => super::glyf::read_metrics(m, h, n)?.advances,
            _ => return Err(TablesError::MissingTable("hmtx".into())),
        };
        let bbox = match (&bounds, recalc_bounds) {
            (Some(b), true) => font_bbox(b),
            _ => [0; 4],
        };
        let cff = table.write_cff(&advances, bbox)?;
        font.remove(*b"CFF2");
        font.insert(*b"CFF ", cff);
        // A font with CFF outlines stores no glyph names in `post`: version 3.
        if let Some(post) = font.get(*b"post")
            && read_u32(post, 0) == Some(0x0002_0000)
        {
            let mut p = post.get(..32).unwrap_or(post).to_vec();
            if let Some(v) = p.get_mut(0..4) {
                v.copy_from_slice(&0x0003_0000u32.to_be_bytes());
            }
            font.insert(*b"post", p);
        }
    } else {
        font.insert(*b"CFF2", cff2);
    }
    if recalc_bounds && let Some(b) = &bounds {
        recalc_font_bounds(font, b)?;
    }
    Ok(())
}

/// `head`'s box and the `hhea` and `vhea` extremes from each glyph's bounds and the metrics, as
/// fontTools recalculates them for CFF outlines when it saves (the side bearings in `hmtx` and
/// `vmtx` stay as they are).
pub(super) fn recalc_font_bounds(font: &mut SfntFont, bounds: &[Option<[f64; 4]>]) -> Result<()> {
    let [x0, y0, x1, y1] = font_bbox(bounds);
    if let Some(head) = font.get(*b"head")
        && head.len() >= 44
    {
        let mut h = head.to_vec();
        for (at, v) in [(36usize, x0), (38, y0), (40, x1), (42, y1)] {
            if let Some(slot) = h.get_mut(at..at + 2) {
                slot.copy_from_slice(&(v.clamp(-32768, 32767) as i16).to_be_bytes());
            }
        }
        font.insert(*b"head", h);
    }
    let n = bounds.len();
    for (hea_tag, mtx_tag, vertical) in [(*b"hhea", *b"hmtx", false), (*b"vhea", *b"vmtx", true)] {
        let (Some(hea), Some(mtx)) = (font.get(hea_tag), font.get(mtx_tag)) else { continue };
        let m = super::glyf::read_metrics(mtx, hea, n)?;
        let max_advance = m.advances.iter().copied().max().unwrap_or(0);
        let mut min_a = i64::MAX;
        let mut min_b = i64::MAX;
        let mut max_extent = i64::MIN;
        for ((adv, bearing), b) in m.advances.iter().zip(&m.bearings).zip(bounds) {
            let Some([bx0, by0, bx1, by1]) = b else { continue };
            let size = (if vertical { by1.ceil() - by0.floor() } else { bx1.ceil() - bx0.floor() }) as i64;
            let (adv, bearing) = (i64::from(*adv), i64::from(*bearing));
            min_a = min_a.min(bearing);
            min_b = min_b.min(adv - bearing - size);
            max_extent = max_extent.max(bearing + size);
        }
        if max_extent == i64::MIN {
            (min_a, min_b, max_extent) = (0, 0, 0);
        }
        let mut h = hea.to_vec();
        let clamp = |v: i64| v.clamp(-32768, 32767) as i16;
        for (at, bytes) in [
            (10usize, max_advance.to_be_bytes()),
            (12, clamp(min_a).to_be_bytes()),
            (14, clamp(min_b).to_be_bytes()),
            (16, clamp(max_extent).to_be_bytes()),
        ] {
            if let Some(slot) = h.get_mut(at..at + 2) {
                slot.copy_from_slice(&bytes);
            }
        }
        font.insert(hea_tag, h);
    }
    Ok(())
}

/// Adds the deltas of `HVAR` (or `VVAR`) to the advances of a font without `glyf`, rounded half
/// to even and kept at 0 or above, as fontTools' `_instantiateVHVAR` does for CFF2 fonts.
pub(super) fn shift_advances(font: &mut SfntFont, var_tag: crate::sfnt::Tag, deltas: &super::ivs::StoreDeltas) -> Result<()> {
    let (mtx_tag, hea_tag) = if var_tag == *b"VVAR" { (*b"vmtx", *b"vhea") } else { (*b"hmtx", *b"hhea") };
    let (Some(var), Some(mtx), Some(hea)) = (font.get(var_tag), font.get(mtx_tag), font.get(hea_tag)) else { return Ok(()) };
    let n = font.get(*b"maxp").and_then(|m| read_u16(m, 4)).map(usize::from).ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    let mut m = super::glyf::read_metrics(mtx, hea, n)?;
    let mapping = if var_tag == *b"VVAR" {
        let t = read_fonts::tables::vvar::Vvar::read(FontData::new(var)).map_err(|e| invalid(format!("the VVAR table cannot be read: {e}")))?;
        t.advance_height_mapping().transpose().map_err(|e| invalid(format!("the VVAR advance mapping cannot be read: {e}")))?
    } else {
        let t = read_fonts::tables::hvar::Hvar::read(FontData::new(var)).map_err(|e| invalid(format!("the HVAR table cannot be read: {e}")))?;
        t.advance_width_mapping().transpose().map_err(|e| invalid(format!("the HVAR advance mapping cannot be read: {e}")))?
    };
    for (gid, adv) in m.advances.iter_mut().enumerate() {
        let (outer, inner) = match &mapping {
            Some(map) => {
                let i = map
                    .get(gid as u32)
                    .map_err(|e| invalid(format!("the {} advance mapping has no entry for glyph {gid}: {e}", crate::sfnt::tag_str(var_tag))))?;
                (i.outer, i.inner)
            }
            None => (0, u16::try_from(gid).unwrap_or(u16::MAX)),
        };
        let v = f64::from(*adv) + round_half_even(deltas.get(outer, inner));
        *adv = v.clamp(0.0, 65535.0) as u16;
    }
    let (bytes, long) = super::glyf::write_metrics(&m);
    font.insert(mtx_tag, bytes);
    if let Some(h) = font.get(hea_tag)
        && h.len() >= 36
    {
        let mut h = h.to_vec();
        if let Some(slot) = h.get_mut(34..36) {
            slot.copy_from_slice(&long.to_be_bytes());
        }
        font.insert(hea_tag, h);
    }
    Ok(())
}
