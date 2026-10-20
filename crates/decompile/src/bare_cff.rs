//! Bare CFF font programs (format `cff`): a CFF table on its own, outside any sfnt, read into a
//! [`PsFont`] and from there into the model by [`crate::type1::to_font`].
//!
//! The first font of the font set is read: name-keyed fonts with their charset and encoding
//! (predefined, custom and supplements), CID-keyed fonts with glyphs named `cidNNNNN` and the first
//! font dict's Private dict reported. Type 2 charstrings keep their hints in tx's form: hint masks
//! become replacement sets, a first counter mask of three stems on an axis `hstem3`/`vstem3`,
//! other counter masks counter groups, the four flex operators flex.
//!
//! Input is untrusted: every offset and count is checked against the data, and charstrings run
//! under the limits of [`crate::type1`].
//!
//! The first counter mask's reading follows `c/shared/t2cstr.cpp` of AFDKO (Apache License 2.0,
//! Copyright Adobe).

use skrifa::raw::ps::string::STANDARD_STRINGS;

use crate::type1::{
    FontBudget, Hints, MAX_CHARSTRING_WORK, MAX_FONT_OPS, MAX_FONT_WORK, MAX_GLYPH_OPS, MAX_PS_BYTES, MAX_STACK, MAX_SUBR_DEPTH, PrivateDict, PsFont,
    PsGlyph, PsKind, PsOp, StemKind, TOO_MANY_OPS, TopDict, be_i32, list, short_number, to_font,
};
use crate::{DecompileError, Decompiled};
use fontcraft_model::MAX_COORDINATE;

/// Most operands one DICT entry may have.
const MAX_DICT_OPERANDS: usize = 64;
/// Most font dicts of a CID-keyed font.
pub const MAX_FONT_DICTS: usize = 256;

fn bad(msg: impl Into<String>) -> DecompileError {
    DecompileError::Unreadable(msg.into())
}

/// Whether `bytes` start like a bare CFF font: major version 1, a header of at least 4 bytes and
/// an offset size of 1 to 4.
pub fn is_bare_cff(bytes: &[u8]) -> bool {
    matches!(bytes.get(..4), Some(&[1, 0, h, off]) if h >= 4 && (1..=4).contains(&off))
}

/// Reads a bare CFF font into the model.
pub fn decompile_cff(bytes: &[u8]) -> Result<Decompiled, DecompileError> {
    let mut warnings = Vec::new();
    let ps = read_cff(bytes, &mut warnings)?;
    to_font(&ps, warnings)
}

/// An offset read from a DICT, refused when it is negative, not a number or past the end of
/// `data` (a real operand such as 9E99 would otherwise become a huge `usize`).
fn offset_in(data: &[u8], v: f64, what: &str) -> Result<usize, DecompileError> {
    if v >= 0.0 && v <= data.len() as f64 { Ok(v as usize) } else { Err(bad(format!("the {what} offset {v} lies outside the data"))) }
}

/// An INDEX: its items, and the offset just past it.
fn index(data: &[u8], pos: usize) -> Result<(Vec<&[u8]>, usize), DecompileError> {
    let short = || bad("a CFF INDEX runs past the end of the data");
    if pos > data.len() {
        return Err(short());
    }
    let count = data
        .get(pos..pos + 2)
        .map(|b| usize::from(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)])))
        .ok_or_else(short)?;
    if count == 0 {
        return Ok((Vec::new(), pos + 2));
    }
    let off_size = usize::from(*data.get(pos + 2).ok_or_else(short)?);
    if !(1..=4).contains(&off_size) {
        return Err(bad("a CFF INDEX has a bad offset size"));
    }
    let offsets_at = pos + 3;
    let offset = |k: usize| -> Option<usize> {
        let b = data.get(offsets_at + k * off_size..offsets_at + (k + 1) * off_size)?;
        Some(b.iter().fold(0usize, |acc, &x| acc << 8 | usize::from(x)))
    };
    let base = offsets_at + (count + 1) * off_size - 1;
    let mut items = Vec::with_capacity(count);
    let mut prev = offset(0).ok_or_else(short)?;
    // Offsets of up to four bytes can pass the end of `usize` on 32-bit targets.
    let at = |o: usize| base.checked_add(o).filter(|&p| p <= data.len()).ok_or_else(short);
    for k in 1..=count {
        let next = offset(k).ok_or_else(short)?;
        let item = data.get(at(prev)?..at(next)?).ok_or_else(short)?;
        items.push(item);
        prev = next;
    }
    Ok((items, at(prev)?))
}

/// A DICT as (operator, operands): two-byte operators are 1200 + their second byte.
fn dict(data: &[u8]) -> Vec<(u16, Vec<f64>)> {
    let mut out = Vec::new();
    let mut operands = Vec::new();
    let mut i = 0usize;
    while let Some(&b) = data.get(i) {
        i += 1;
        let value = match b {
            0..=21 => {
                let op = if b == 12 {
                    let Some(&b1) = data.get(i) else { break };
                    i += 1;
                    1200 + u16::from(b1)
                } else {
                    u16::from(b)
                };
                out.push((op, std::mem::take(&mut operands)));
                continue;
            }
            28 => match data.get(i..i + 2) {
                Some(&[a, c]) => {
                    i += 2;
                    f64::from(i16::from_be_bytes([a, c]))
                }
                _ => break,
            },
            29 => match be_i32(data, &mut i) {
                Some(v) => f64::from(v),
                None => break,
            },
            30 => {
                let mut text = String::new();
                'real: while let Some(&byte) = data.get(i) {
                    i += 1;
                    for nibble in [byte >> 4, byte & 0xF] {
                        match nibble {
                            0..=9 => text.push(char::from(b'0' + nibble)),
                            0xA => text.push('.'),
                            0xB => text.push('E'),
                            0xC => text.push_str("E-"),
                            0xE => text.push('-'),
                            0xF => break 'real,
                            _ => {}
                        }
                    }
                    if text.len() > 64 {
                        break;
                    }
                }
                text.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0)
            }
            32..=254 => match short_number(b, data, &mut i) {
                Some(v) => v,
                None => break,
            },
            _ => continue,
        };
        if operands.len() < MAX_DICT_OPERANDS {
            operands.push(value);
        }
    }
    out
}

