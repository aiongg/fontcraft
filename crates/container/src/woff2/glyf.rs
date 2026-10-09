//! The WOFF2 `glyf`/`loca` transform (version 0), both ways.
//!
//! Ported from fontTools (MIT): `WOFF2GlyfTable`, `WOFF2LocaTable` and
//! `WOFF2Writer._normaliseGlyfAndLoca` in `ttLib/woff2.py`, and from `ttLib/tables/_g_l_y_f.py`
//! and `_l_o_c_a.py` the glyph reading and writing those rely on: `Glyph.expand` and
//! `decompileCoordinates`, `compileCoordinates` with `compileDeltasGreedy`, `GlyphComponent`'s
//! `decompile` and `compile`, the padding rules of `table__g_l_y_f.compile`, and the choice of
//! `loca` offset format.
//!
//! Writing: the glyphs are first laid out again with every glyph padded to four bytes and `loca`
//! in the shortest format that holds the offsets (`normalise`). Each glyph is then split into the
//! seven streams of the format. A glyph the transform cannot carry losslessly (a cubic point,
//! damaged data) makes [`transform`] return `None`, and the caller stores `glyf` and `loca`
//! untransformed, as fontTools does for cubic outlines.
//!
//! Reading: the glyphs are rebuilt and written as fontTools writes them (flags and coordinates
//! packed greedily, components with recomputed argument and scale flags); glyphs are not padded,
//! except that odd-sized ones get one byte when that lets `loca` use short offsets. The offset
//! format is the one the transformed table names.

use super::{push_255u16, read_255u16};
use crate::{Error, MAX_SFNT};

const FORMAT: &str = "WOFF2";

fn bad(what: impl Into<String>) -> Error {
    Error::invalid(FORMAT, what)
}

const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME: u8 = 0x10;
const Y_SAME: u8 = 0x20;
const OVERLAP_SIMPLE: u8 = 0x40;
const CUBIC: u8 = 0x80;

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;
/// The component flags a glyph keeps (round to grid, non-overlapping, use my metrics, overlap
/// compound, scaled and unscaled offset); the others follow from the component's data.
const KEPT_COMPONENT_FLAGS: u16 = 0x0004 | 0x0010 | 0x0200 | 0x0400 | 0x0800 | 0x1000;

/// Bit 0 of the transformed table's `optionFlags`: the overlap-simple bitmap follows the streams.
const HAS_OVERLAP_BITMAP: u16 = 0x0001;
/// The transformed table's header: four 16-bit fields and seven 32-bit stream sizes.
const HEADER: usize = 36;
/// The largest offset short `loca` offsets can hold, plus one.
const SHORT_LOCA_LIMIT: usize = 0x2_0000;

/// A reader over one stream; running past its end is an error naming the stream.
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
    what: &'static str,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8], what: &'static str) -> Self {
        Cursor { data, at: 0, what }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).ok_or_else(|| bad(format!("{} is cut short", self.what)))?;
        let part = self.data.get(self.at..end).ok_or_else(|| bad(format!("{} is cut short", self.what)))?;
        self.at = end;
        Ok(part)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        self.take(1)?.first().copied().ok_or_else(|| bad(format!("{} is cut short", self.what)))
    }

    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap_or_default()))
    }

    fn i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap_or_default()))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap_or_default()))
    }

    fn u255(&mut self) -> Result<u16, Error> {
        read_255u16(self.data, &mut self.at).map_err(|_| bad(format!("{} is cut short", self.what)))
    }
}

/// The two arguments of a component: a point on each glyph to match, or an offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Args {
    Points(u16, u16),
    Offset(i16, i16),
}

/// One component of a composite glyph, with its flags reduced to the kept ones.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Component {
    flags: u16,
    glyph: u16,
    args: Args,
    /// The 2×2 matrix in F2Dot14 (xx, xy, yx, yy), when the component has a scale.
    transform: Option<[i16; 4]>,
}

impl Component {
    /// Reads one component record; returns it, whether more follow, and whether it says
    /// instructions follow the components.
    fn read(c: &mut Cursor<'_>) -> Result<(Self, bool, bool), Error> {
        let flags = c.u16()?;
        let glyph = c.u16()?;
        let byte = |c: &mut Cursor<'_>| c.u8();
        let args = match (flags & ARG_1_AND_2_ARE_WORDS != 0, flags & ARGS_ARE_XY_VALUES != 0) {
            (true, true) => Args::Offset(c.i16()?, c.i16()?),
            (true, false) => Args::Points(c.u16()?, c.u16()?),
            (false, true) => Args::Offset(i16::from(byte(c)? as i8), i16::from(byte(c)? as i8)),
            (false, false) => Args::Points(u16::from(byte(c)?), u16::from(byte(c)?)),
        };
        let transform = if flags & WE_HAVE_A_SCALE != 0 {
            let s = c.i16()?;
            Some([s, 0, 0, s])
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            let (x, y) = (c.i16()?, c.i16()?);
            Some([x, 0, 0, y])
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            Some([c.i16()?, c.i16()?, c.i16()?, c.i16()?])
        } else {
            None
        };
        let component = Component { flags: flags & KEPT_COMPONENT_FLAGS, glyph, args, transform };
        Ok((component, flags & MORE_COMPONENTS != 0, flags & WE_HAVE_INSTRUCTIONS != 0))
    }

    /// Writes the record with the smallest argument size and scale form that hold its values.
    fn write(&self, more: bool, instructions: bool, out: &mut Vec<u8>) {
        let mut flags = self.flags;
        if more {
            flags |= MORE_COMPONENTS;
        }
        if instructions {
            flags |= WE_HAVE_INSTRUCTIONS;
        }
        let mut tail = Vec::with_capacity(12);
        match self.args {
            Args::Points(a, b) => match (u8::try_from(a), u8::try_from(b)) {
                (Ok(a), Ok(b)) => tail.extend_from_slice(&[a, b]),
                _ => {
                    flags |= ARG_1_AND_2_ARE_WORDS;
                    tail.extend_from_slice(&a.to_be_bytes());
                    tail.extend_from_slice(&b.to_be_bytes());
                }
            },
            Args::Offset(x, y) => {
                flags |= ARGS_ARE_XY_VALUES;
                match (i8::try_from(x), i8::try_from(y)) {
                    (Ok(x), Ok(y)) => tail.extend_from_slice(&[x as u8, y as u8]),
                    _ => {
                        flags |= ARG_1_AND_2_ARE_WORDS;
                        tail.extend_from_slice(&x.to_be_bytes());
                        tail.extend_from_slice(&y.to_be_bytes());
                    }
                }
            }
        }
        if let Some([xx, xy, yx, yy]) = self.transform {
            let values: &[i16] = if xy != 0 || yx != 0 {
                flags |= WE_HAVE_A_TWO_BY_TWO;
                &[xx, xy, yx, yy]
            } else if xx != yy {
                flags |= WE_HAVE_AN_X_AND_Y_SCALE;
                &[xx, yy]
            } else {
                flags |= WE_HAVE_A_SCALE;
                &[xx]
            };
            for v in values {
                tail.extend_from_slice(&v.to_be_bytes());
            }
        }
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(&self.glyph.to_be_bytes());
        out.extend_from_slice(&tail);
    }
}

/// A simple glyph: contours of points, each point given as its move from the previous one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Simple<'a> {
    bbox: [i16; 4],
    end_pts: Vec<u16>,
    instructions: &'a [u8],
    on_curve: Vec<bool>,
    /// The overlap-simple flag, which only the first point's flags carry.
    overlap: bool,
    deltas: Vec<(i32, i32)>,
}

/// One glyph of a `glyf` table.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Glyph<'a> {
    /// No outline (no data, or zero contours).
    Empty,
    Simple(Simple<'a>),
    Composite {
        bbox: [i16; 4],
        components: Vec<Component>,
        instructions: Option<&'a [u8]>,
    },
}

impl Glyph<'_> {
    fn x_min(&self) -> i16 {
        match self {
            Glyph::Empty => 0,
            Glyph::Simple(s) => s.bbox[0],
            Glyph::Composite { bbox, .. } => bbox[0],
        }
    }
}

/// Why a glyph cannot go into the transformed table.
enum Refusal {
    /// The glyph has cubic points, which the transform cannot carry.
    Cubic,
    Damaged,
}

impl From<Error> for Refusal {
    fn from(_: Error) -> Self {
        Refusal::Damaged
    }
}