fn deltas(v: &[f64]) -> Vec<f64> {
    let mut acc = 0.0;
    v.iter()
        .take(crate::type1::MAX_DICT_ARRAY)
        .map(|d| {
            acc += d;
            acc
        })
        .collect()
}

struct Strings<'a>(Vec<&'a [u8]>);

impl Strings<'_> {
    fn get(&self, sid: f64) -> Option<String> {
        if !(0.0..=65535.0).contains(&sid) {
            return None;
        }
        let sid = sid as usize;
        match STANDARD_STRINGS.get(sid) {
            Some(s) => Some((*s).to_string()),
            None => self.0.get(sid - STANDARD_STRINGS.len()).map(|b| b.iter().map(|&c| char::from(c)).collect()),
        }
    }
}

fn first(v: &[f64]) -> Option<f64> {
    v.first().copied()
}

struct Private<'a> {
    dict: PrivateDict,
    subrs: Vec<&'a [u8]>,
    default_width: f64,
    nominal_width: f64,
}

fn private<'a>(data: &'a [u8], range: &[f64]) -> Result<Private<'a>, DecompileError> {
    let mut out = Private { dict: PrivateDict::default(), subrs: Vec::new(), default_width: 0.0, nominal_width: 0.0 };
    let &[size, offset] = range else {
        return Ok(out);
    };
    if size < 0.0 || offset < 0.0 {
        return Err(bad("the Private dict has a negative offset"));
    }
    let (start, size) = (offset_in(data, offset, "Private dict")?, size as usize);
    let body = data.get(start..start.saturating_add(size)).ok_or_else(|| bad("the Private dict runs past the end of the data"))?;
    let p = &mut out.dict;
    for (op, v) in dict(body) {
        match op {
            6 => p.blue_values = deltas(&v),
            7 => p.other_blues = deltas(&v),
            8 => p.family_blues = deltas(&v),
            9 => p.family_other_blues = deltas(&v),
            1209 => p.blue_scale = first(&v),
            1210 => p.blue_shift = first(&v),
            1211 => p.blue_fuzz = first(&v),
            10 => p.std_hw = first(&v),
            11 => p.std_vw = first(&v),
            1212 => p.stem_snap_h = deltas(&v),
            1213 => p.stem_snap_v = deltas(&v),
            1214 => p.force_bold = first(&v).map(|b| b != 0.0),
            1217 => p.language_group = first(&v).map(|g| g as i64),
            1218 => p.expansion_factor = first(&v),
            20 => out.default_width = first(&v).unwrap_or(0.0),
            21 => out.nominal_width = first(&v).unwrap_or(0.0),
            19 => {
                if let Some(rel) = first(&v).filter(|r| *r >= 0.0) {
                    let at = offset_in(data, rel, "Subrs")?.checked_add(start).ok_or_else(|| bad("the Subrs offset lies outside the data"))?;
                    out.subrs = index(data, at)?.0;
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Reads the first font of a bare CFF font set into a [`PsFont`].
pub fn read_cff(bytes: &[u8], warnings: &mut Vec<String>) -> Result<PsFont, DecompileError> {
    read_cff_within(bytes, warnings, FontBudget::new())
}

/// [`read_cff`] keeping at most `max_ops` hint and path operations for the whole font (never
/// more than [`MAX_FONT_OPS`]), for callers with a smaller memory budget.
pub fn read_cff_with_ops(bytes: &[u8], warnings: &mut Vec<String>, max_ops: usize) -> Result<PsFont, DecompileError> {
    read_cff_within(bytes, warnings, FontBudget::with_limits(max_ops.min(MAX_FONT_OPS), MAX_FONT_WORK))
}

/// [`read_cff`] under a given font-wide budget.
fn read_cff_within(bytes: &[u8], warnings: &mut Vec<String>, mut budget: FontBudget) -> Result<PsFont, DecompileError> {
    if bytes.len() > MAX_PS_BYTES {
        return Err(bad(format!("the font is {} bytes; the limit is {MAX_PS_BYTES}", bytes.len())));
    }
    if !is_bare_cff(bytes) {
        return Err(bad("not a CFF font program (version 1)"));
    }
    let header_size = usize::from(bytes.get(2).copied().unwrap_or(4));
    let (names, pos) = index(bytes, header_size)?;
    let (tops, pos) = index(bytes, pos)?;
    let (strings, pos) = index(bytes, pos)?;
    let (gsubrs, _) = index(bytes, pos)?;
    let strings = Strings(strings);
    if names.len() > 1 {
        warnings.push(format!("the font set holds {} fonts; only the first was read", names.len()));
    }
    let top_data = tops.first().ok_or_else(|| bad("the CFF has no Top DICT"))?;
    let mut font = PsFont {
        kind: PsKind::Cff,
        font_name: names.first().map(|n| n.iter().map(|&c| char::from(c)).collect()).unwrap_or_default(),
        ..Default::default()
    };
    let mut t = TopDict::default();
    let (mut charset_at, mut encoding_at, mut charstrings_at) = (0.0, 0.0, None);
    let (mut private_range, mut fd_array, mut fd_select) = (Vec::new(), None, None);
    for (op, v) in dict(top_data) {
        let s = || first(&v).and_then(|sid| strings.get(sid));
        match op {
            0 => t.version = s(),
            1 => t.notice = s(),
            1200 => t.copyright = s(),
            2 => t.full_name = s(),
            3 => t.family_name = s(),
            4 => t.weight = s(),
            1201 => t.is_fixed_pitch = first(&v).is_some_and(|b| b != 0.0),
            1202 => t.italic_angle = first(&v).unwrap_or(0.0),
            1203 => t.underline_position = first(&v).unwrap_or(-100.0),
            1204 => t.underline_thickness = first(&v).unwrap_or(50.0),
            1205 => t.paint_type = first(&v).unwrap_or(0.0) as i64,
            1206 if first(&v) != Some(2.0) => return Err(bad("only Type 2 charstrings are read")),
            1207 => {
                if let [a, b, c, d, e, f] = v.as_slice() {
                    let m = [*a, *b, *c, *d, *e, *f];
                    t.font_matrix = (m != [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]).then_some(m);
                }
            }
            13 => t.unique_id = first(&v).map(|u| u as i64),
            5 => {
                if let [a, b, c, d] = v.as_slice() {
                    t.font_bbox = [*a, *b, *c, *d];
                }
            }
            1208 => t.stroke_width = first(&v).unwrap_or(0.0),
            1222 => t.base_font_name = s(),
            1230 => font.kind = PsKind::CffCid,
            15 => charset_at = first(&v).unwrap_or(0.0),
            16 => encoding_at = first(&v).unwrap_or(0.0),
            17 => charstrings_at = first(&v),
            18 => private_range = v,
            1236 => fd_array = first(&v),
            1237 => fd_select = first(&v),
            _ => {}
        }
    }
    font.top = t;
    let charstrings_at = charstrings_at.ok_or_else(|| bad("the CFF has no CharStrings"))?;
    let (charstrings, _) = index(bytes, offset_in(bytes, charstrings_at, "CharStrings")?)?;
    if charstrings.is_empty() {
        return Err(bad("the CFF has no glyphs"));
    }
    let count = charstrings.len();
    let cid = font.kind == PsKind::CffCid;

    // Private dicts: one, or one per font dict of a CID-keyed font.
    let mut privates = Vec::new();
    let mut select = vec![0usize; count];
    if cid {
        let fd_array = fd_array.ok_or_else(|| bad("the CID-keyed CFF has no FDArray"))?;
        let (fds, _) = index(bytes, offset_in(bytes, fd_array, "FDArray")?)?;
        for fd in fds.iter().take(MAX_FONT_DICTS) {
            let range = dict(fd).into_iter().find(|(op, _)| *op == 18).map(|(_, v)| v).unwrap_or_default();
            privates.push(private(bytes, &range)?);
        }
        if let Some(at) = fd_select.filter(|o| *o >= 0.0) {
            read_fd_select(bytes, offset_in(bytes, at, "FDSelect")?, &mut select)?;
        }
    } else {
        privates.push(private(bytes, &private_range)?);
    }
    if privates.is_empty() {
        return Err(bad("the CFF has no Private dict"));
    }
    font.private = privates.first().map(|p| p.dict.clone()).unwrap_or_default();

    let names = glyph_names(bytes, charset_at, count, cid, &strings)?;
    let codes = encoding(bytes, encoding_at, cid, &names, &strings)?;

    let mut failed = Vec::new();
    for (gid, cs) in charstrings.iter().enumerate() {
        let fd = select.get(gid).copied().unwrap_or(0);
        let p = privates.get(fd).or(privates.first()).ok_or_else(|| bad("the CFF has no Private dict"))?;
        let mut run = T2Run::new(&gsubrs, &p.subrs);
        run.global_coloring = p.dict.language_group == Some(1);
        let result = run.run(cs, 0).map(|_| ());
        let finished = run.finish();
        if let Err(e) = result.and(finished) {
            failed.push(format!("{} ({e})", names.get(gid).map_or("?", String::as_str)));
        }
        budget.charge(run.ops.len().saturating_add(run.declared.len()), run.work)?;
        let width = run.width.map_or(p.default_width, |w| p.nominal_width + w);
        font.glyphs.push(PsGlyph {
            name: names.get(gid).cloned().unwrap_or_default(),
            codes: codes.get(gid).cloned().unwrap_or_default(),
            width,
            ops: run.ops,
            stems: run.declared,
        });
    }
    if !failed.is_empty() {
        warnings.push(format!("{} glyphs could not be read in full: {}", failed.len(), list(&failed)));
    }
    Ok(font)
}

fn u16_at(data: &[u8], pos: usize) -> Option<usize> {
    let b = data.get(pos..pos + 2)?;
    Some(usize::from(u16::from_be_bytes([*b.first()?, *b.get(1)?])))
}

fn read_fd_select(data: &[u8], at: usize, select: &mut [usize]) -> Result<(), DecompileError> {
    let short = || bad("the FDSelect runs past the end of the data");
    match data.get(at) {
        Some(0) => {
            for (gid, slot) in select.iter_mut().enumerate() {
                *slot = usize::from(*data.get(at + 1 + gid).ok_or_else(short)?);
            }
        }
        Some(3) => {
            let ranges = u16_at(data, at + 1).ok_or_else(short)?;
            for r in 0..ranges {
                let rec = at + 3 + r * 3;
                let start = u16_at(data, rec).ok_or_else(short)?;
                let fd = usize::from(*data.get(rec + 2).ok_or_else(short)?);
                let end = u16_at(data, rec + 3).ok_or_else(short)?;
                for slot in select.iter_mut().take(end).skip(start) {
                    *slot = fd;
                }
            }
        }
        _ => return Err(bad("unknown FDSelect format")),
    }
    Ok(())
}

/// Glyph names from the charset (CIDs as `cidNNNNN` in a CID-keyed font).
fn glyph_names(data: &[u8], at: f64, count: usize, cid: bool, strings: &Strings) -> Result<Vec<String>, DecompileError> {
    let short = || bad("the charset runs past the end of the data");
    let mut ids = vec![0usize];
    if at <= 2.0 {
        // Predefined charsets: ISOAdobe maps glyph ids to the same string ids; the expert ones are
        // not carried, so their glyphs get numbered names.
        ids = (0..count).map(|g| if at == 0.0 { g } else { usize::MAX }).collect();
    } else {
        let at = offset_in(data, at, "charset")?;
        let format = *data.get(at).ok_or_else(short)?;
        let mut pos = at + 1;
        while ids.len() < count {
            match format {
                0 => {
                    ids.push(u16_at(data, pos).ok_or_else(short)?);
                    pos += 2;
                }
                1 | 2 => {
                    let first = u16_at(data, pos).ok_or_else(short)?;
                    let left =
                        if format == 1 { usize::from(*data.get(pos + 2).ok_or_else(short)?) } else { u16_at(data, pos + 2).ok_or_else(short)? };
                    pos += if format == 1 { 3 } else { 4 };
                    for k in 0..=left {
                        if ids.len() >= count {
                            break;
                        }
                        ids.push(first + k);
                    }
                }
                _ => return Err(bad("unknown charset format")),
            }
        }
    }
    Ok(ids
        .into_iter()
        .enumerate()
        .map(|(gid, id)| {
            if gid == 0 {
                ".notdef".to_string()
            } else if cid {
                format!("cid{id:05}")
            } else {
                strings.get(id as f64).unwrap_or_else(|| format!("glyph{gid:05}"))
            }
        })
        .collect())
}

/// The codes of each glyph under a custom encoding; empty for the predefined ones (as tx reports).
fn encoding(data: &[u8], at: f64, cid: bool, names: &[String], strings: &Strings) -> Result<Vec<Vec<u8>>, DecompileError> {
    let mut codes = vec![Vec::new(); names.len()];
    if cid || at <= 1.0 {
        return Ok(codes);
    }
    let short = || bad("the encoding runs past the end of the data");
    let at = offset_in(data, at, "encoding")?;
    let format = *data.get(at).ok_or_else(short)?;
    let mut pos = at + 1;
    let mut gid = 1usize;
    match format & 0x7F {
        0 => {
            let n = usize::from(*data.get(pos).ok_or_else(short)?);
            for k in 0..n {
                let code = *data.get(pos + 1 + k).ok_or_else(short)?;
                if let Some(c) = codes.get_mut(gid) {
                    c.push(code);
                }
                gid += 1;
            }
            pos += 1 + n;
        }
        1 => {
            let n = usize::from(*data.get(pos).ok_or_else(short)?);
            for r in 0..n {
                let first = *data.get(pos + 1 + r * 2).ok_or_else(short)?;
                let left = *data.get(pos + 2 + r * 2).ok_or_else(short)?;
                for k in 0..=u16::from(left) {
                    if let (Some(c), Ok(code)) = (codes.get_mut(gid), u8::try_from(u16::from(first) + k)) {
                        c.push(code);
                    }
                    gid += 1;
                }
            }
            pos += 1 + n * 2;
        }
        _ => return Err(bad("unknown encoding format")),
    }
    if format & 0x80 != 0 {
        let n = usize::from(*data.get(pos).ok_or_else(short)?);
        for k in 0..n {
            let rec = pos + 1 + k * 3;
            let code = *data.get(rec).ok_or_else(short)?;
            let sid = u16_at(data, rec + 1).ok_or_else(short)?;
            let Some(name) = strings.get(sid as f64) else { continue };
            if let Some(g) = names.iter().position(|n| *n == name)
                && let Some(c) = codes.get_mut(g)
            {
                c.push(code);
            }
        }
    }
    for c in &mut codes {
        c.sort_unstable();
    }
    Ok(codes)
}

// ---------------------------------------------------------------------------------------------
// Type 2 charstrings

fn bias(count: usize) -> i64 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

enum Flow {
    Next,
    Return,
    End,
}

struct T2Run<'a> {
    gsubrs: &'a [&'a [u8]],
    subrs: &'a [&'a [u8]],
    stack: Vec<f64>,
    x: f64,
    y: f64,
    width: Option<f64>,
    width_seen: bool,
    /// Every stem declared, in order: hint masks select from these.
    declared: Vec<PsOp>,
    hints: Hints,
    /// The first counter mask's stems (indices into `declared`) until the next mask or moveto
    /// decides, as tx does, whether they are `hstem3`/`vstem3` or a counter group.
    first_counter: Option<Vec<usize>>,
    counter_settled: bool,
    /// The Private dict's `LanguageGroup` is 1: counter masks are always counter groups.
    global_coloring: bool,
    ops: Vec<PsOp>,
    work: usize,
}

impl<'a> T2Run<'a> {
    fn new(gsubrs: &'a [&'a [u8]], subrs: &'a [&'a [u8]]) -> Self {
        Self {
            gsubrs,
            subrs,
            stack: Vec::new(),
            x: 0.0,
            y: 0.0,
            width: None,
            width_seen: false,
            declared: Vec::new(),
            hints: Hints::default(),
            first_counter: None,
            counter_settled: false,
            global_coloring: false,
            ops: Vec::new(),
            work: 0,
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        // A first counter mask with no later mask or moveto is decided at the end.
        self.settle_counter(false)?;
        if !self.hints.emitted {
            self.hints.flush(&mut self.ops)?;
        }
        Ok(())
    }

    fn push(&mut self, v: f64) -> Result<(), String> {
        if self.stack.len() >= MAX_STACK {
            return Err("operand stack overflow".into());
        }
        self.stack.push(v);
        Ok(())
    }

    fn pop(&mut self) -> Result<f64, String> {
        self.stack.pop().ok_or_else(|| "operand stack underflow".to_string())
    }

    /// At the first stack-clearing operator: an extra operand in front is the width.
    fn width_from(&mut self, extra: bool) {
        if !self.width_seen {
            self.width_seen = true;
            if extra && !self.stack.is_empty() {
                self.width = Some(self.stack.remove(0));
            }
        }
    }

    fn emit(&mut self, op: PsOp) -> Result<(), String> {
        if self.ops.len() >= MAX_GLYPH_OPS {
            return Err(TOO_MANY_OPS.into());
        }
        if !op.in_range() {
            return Err(format!("a point lies outside the coordinate range of ±{MAX_COORDINATE}"));
        }
        if matches!(op, PsOp::Move(..)) {
            self.settle_counter(false)?;
        }
        self.hints.flush(&mut self.ops)?;
        self.ops.push(op);
        Ok(())
    }

    /// Decides the first counter mask: three horizontal and/or three vertical stems, with no
    /// second counter mask after it and `LanguageGroup` not 1, mark those stems `hstem3`/`vstem3`;
    /// anything else is a counter group.
    fn settle_counter(&mut self, another: bool) -> Result<(), String> {
        let Some(first) = self.first_counter.take() else { return Ok(()) };
        self.counter_settled = true;
        let horizontal = first.iter().filter(|&&k| matches!(self.declared.get(k), Some(PsOp::HStem { .. }))).count();
        let stem3 = matches!((horizontal, first.len().saturating_sub(horizontal)), (0, 3) | (3, 0) | (3, 3));
        if another || self.global_coloring || !stem3 {
            let active: Vec<PsOp> = first.iter().filter_map(|&k| self.declared.get(k).cloned()).collect();
            return self.counter_group(active);
        }
        for k in first {
            let Some(op) = self.declared.get_mut(k) else { continue };
            let plain = op.clone();
            match op {
                PsOp::HStem { kind, .. } | PsOp::VStem { kind, .. } => *kind = StemKind::Stem3,
                _ => {}
            }
            let marked = op.clone();
            for p in self.hints.pending.iter_mut().filter(|p| **p == plain) {
                *p = marked.clone();
            }
        }
        Ok(())
    }

    fn counter_group(&mut self, active: Vec<PsOp>) -> Result<(), String> {
        if self.ops.len().saturating_add(active.len()).saturating_add(1) > MAX_GLYPH_OPS {
            return Err(TOO_MANY_OPS.into());
        }
        self.ops.push(PsOp::NewGroup);
        self.ops.extend(active.into_iter().map(|op| match op {
            PsOp::HStem { y, dy, .. } => PsOp::HStem { y, dy, kind: StemKind::Counter },
            PsOp::VStem { x, dx, .. } => PsOp::VStem { x, dx, kind: StemKind::Counter },
            other => other,
        }));
        Ok(())
    }

    fn line(&mut self, dx: f64, dy: f64) -> Result<(), String> {
        self.x += dx;
        self.y += dy;
        self.emit(PsOp::Line(self.x, self.y))
    }

    fn curve(&mut self, d: [f64; 6]) -> Result<(), String> {
        let (x1, y1) = (self.x + d[0], self.y + d[1]);
        let (x2, y2) = (x1 + d[2], y1 + d[3]);
        (self.x, self.y) = (x2 + d[4], y2 + d[5]);
        self.emit(PsOp::Curve([x1, y1, x2, y2, self.x, self.y]))
    }

    fn stems(&mut self, horizontal: bool) {
        let args = std::mem::take(&mut self.stack);
        let mut edge = 0.0;
        for &[a, d] in args.as_chunks::<2>().0 {
            let pos = edge + a;
            edge = pos + d;
            let op =
                if horizontal { PsOp::HStem { y: pos, dy: d, kind: StemKind::Stem } } else { PsOp::VStem { x: pos, dx: d, kind: StemKind::Stem } };
            if self.declared.len() < MAX_GLYPH_OPS && op.in_range() {
                self.declared.push(op.clone());
                self.hints.stem(op);
            }
        }
    }

    fn mask(&mut self, counter: bool, bytes: &[u8]) -> Result<(), String> {
        self.settle_counter(counter)?;
        let on = |k: usize| bytes.get(k / 8).is_some_and(|b| b & (0x80 >> (k % 8)) != 0);
        if counter && !self.counter_settled {
            self.first_counter = Some((0..self.declared.len()).filter(|&k| on(k)).collect());
            return Ok(());
        }
        let active: Vec<PsOp> = self.declared.iter().enumerate().filter(|(k, _)| on(*k)).map(|(_, op)| op.clone()).collect();
        if counter {
            self.counter_group(active)?;
        } else {
            if self.hints.emitted {
                self.hints.replacing = true;
            }
            self.hints.pending = active;
        }
        Ok(())
    }

    fn run(&mut self, cs: &[u8], depth: usize) -> Result<Flow, String> {
        if depth > MAX_SUBR_DEPTH {
            return Err("subroutines nest too deeply".into());
        }
        let mut i = 0usize;
        while let Some(&b) = cs.get(i) {
            i += 1;
            self.work += 1;
            if self.work > MAX_CHARSTRING_WORK {
                return Err("the charstring does too much work".into());
            }
            let flow = match b {
                28 => {
                    let v = cs.get(i..i + 2).and_then(|s| <[u8; 2]>::try_from(s).ok()).ok_or("a number is cut short")?;
                    i += 2;
                    self.push(f64::from(i16::from_be_bytes(v)))?;
                    Flow::Next
                }
                32..=254 => {
                    let v = short_number(b, cs, &mut i).ok_or("a number is cut short")?;
                    self.push(v)?;
                    Flow::Next
                }
                255 => {
                    let v = be_i32(cs, &mut i).ok_or("a number is cut short")?;
                    self.push(f64::from(v) / 65536.0)?;
                    Flow::Next
                }
                19 | 20 => {
                    self.width_from(self.stack.len() % 2 == 1);
                    self.stems(false);
                    let n = self.declared.len().div_ceil(8);
                    let bytes = cs.get(i..i + n).ok_or("a hint mask is cut short")?;
                    i += n;
                    // Each mask byte selects up to eight stems: charge it as work, so a glyph
                    // cannot repeat a long mask without limit.
                    self.work = self.work.saturating_add(n);
                    if self.work > MAX_CHARSTRING_WORK {
                        return Err("the charstring does too much work".into());
                    }
                    self.mask(b == 20, bytes)?;
                    Flow::Next
                }
                10 | 29 => {
                    let subrs = if b == 10 { self.subrs } else { self.gsubrs };
                    let index = (self.pop()? as i64).saturating_add(bias(subrs.len()));
                    let sub = usize::try_from(index).ok().and_then(|k| subrs.get(k)).ok_or("a subroutine is missing")?;
                    match self.run(sub, depth + 1)? {
                        Flow::End => Flow::End,
                        _ => Flow::Next,
                    }
                }
                12 => {
                    let b1 = *cs.get(i).ok_or("an operator is cut short")?;
                    i += 1;
                    self.escape(b1)?;
                    Flow::Next
                }
                _ => self.op(b)?,
            };
            match flow {
                Flow::Next => {}
                Flow::Return => return Ok(Flow::Return),
                Flow::End => return Ok(Flow::End),
            }
        }
        Ok(Flow::Return)
    }

    fn op(&mut self, b: u8) -> Result<Flow, String> {
        let args = |s: &Self| s.stack.clone();
        match b {
            1 | 3 | 18 | 23 => {
                self.width_from(self.stack.len() % 2 == 1);
                self.stems(b == 1 || b == 18);
            }
            21 => {
                self.width_from(self.stack.len() > 2);
                let [dx, dy] = last2(&self.stack)?;
                self.stack.clear();
                self.x += dx;
                self.y += dy;
                self.emit(PsOp::Move(self.x, self.y))?;
            }
            22 | 4 => {
                self.width_from(self.stack.len() > 1);
                let d = *self.stack.last().ok_or("too few operands")?;
                self.stack.clear();
                if b == 22 {
                    self.x += d;
                } else {
                    self.y += d;
                }
                self.emit(PsOp::Move(self.x, self.y))?;
            }
            5 => {
                for &[dx, dy] in args(self).as_chunks::<2>().0 {
                    self.line(dx, dy)?;
                }
                self.stack.clear();
            }
            6 | 7 => {
                let mut horizontal = b == 6;
                for d in args(self) {
                    if horizontal {
                        self.line(d, 0.0)?
                    } else {
                        self.line(0.0, d)?
                    }
                    horizontal = !horizontal;
                }
                self.stack.clear();
            }
            8 | 24 | 25 => {
                let a = args(self);
                self.stack.clear();
                let (lines_first, curves) = match b {
                    8 => (false, a.as_slice()),
                    24 => (false, a.get(..a.len().saturating_sub(2)).unwrap_or_default()),
                    _ => (true, a.get(a.len().saturating_sub(6)..).unwrap_or_default()),
                };
                if lines_first {
                    for &[dx, dy] in a.get(..a.len().saturating_sub(6)).unwrap_or_default().as_chunks::<2>().0 {
                        self.line(dx, dy)?;
                    }
                }
                for &c in curves.as_chunks::<6>().0 {
                    self.curve(c)?;
                }
                if b == 24
                    && let Some(&[dx, dy]) = a.get(a.len().saturating_sub(2)..)
                {
                    self.line(dx, dy)?;
                }
            }
            26 | 27 => {
                let mut a = args(self);
                self.stack.clear();
                let mut extra = if a.len() % 2 == 1 && !a.is_empty() { a.remove(0) } else { 0.0 };
                for &[p, q, r, s] in a.as_chunks::<4>().0 {
                    if b == 26 {
                        self.curve([extra, p, q, r, 0.0, s])?;
                    } else {
                        self.curve([p, extra, q, r, s, 0.0])?;
                    }
                    extra = 0.0;
                }
            }
            30 | 31 => {
                let a = args(self);
                self.stack.clear();
                let mut vertical = b == 30;
                let groups = a.len() / 4;
                for g in 0..groups {
                    let Some(&[p, q, r, s]) = a.get(g * 4..g * 4 + 4) else { break };
                    let last = if g + 1 == groups { a.get(groups * 4).copied().unwrap_or(0.0) } else { 0.0 };
                    if vertical {
                        self.curve([0.0, p, q, r, s, last])?;
                    } else {
                        self.curve([p, 0.0, q, r, last, s])?;
                    }
                    vertical = !vertical;
                }
            }
            11 => return Ok(Flow::Return),
            14 => {
                self.width_from(self.stack.len() == 1 || self.stack.len() == 5);
                if let [adx, ady, base, accent] = self.stack.as_slice() {
                    let code = |v: f64| v.clamp(0.0, 255.0) as u8;
                    let seac = PsOp::Seac { adx: *adx, ady: *ady, base: code(*base), accent: code(*accent) };
                    self.emit(seac)?;
                }
                self.stack.clear();
                return Ok(Flow::End);
            }
            other => return Err(format!("unknown operator {other}")),
        }
        Ok(Flow::Next)
    }

    fn escape(&mut self, b: u8) -> Result<(), String> {
        match b {
            0 => self.stack.clear(),
            9 => {
                let v = self.pop()?;
                self.push(v.abs())?;
            }
            10 | 11 | 12 | 24 => {
                let q = self.pop()?;
                let p = self.pop()?;
                let v = match b {
                    10 => p + q,
                    11 => p - q,
                    12 if q == 0.0 => 0.0,
                    12 => p / q,
                    _ => p * q,
                };
                self.push(if v.is_finite() { v } else { 0.0 })?;
            }
            14 => {
                let v = self.pop()?;
                self.push(-v)?;
            }
            18 => {
                self.pop()?;
            }
            26 => {
                let v = self.pop()?;
                self.push(if v >= 0.0 { v.sqrt() } else { 0.0 })?;
            }
            27 => {
                let v = *self.stack.last().ok_or("operand stack underflow")?;
                self.push(v)?;
            }
            28 => {
                let q = self.pop()?;
                let p = self.pop()?;
                self.push(q)?;
                self.push(p)?;
            }
            34..=37 => {
                let a = self.stack.clone();
                self.stack.clear();
                let (x0, y0) = (self.x, self.y);
                let mut pts = [0.0; 12];
                let mut depth = 50.0;
                let rel: Vec<(f64, f64)> = match (b, a.as_slice()) {
                    (35, &[a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, fd]) => {
                        depth = fd;
                        vec![(a0, a1), (a2, a3), (a4, a5), (a6, a7), (a8, a9), (a10, a11)]
                    }
                    (34, &[dx1, dx2, dy2, dx3, dx4, dx5, dx6]) => vec![(dx1, 0.0), (dx2, dy2), (dx3, 0.0), (dx4, 0.0), (dx5, -dy2), (dx6, 0.0)],
                    (36, &[dx1, dy1, dx2, dy2, dx3, dx4, dx5, dy5, dx6]) => {
                        let back = -(dy1 + dy2 + dy5);
                        vec![(dx1, dy1), (dx2, dy2), (dx3, 0.0), (dx4, 0.0), (dx5, dy5), (dx6, back)]
                    }
                    (37, &[d1x, d1y, d2x, d2y, d3x, d3y, d4x, d4y, d5x, d5y, d6]) => {
                        let (sx, sy) = (d1x + d2x + d3x + d4x + d5x, d1y + d2y + d3y + d4y + d5y);
                        let last = if sx.abs() > sy.abs() { (d6, -sy) } else { (-sx, d6) };
                        vec![(d1x, d1y), (d2x, d2y), (d3x, d3y), (d4x, d4y), (d5x, d5y), last]
                    }
                    _ => return Err("bad flex operands".into()),
                };
                let (mut x, mut y) = (x0, y0);
                for ([px, py], (dx, dy)) in pts.as_chunks_mut::<2>().0.iter_mut().zip(rel) {
                    x += dx;
                    y += dy;
                    *px = x;
                    *py = y;
                }
                (self.x, self.y) = (x, y);
                self.emit(PsOp::Flex(pts, depth))?;
            }
            other => return Err(format!("unsupported operator 12 {other}")),
        }
        Ok(())
    }
}