/// Reads a glyph as stored in a `glyf` table.
fn parse(data: &[u8]) -> Result<Glyph<'_>, Refusal> {
    if data.is_empty() {
        return Ok(Glyph::Empty);
    }
    let mut c = Cursor::new(data, "a glyph");
    let contours = c.i16()?;
    let bbox = [c.i16()?, c.i16()?, c.i16()?, c.i16()?];
    if contours == 0 {
        return Ok(Glyph::Empty);
    }
    if contours == -1 {
        let mut components = Vec::new();
        let (mut more, mut instructions) = (true, false);
        while more {
            let (component, m, i) = Component::read(&mut c)?;
            components.push(component);
            more = m;
            instructions |= i;
        }
        let instructions = if instructions {
            let n = usize::try_from(c.i16()?).map_err(|_| Refusal::Damaged)?;
            Some(c.take(n)?)
        } else {
            None
        };
        return Ok(Glyph::Composite { bbox, components, instructions });
    }
    let contours = usize::try_from(contours).map_err(|_| Refusal::Damaged)?;
    let mut end_pts = Vec::with_capacity(contours.min(data.len() / 2));
    for _ in 0..contours {
        let end = c.u16()?;
        if end_pts.last().is_some_and(|&last| end < last) {
            return Err(Refusal::Damaged);
        }
        end_pts.push(end);
    }
    let n = usize::try_from(c.i16()?).map_err(|_| Refusal::Damaged)?;
    let instructions = c.take(n)?;
    let points = end_pts.last().map_or(0, |&e| usize::from(e) + 1);
    let mut flags = Vec::with_capacity(points.min(data.len()));
    while flags.len() < points {
        let flag = c.u8()?;
        let repeat = if flag & REPEAT != 0 { usize::from(c.u8()?) + 1 } else { 1 };
        if flags.len() + repeat > points {
            return Err(Refusal::Damaged);
        }
        flags.extend(std::iter::repeat_n(flag, repeat));
    }
    if flags.iter().any(|f| f & CUBIC != 0) {
        return Err(Refusal::Cubic);
    }
    let mut xs = Vec::with_capacity(points);
    for f in &flags {
        xs.push(coordinate(&mut c, *f, X_SHORT, X_SAME)?);
    }
    let mut deltas = Vec::with_capacity(points);
    for (f, x) in flags.iter().zip(xs) {
        deltas.push((x, coordinate(&mut c, *f, Y_SHORT, Y_SAME)?));
    }
    Ok(Glyph::Simple(Simple {
        bbox,
        end_pts,
        instructions,
        on_curve: flags.iter().map(|f| f & ON_CURVE != 0).collect(),
        overlap: flags.first().is_some_and(|f| f & OVERLAP_SIMPLE != 0),
        deltas,
    }))
}

/// One coordinate move of a simple glyph, as its flag says it is stored.
fn coordinate(c: &mut Cursor<'_>, flag: u8, short: u8, same: u8) -> Result<i32, Error> {
    Ok(if flag & short != 0 {
        let v = i32::from(c.u8()?);
        if flag & same != 0 { v } else { -v }
    } else if flag & same != 0 {
        0
    } else {
        i32::from(c.i16()?)
    })
}

/// The bounding box of points given as moves, or `None` without points.
fn bounds(deltas: &[(i32, i32)]) -> Option<[i64; 4]> {
    let (mut x, mut y) = (0i64, 0i64);
    let mut b: Option<[i64; 4]> = None;
    for (dx, dy) in deltas {
        x += i64::from(*dx);
        y += i64::from(*dy);
        b = Some(match b {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }
    b
}

/// Writes a glyph as fontTools writes it into a `glyf` table.
fn compile(glyph: &Glyph<'_>, out: &mut Vec<u8>) -> Result<(), Error> {
    let header = |out: &mut Vec<u8>, contours: i16, bbox: &[i16; 4]| {
        out.extend_from_slice(&contours.to_be_bytes());
        for v in bbox {
            out.extend_from_slice(&v.to_be_bytes());
        }
    };
    let instruction_length = |n: usize| i16::try_from(n).map_err(|_| bad("a glyph has too many instructions"));
    match glyph {
        Glyph::Empty => {}
        Glyph::Composite { bbox, components, instructions } => {
            header(out, -1, bbox);
            let last = components.len().saturating_sub(1);
            for (i, component) in components.iter().enumerate() {
                component.write(i != last, i == last && instructions.is_some(), out);
            }
            if let Some(code) = instructions {
                out.extend_from_slice(&instruction_length(code.len())?.to_be_bytes());
                out.extend_from_slice(code);
            }
        }
        Glyph::Simple(s) => {
            let contours = i16::try_from(s.end_pts.len()).map_err(|_| bad("a glyph has too many contours"))?;
            header(out, contours, &s.bbox);
            for e in &s.end_pts {
                out.extend_from_slice(&e.to_be_bytes());
            }
            out.extend_from_slice(&instruction_length(s.instructions.len())?.to_be_bytes());
            out.extend_from_slice(s.instructions);
            let (mut flags, mut xs, mut ys) = (Vec::new(), Vec::new(), Vec::new());
            let (mut last, mut repeat) = (None, 0u8);
            for (i, (on, (x, y))) in s.on_curve.iter().zip(&s.deltas).enumerate() {
                let mut flag = if *on { ON_CURVE } else { 0 };
                if i == 0 && s.overlap {
                    flag |= OVERLAP_SIMPLE;
                }
                flag |= pack_coordinate(*x, X_SHORT, X_SAME, &mut xs)?;
                flag |= pack_coordinate(*y, Y_SHORT, Y_SAME, &mut ys)?;
                if last == Some(flag) && repeat != u8::MAX {
                    repeat += 1;
                    if repeat > 1 {
                        flags.pop();
                        flags.pop();
                        flags.push(flag | REPEAT);
                    }
                    flags.push(if repeat > 1 { repeat } else { flag });
                } else {
                    repeat = 0;
                    flags.push(flag);
                }
                last = Some(flag);
            }
            out.extend_from_slice(&flags);
            out.extend_from_slice(&xs);
            out.extend_from_slice(&ys);
        }
    }
    Ok(())
}

/// Stores one coordinate move in its shortest form and returns the flag bits for it.
fn pack_coordinate(v: i32, short: u8, same: u8, out: &mut Vec<u8>) -> Result<u8, Error> {
    if v == 0 {
        return Ok(same);
    }
    if let Ok(byte) = u8::try_from(v.unsigned_abs()) {
        out.push(byte);
        return Ok(if v > 0 { short | same } else { short });
    }
    let v = i16::try_from(v).map_err(|_| bad("a glyph's coordinates are out of range"))?;
    out.extend_from_slice(&v.to_be_bytes());
    Ok(0)
}

/// The glyphs of a `glyf` table as `loca` cuts it, or `None` when `loca` does not fit the table
/// (its size is not a whole number of offsets, offsets run backwards or past the end, or there
/// are more than 65,535 glyphs).
pub(crate) fn glyph_slices<'a>(glyf: &'a [u8], loca: &[u8], long: bool) -> Option<Vec<&'a [u8]>> {
    let size = if long { 4 } else { 2 };
    if loca.is_empty() || !loca.len().is_multiple_of(size) || loca.len() / size - 1 > usize::from(u16::MAX) {
        return None;
    }
    let offsets: Vec<usize> = loca
        .chunks_exact(size)
        .map(|w| {
            if long {
                u32::from_be_bytes(w.try_into().unwrap_or_default()) as usize
            } else {
                2 * usize::from(u16::from_be_bytes(w.try_into().unwrap_or_default()))
            }
        })
        .collect();
    offsets.windows(2).map(|w| glyf.get(*w.first()?..*w.get(1)?)).collect()
}

/// `glyf` laid out again with every glyph padded to four bytes, and the `loca` for it in the
/// shortest offset format: the new `glyf`, the new `loca`, and whether the offsets are long.
pub(crate) fn normalise(glyphs: &[&[u8]]) -> Option<(Vec<u8>, Vec<u8>, bool)> {
    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(glyphs.len() + 1);
    for g in glyphs {
        offsets.push(glyf.len());
        glyf.extend_from_slice(g);
        glyf.resize(crate::pad4(glyf.len())?, 0);
        if glyf.len() > MAX_SFNT {
            return None;
        }
    }
    offsets.push(glyf.len());
    let long = glyf.len() >= SHORT_LOCA_LIMIT;
    if glyf.is_empty() {
        // An outline table with no outlines keeps one byte, as fontTools writes it.
        glyf.push(0);
    }
    Some((glyf, loca(&offsets, long)?, long))
}

/// `loca` for glyph `offsets` (all even when `long` is false).
fn loca(offsets: &[usize], long: bool) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(offsets.len() * if long { 4 } else { 2 });
    for &o in offsets {
        if long {
            out.extend_from_slice(&u32::try_from(o).ok()?.to_be_bytes());
        } else {
            out.extend_from_slice(&u16::try_from(o / 2).ok()?.to_be_bytes());
        }
    }
    Some(out)
}

/// A glyph's `xMin` as a WOFF2 reader sees it, for the `hmtx` transform: 0 for a glyph without
/// data, or (when `glyf` is transformed, which drops their bounding box) without contours.
/// `None` for a glyph too short to hold its header.
pub(crate) fn x_min(glyph: &[u8], glyf_transformed: bool) -> Option<i16> {
    if glyph.is_empty() {
        return Some(0);
    }
    let contours = i16::from_be_bytes(glyph.get(0..2)?.try_into().ok()?);
    let x_min = i16::from_be_bytes(glyph.get(2..4)?.try_into().ok()?);
    glyph.get(..10)?;
    Some(if glyf_transformed && contours == 0 { 0 } else { x_min })
}