fn last2(stack: &[f64]) -> Result<[f64; 2], String> {
    match stack.get(stack.len().saturating_sub(2)..) {
        Some(&[a, b]) => Ok([a, b]),
        _ => Err("too few operands".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type1::{MAX_FONT_OPS, MAX_FONT_WORK};

    fn index_of(items: &[Vec<u8>]) -> Vec<u8> {
        let mut out = (items.len() as u16).to_be_bytes().to_vec();
        if items.is_empty() {
            return out;
        }
        out.push(4);
        let mut off = 1u32;
        out.extend(off.to_be_bytes());
        for it in items {
            off += it.len() as u32;
            out.extend(off.to_be_bytes());
        }
        for it in items {
            out.extend_from_slice(it);
        }
        out
    }

    fn int5(v: usize) -> Vec<u8> {
        [vec![29], (v as i32).to_be_bytes().to_vec()].concat()
    }

    /// A bare CFF font with these charstrings and Private dict, `top_extra` (DICT entries) at
    /// the end of its Top DICT.
    fn font(charstrings: &[Vec<u8>], private: &[u8], top_extra: &[u8]) -> Vec<u8> {
        let header = vec![1, 0, 4, 4];
        let names = index_of(&[b"T".to_vec()]);
        let charstrings = index_of(charstrings);
        let top_len = index_of(&[vec![0; 17 + top_extra.len()]]).len();
        let cs_at = header.len() + names.len() + top_len + 4;
        let private_at = cs_at + charstrings.len();
        let top = [int5(cs_at), vec![17], int5(private.len()), int5(private_at), vec![18], top_extra.to_vec()].concat();
        [header, names, index_of(&[top]), index_of(&[]), index_of(&[]), charstrings, private.to_vec()].concat()
    }

    /// `0 0 rmoveto`, `count` times `1 1 rlineto`, `endchar`.
    fn lines(count: usize) -> Vec<u8> {
        [vec![139, 139, 21], [140, 140, 5].repeat(count), vec![14]].concat()
    }

    #[test]
    fn the_test_font_reads() {
        let ps = read_cff(&font(&[lines(1), lines(2)], &[], &[]), &mut Vec::new()).unwrap();
        assert_eq!(ps.glyphs.iter().map(|g| g.ops.len()).collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn stems_no_hint_mask_selects_are_imported_and_written_back() {
        // `0 40 660 40 hstemhm`, `100 80 200 80 hintmask 0xC0` (the mask selects the two
        // horizontal stems only), then a rectangle from x 100 to 180, 0 to 700 high.
        let cs = vec![139, 179, 249, 40, 179, 18, 239, 219, 247, 92, 219, 19, 0xC0, 239, 139, 21, 219, 139, 139, 249, 80, 59, 139, 5, 14];
        let bytes = font(&[lines(1), cs], &[], &[]);
        let want = vec![
            PsOp::HStem { y: 0.0, dy: 40.0, kind: StemKind::Stem },
            PsOp::HStem { y: 700.0, dy: 40.0, kind: StemKind::Stem },
            PsOp::VStem { x: 100.0, dx: 80.0, kind: StemKind::Stem },
            PsOp::VStem { x: 380.0, dx: 80.0, kind: StemKind::Stem },
        ];
        let ps = read_cff(&bytes, &mut Vec::new()).unwrap();
        assert_eq!(ps.glyphs[1].stems, want);
        let name = ps.glyphs[1].name.clone();

        let d = decompile_cff(&bytes).unwrap();
        assert_eq!(d.font.glyph(&name).unwrap().layers[0].info.hints.len(), 4, "every declared stem is a layer hint");

        let target = d.font.default_target().unwrap();
        let out = fontcraft_compile::compile(&d.font, &target, fontcraft_compile::Flavour::Cff, &fontcraft_compile::Options::default()).unwrap();
        let sfnt = skrifa::raw::FontRef::new(&out.bytes).unwrap();
        let table = sfnt.table_data(skrifa::raw::types::Tag::new(b"CFF ")).unwrap();
        let back = read_cff(table.as_bytes(), &mut Vec::new()).unwrap();
        let g = back.glyphs.iter().find(|g| g.name == name).unwrap();
        assert_eq!(g.stems, want, "the export writes the same stems back");
    }

    #[test]
    fn a_font_past_the_font_wide_budget_is_refused() {
        let bytes = font(&std::iter::repeat_n(lines(100), 50).collect::<Vec<_>>(), &[], &[]);
        assert_eq!(read_cff_within(&bytes, &mut Vec::new(), FontBudget::new()).unwrap().glyphs.len(), 50);
        assert!(read_cff_within(&bytes, &mut Vec::new(), FontBudget::with_limits(1_000, MAX_FONT_WORK)).is_err());
        assert!(read_cff_within(&bytes, &mut Vec::new(), FontBudget::with_limits(MAX_FONT_OPS, 1_000)).is_err());
    }

    #[test]
    fn repeated_counter_masks_stop_at_the_glyph_limit() {
        // 24 stems, then 60,000 counter masks selecting all of them (25 operations each).
        let mut cs = [vec![139; 48], vec![1]].concat();
        cs.extend([20, 0xFF, 0xFF, 0xFF].repeat(60_000));
        cs.push(14);
        let mut warnings = Vec::new();
        let ps = read_cff(&font(&[lines(1), cs], &[], &[]), &mut warnings).unwrap();
        assert!(ps.glyphs[1].ops.len() <= MAX_GLYPH_OPS, "{}", ps.glyphs[1].ops.len());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn a_first_counter_mask_without_a_later_moveto_is_still_decided() {
        // `0 20 100 20 100 20 hstem`, `cntrmask 0xE0` (all three stems), `endchar`: no path.
        let cs = vec![139, 159, 239, 159, 239, 159, 1, 20, 0xE0, 14];
        let ps = read_cff(&font(&[lines(1), cs], &[], &[]), &mut Vec::new()).unwrap();
        let stems = &ps.glyphs[1].stems;
        assert_eq!(stems.len(), 3);
        assert!(stems.iter().all(|s| matches!(s, PsOp::HStem { kind: StemKind::Stem3, .. })), "{stems:?}");
    }

    #[test]
    fn offsets_past_the_data_are_refused() {
        // The real 9E99.
        let huge = vec![30, 0x9B, 0x99, 0xFF];
        let with = |op: &[u8]| [huge.clone(), op.to_vec()].concat();
        let cid = vec![139, 139, 139, 12, 30];
        let cases = [
            (Vec::new(), with(&[17])),
            (Vec::new(), with(&[15])),
            (Vec::new(), with(&[16])),
            (Vec::new(), [cid, with(&[12, 36])].concat()),
            // Subrs far past the Private dict.
            (with(&[19]), Vec::new()),
        ];
        for (k, (private, extra)) in cases.iter().enumerate() {
            assert!(read_cff(&font(&[lines(1)], private, extra), &mut Vec::new()).is_err(), "case {k}");
        }
    }
}