/// The seven streams and two bitmaps of a transformed `glyf` table.
#[derive(Default)]
struct Streams {
    n_contour: Vec<u8>,
    n_points: Vec<u8>,
    flags: Vec<u8>,
    glyph: Vec<u8>,
    composite: Vec<u8>,
    bbox_bitmap: Vec<u8>,
    bbox: Vec<u8>,
    instructions: Vec<u8>,
    overlap_bitmap: Vec<u8>,
}

fn set_bit(bitmap: &mut [u8], id: usize) {
    if let Some(byte) = bitmap.get_mut(id >> 3) {
        *byte |= 0x80 >> (id & 7);
    }
}

fn bit(bitmap: &[u8], id: usize) -> bool {
    bitmap.get(id >> 3).is_some_and(|byte| byte & (0x80 >> (id & 7)) != 0)
}

/// The transformed `glyf` table for `glyphs`, with `index_format` as the `loca` format to rebuild.
/// `None` when a glyph cannot be carried (cubic points or damaged data).
pub(crate) fn transform(glyphs: &[&[u8]], index_format: u16) -> Option<Vec<u8>> {
    let count = u16::try_from(glyphs.len()).ok()?;
    let mut s =
        Streams { bbox_bitmap: vec![0; glyphs.len().div_ceil(32) * 4], overlap_bitmap: vec![0; glyphs.len().div_ceil(8)], ..Streams::default() };
    for (id, data) in glyphs.iter().enumerate() {
        let glyph = parse(data).ok()?;
        match &glyph {
            Glyph::Empty => s.n_contour.extend_from_slice(&0i16.to_be_bytes()),
            Glyph::Composite { bbox, components, instructions } => {
                s.n_contour.extend_from_slice(&(-1i16).to_be_bytes());
                let last = components.len().saturating_sub(1);
                for (i, component) in components.iter().enumerate() {
                    component.write(i != last, i == last && instructions.is_some(), &mut s.composite);
                }
                if let Some(code) = instructions {
                    push_255u16(&mut s.glyph, u16::try_from(code.len()).ok()?);
                    s.instructions.extend_from_slice(code);
                }
                push_bbox(&mut s, id, bbox);
            }
            Glyph::Simple(g) => {
                s.n_contour.extend_from_slice(&i16::try_from(g.end_pts.len()).ok()?.to_be_bytes());
                let mut last = -1i32;
                for &end in &g.end_pts {
                    push_255u16(&mut s.n_points, u16::try_from(i32::from(end) - last).ok()?);
                    last = i32::from(end);
                }
                for (on, (dx, dy)) in g.on_curve.iter().zip(&g.deltas) {
                    push_triplet(&mut s, *dx, *dy, *on);
                }
                push_255u16(&mut s.glyph, u16::try_from(g.instructions.len()).ok()?);
                s.instructions.extend_from_slice(g.instructions);
                if g.overlap {
                    set_bit(&mut s.overlap_bitmap, id);
                }
                let computed = bounds(&g.deltas).unwrap_or_default();
                if g.bbox.map(i64::from) != computed {
                    push_bbox(&mut s, id, &g.bbox);
                }
            }
        }
    }
    let overlap = s.overlap_bitmap.iter().any(|b| *b != 0);
    let mut bbox_stream = std::mem::take(&mut s.bbox_bitmap);
    bbox_stream.extend_from_slice(&s.bbox);
    let streams = [&s.n_contour, &s.n_points, &s.flags, &s.glyph, &s.composite, &bbox_stream, &s.instructions];
    let mut out = Vec::with_capacity(HEADER + streams.iter().map(|v| v.len()).sum::<usize>() + s.overlap_bitmap.len());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(if overlap { HAS_OVERLAP_BITMAP } else { 0 }).to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&index_format.to_be_bytes());
    for v in streams {
        out.extend_from_slice(&u32::try_from(v.len()).ok()?.to_be_bytes());
    }
    for v in streams {
        out.extend_from_slice(v);
    }
    if overlap {
        out.extend_from_slice(&s.overlap_bitmap);
    }
    Some(out)
}

fn push_bbox(s: &mut Streams, id: usize, bbox: &[i16; 4]) {
    set_bit(&mut s.bbox_bitmap, id);
    for v in bbox {
        s.bbox.extend_from_slice(&v.to_be_bytes());
    }
}

/// Appends one point as a flag byte and one to four triplet bytes, in the first of the format's
/// encodings that holds the move.
fn push_triplet(s: &mut Streams, dx: i32, dy: i32, on_curve: bool) {
    let (ax, ay) = (dx.unsigned_abs(), dy.unsigned_abs());
    let on = if on_curve { 0u32 } else { 128 };
    let (x_sign, y_sign) = (u32::from(dx >= 0), u32::from(dy >= 0));
    let signs = x_sign + 2 * y_sign;
    let (flag, bytes): (u32, &[u32]) = if dx == 0 && ay < 1280 {
        (on + ((ay & 0xf00) >> 7) + y_sign, &[ay & 0xff])
    } else if dy == 0 && ax < 1280 {
        (on + 10 + ((ax & 0xf00) >> 7) + x_sign, &[ax & 0xff])
    } else if ax < 65 && ay < 65 {
        let (x1, y1) = (ax.saturating_sub(1), ay.saturating_sub(1));
        (on + 20 + (x1 & 0x30) + ((y1 & 0x30) >> 2) + signs, &[((x1 & 0xf) << 4) | (y1 & 0xf)])
    } else if ax < 769 && ay < 769 {
        let (x1, y1) = (ax.saturating_sub(1), ay.saturating_sub(1));
        (on + 84 + 12 * ((x1 & 0x300) >> 8) + ((y1 & 0x300) >> 6) + signs, &[x1 & 0xff, y1 & 0xff])
    } else if ax < 4096 && ay < 4096 {
        (on + 120 + signs, &[ax >> 4, ((ax & 0xf) << 4) | (ay >> 8), ay & 0xff])
    } else {
        (on + 124 + signs, &[ax >> 8, ax & 0xff, ay >> 8, ay & 0xff])
    };
    s.flags.push(flag as u8);
    s.glyph.extend(bytes.iter().map(|b| *b as u8));
}

/// A `glyf` and `loca` rebuilt from the transformed table.
pub(crate) struct Rebuilt {
    pub glyf: Vec<u8>,
    pub loca: Vec<u8>,
    /// Every glyph's `xMin` (0 for empty glyphs), for the `hmtx` transform.
    pub x_mins: Vec<i16>,
}

/// The streams of a transformed table, each read from its own cursor.
struct Input<'a> {
    n_contour: Cursor<'a>,
    n_points: Cursor<'a>,
    flags: Cursor<'a>,
    glyph: Cursor<'a>,
    composite: Cursor<'a>,
    bbox_bitmap: &'a [u8],
    bbox: Cursor<'a>,
    instructions: Cursor<'a>,
    overlap_bitmap: Option<&'a [u8]>,
}

/// Rebuilds `glyf` and `loca` from a transformed `glyf` table.
pub(crate) fn reconstruct(data: &[u8]) -> Result<Rebuilt, Error> {
    let mut c = Cursor::new(data, "the transformed glyf table");
    let _version = c.u16()?;
    let options = c.u16()?;
    let count = usize::from(c.u16()?);
    let index_format = c.u16()?;
    let mut sizes = [0usize; 7];
    for size in &mut sizes {
        *size = c.u32()? as usize;
    }
    let [n_contour, n_points, flags, glyph, composite, bbox, instructions] = sizes.map(|n| c.take(n));
    let overlap_bitmap = if options & HAS_OVERLAP_BITMAP != 0 { Some(c.take(count.div_ceil(8))?) } else { None };
    if c.at != data.len() {
        return Err(bad("the transformed glyf table is longer than its streams"));
    }
    let n_contour = n_contour?;
    if n_contour.len() != 2 * count {
        return Err(bad("the transformed glyf table's contour counts do not match its glyph count"));
    }
    let bbox = bbox?;
    let bitmap_len = count.div_ceil(32) * 4;
    let bbox_bitmap = bbox.get(..bitmap_len).ok_or_else(|| bad("the transformed glyf table's bounding box bitmap is cut short"))?;
    let mut input = Input {
        n_contour: Cursor::new(n_contour, "the contour count stream"),
        n_points: Cursor::new(n_points?, "the point count stream"),
        flags: Cursor::new(flags?, "the flag stream"),
        glyph: Cursor::new(glyph?, "the glyph stream"),
        composite: Cursor::new(composite?, "the composite stream"),
        bbox_bitmap,
        bbox: Cursor::new(bbox.get(bitmap_len..).unwrap_or_default(), "the bounding box stream"),
        instructions: Cursor::new(instructions?, "the instruction stream"),
        overlap_bitmap,
    };
    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(count + 1);
    let mut x_mins = Vec::with_capacity(count);
    for id in 0..count {
        let g = decode_glyph(&mut input, id)?;
        offsets.push(glyf.len());
        compile(&g, &mut glyf)?;
        if glyf.len() > MAX_SFNT {
            return Err(bad("the rebuilt glyf table is too large"));
        }
        x_mins.push(g.x_min());
    }
    offsets.push(glyf.len());
    let odd_length = |w: &[usize]| matches!(w, [a, b] if b.saturating_sub(*a) % 2 == 1);
    let odd = offsets.windows(2).filter(|w| odd_length(w)).count();
    let total = glyf.len();
    if total < SHORT_LOCA_LIMIT && odd > 0 && total + odd < SHORT_LOCA_LIMIT {
        // Small enough for short offsets once each odd glyph gets a pad byte; copying is cheap
        // below this size.
        let mut padded = Vec::with_capacity(total + odd);
        let mut padded_offsets = Vec::with_capacity(count + 1);
        for w in offsets.windows(2) {
            let &[a, b] = w else { continue };
            padded_offsets.push(padded.len());
            padded.extend_from_slice(glyf.get(a..b).unwrap_or_default());
            if odd_length(w) {
                padded.push(0);
            }
        }
        padded_offsets.push(padded.len());
        glyf = padded;
        offsets = padded_offsets;
    }
    let long = index_format != 0;
    if !long && (glyf.len() >= SHORT_LOCA_LIMIT || offsets.iter().any(|o| o % 2 == 1)) {
        return Err(bad("the glyphs do not fit the short loca format the transformed glyf table names"));
    }
    let loca = loca(&offsets, long).ok_or_else(|| bad("the rebuilt glyf table is too large"))?;
    if glyf.is_empty() {
        glyf.push(0);
    }
    Ok(Rebuilt { glyf, loca, x_mins })
}

fn decode_glyph<'a>(s: &mut Input<'a>, id: usize) -> Result<Glyph<'a>, Error> {
    let contours = s.n_contour.i16()?;
    if contours == 0 {
        return Ok(Glyph::Empty);
    }
    let read_bbox = |s: &mut Input<'a>| -> Result<[i16; 4], Error> { Ok([s.bbox.i16()?, s.bbox.i16()?, s.bbox.i16()?, s.bbox.i16()?]) };
    if contours == -1 {
        let mut components = Vec::new();
        let (mut more, mut has_code) = (true, false);
        while more {
            let (component, m, i) = Component::read(&mut s.composite)?;
            components.push(component);
            more = m;
            has_code |= i;
        }
        let instructions = if has_code {
            let n = usize::from(s.glyph.u255()?);
            Some(s.instructions.take(n)?)
        } else {
            None
        };
        if !bit(s.bbox_bitmap, id) {
            return Err(bad(format!("composite glyph {id} has no bounding box")));
        }
        return Ok(Glyph::Composite { bbox: read_bbox(s)?, components, instructions });
    }
    let contours = usize::try_from(contours).map_err(|_| bad(format!("glyph {id} has a negative contour count")))?;
    let mut end_pts = Vec::with_capacity(contours.min(s.n_points.data.len()));
    let mut end = -1i64;
    for _ in 0..contours {
        end += i64::from(s.n_points.u255()?);
        end_pts.push(u16::try_from(end).map_err(|_| bad(format!("glyph {id} has too many points or an empty first contour")))?);
    }
    let points = usize::try_from(end + 1).unwrap_or(0);
    let flags = s.flags.take(points)?;
    let mut on_curve = Vec::with_capacity(points);
    let mut deltas = Vec::with_capacity(points);
    for &flag in flags {
        on_curve.push(flag & 0x80 == 0);
        deltas.push(read_triplet(&mut s.glyph, flag & 0x7f)?);
    }
    let n = usize::from(s.glyph.u255()?);
    let instructions = s.instructions.take(n)?;
    let overlap = s.overlap_bitmap.is_some_and(|b| bit(b, id));
    let bbox = if bit(s.bbox_bitmap, id) {
        read_bbox(s)?
    } else {
        let b = bounds(&deltas).unwrap_or_default();
        let v = |i: usize| b.get(i).and_then(|v| i16::try_from(*v).ok()).ok_or_else(|| bad(format!("glyph {id} is out of range")));
        [v(0)?, v(1)?, v(2)?, v(3)?]
    };
    Ok(Glyph::Simple(Simple { bbox, end_pts, instructions, on_curve, overlap, deltas }))
}

/// Reads the triplet bytes of one point whose flag (without the on-curve bit) is `flag`.
fn read_triplet(c: &mut Cursor<'_>, flag: u8) -> Result<(i32, i32), Error> {
    let with_sign = |f: u8, v: i32| if f & 1 != 0 { v } else { -v };
    let flag32 = i32::from(flag);
    Ok(if flag < 10 {
        (0, with_sign(flag, ((flag32 & 14) << 7) + i32::from(c.u8()?)))
    } else if flag < 20 {
        (with_sign(flag, (((flag32 - 10) & 14) << 7) + i32::from(c.u8()?)), 0)
    } else if flag < 84 {
        let b0 = flag32 - 20;
        let b1 = i32::from(c.u8()?);
        (with_sign(flag, 1 + (b0 & 0x30) + (b1 >> 4)), with_sign(flag >> 1, 1 + ((b0 & 0x0c) << 2) + (b1 & 0x0f)))
    } else if flag < 120 {
        let b0 = flag32 - 84;
        let (t0, t1) = (i32::from(c.u8()?), i32::from(c.u8()?));
        (with_sign(flag, 1 + ((b0 / 12) << 8) + t0), with_sign(flag >> 1, 1 + (((b0 % 12) >> 2) << 8) + t1))
    } else if flag < 124 {
        let (t0, t1, t2) = (i32::from(c.u8()?), i32::from(c.u8()?), i32::from(c.u8()?));
        (with_sign(flag, (t0 << 4) + (t1 >> 4)), with_sign(flag >> 1, ((t1 & 0x0f) << 8) + t2))
    } else {
        let (t0, t1, t2, t3) = (i32::from(c.u8()?), i32::from(c.u8()?), i32::from(c.u8()?), i32::from(c.u8()?));
        (with_sign(flag, (t0 << 8) + t1), with_sign(flag >> 1, (t2 << 8) + t3))
    })
}

/// Every glyph's `xMin` from an untransformed `glyf` and `loca`, for the first `count` glyphs.
pub(crate) fn x_mins(glyf: &[u8], loca: &[u8], long: bool, count: usize) -> Result<Vec<i16>, Error> {
    let glyphs = glyph_slices(glyf, loca, long).ok_or_else(|| bad("the loca table does not fit the glyf table"))?;
    let glyphs = glyphs.get(..count).ok_or_else(|| bad("the loca table lists fewer glyphs than maxp"))?;
    glyphs.iter().map(|g| x_min(g, false).ok_or_else(|| bad("a glyph is cut short"))).collect()
}

#[cfg(test)]
pub(crate) mod test_glyphs {
    //! Glyph builders for the crate's tests.
    use super::*;

    /// A simple glyph from contours of absolute points `(x, y, on_curve)`, written as fontTools
    /// writes it; `bbox` overrides the computed bounding box.
    pub fn simple(contours: &[&[(i32, i32, bool)]], instructions: &[u8], overlap: bool, bbox: Option<[i16; 4]>) -> Vec<u8> {
        let mut end_pts = Vec::new();
        let (mut on_curve, mut deltas) = (Vec::new(), Vec::new());
        let (mut px, mut py) = (0, 0);
        for c in contours {
            for &(x, y, on) in *c {
                deltas.push((x - px, y - py));
                on_curve.push(on);
                (px, py) = (x, y);
            }
            end_pts.push(u16::try_from(deltas.len() - 1).unwrap());
        }
        let b = bounds(&deltas).unwrap();
        let bbox = bbox.unwrap_or(b.map(|v| i16::try_from(v).unwrap()));
        let mut out = Vec::new();
        compile(&Glyph::Simple(Simple { bbox, end_pts, instructions, on_curve, overlap, deltas }), &mut out).unwrap();
        out
    }

    /// A composite glyph of `(glyph, dx, dy, scale)` components.
    pub fn composite(parts: &[(u16, i16, i16, Option<[i16; 4]>)], bbox: [i16; 4], instructions: Option<&[u8]>) -> Vec<u8> {
        let components =
            parts.iter().map(|&(glyph, x, y, transform)| Component { flags: 0x0004, glyph, args: Args::Offset(x, y), transform }).collect();
        let mut out = Vec::new();
        compile(&Glyph::Composite { bbox, components, instructions }, &mut out).unwrap();
        out
    }
}
