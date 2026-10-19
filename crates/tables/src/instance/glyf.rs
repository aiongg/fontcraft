//! `glyf`, `loca`, `hmtx` and `vmtx` of a full instance: every glyph's points moved by its `gvar`
//! deltas (left-out points inferred), bounds recomputed and metrics taken from the four phantom
//! points. Follows fontTools' `instantiateGvar`, `_getCoordinatesAndControls`, `_setCoordinates`
//! and `Glyph.recalcBounds` (MIT, Copyright (c) 2013 Google Inc. and the fontTools authors):
//! glyphs are visited in order of component depth so that a composite's bounds use its
//! components' new outlines; points are rounded once moved; component offsets keep their
//! fractions until the glyph is written, as fontTools keeps them in memory.

use read_fonts::tables::gvar::Gvar;
use read_fonts::types::GlyphId;
use read_fonts::{FontData, FontRead};

use super::iup::{Pair, iup_delta};
use super::{ot_round, tuple_scalar};
use crate::limits::MAX_TUPLE_VARIATIONS;
use crate::{Result, TablesError};

/// Deepest component nesting followed.
const MAX_COMPONENT_DEPTH: usize = 64;
/// Most work done in all flattening composites to compute their bounds in one instance: one
/// unit per component visited (empty glyphs included) and one per point copied.
pub(super) const MAX_FLATTEN_WORK: usize = 20_000_000;
/// Most points one simple glyph may have.
const MAX_POINTS: usize = 65_536;

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;
const OVERLAP_COMPOUND: u16 = 0x0400;
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;
const FLAG_ON_CURVE: u8 = 0x01;
const FLAG_X_SHORT: u8 = 0x02;
const FLAG_Y_SHORT: u8 = 0x04;
const FLAG_REPEAT: u8 = 0x08;
const FLAG_X_SAME: u8 = 0x10;
const FLAG_Y_SAME: u8 = 0x20;
const FLAG_OVERLAP_SIMPLE: u8 = 0x40;
/// The point flags kept when a glyph is written again: on-curve, overlap and cubic.
const KEPT_POINT_FLAGS: u8 = FLAG_ON_CURVE | FLAG_OVERLAP_SIMPLE | 0x80;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Simple {
    pub bbox: [i16; 4],
    pub end_pts: Vec<u16>,
    pub instructions: Vec<u8>,
    pub flags: Vec<u8>,
    pub points: Vec<Pair>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Component {
    pub flags: u16,
    pub gid: u16,
    /// Offsets (when [`ARGS_ARE_XY_VALUES`]); fractional once moved.
    pub dx: f64,
    pub dy: f64,
    /// Matching point numbers (otherwise).
    pub points: (u16, u16),
    /// The transform as stored (2, 4 or 8 bytes) and as a matrix `[xx, xy, yx, yy]`.
    pub transform_raw: Vec<u8>,
    pub transform: Option<[f64; 4]>,
}

impl Component {
    fn is_xy(&self) -> bool {
        self.flags & ARGS_ARE_XY_VALUES != 0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Composite {
    pub bbox: [i16; 4],
    pub components: Vec<Component>,
    /// `Some` when the glyph has an instruction block, even an empty one.
    pub instructions: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Glyph {
    Empty,
    Simple(Simple),
    Composite(Composite),
}

impl Glyph {
    /// The bounds as stored or last computed; 0s for an empty glyph.
    pub fn bbox(&self) -> [i16; 4] {
        match self {
            Glyph::Empty => [0; 4],
            Glyph::Simple(s) => s.bbox,
            Glyph::Composite(c) => c.bbox,
        }
    }
}

/// Advance and side bearing of every glyph, from `hmtx` or `vmtx`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Metrics {
    pub advances: Vec<u16>,
    pub bearings: Vec<i16>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes([s.first().copied().unwrap_or(0), s.get(1).copied().unwrap_or(0)]))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|v| v as i16)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some((u32::from(u16_at(b, at)?) << 16) | u32::from(u16_at(b, at + 2)?))
}

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

/// Reads `hmtx` (with `hhea`) or `vmtx` (with `vhea`) for `glyphs` glyphs. Glyphs past the long
/// metrics share the last advance; missing side bearings read as 0.
pub(super) fn read_metrics(mtx: &[u8], hea: &[u8], glyphs: usize) -> Result<Metrics> {
    let long = usize::from(u16_at(hea, 34).ok_or_else(|| bad("the metrics header table is too short"))?);
    if long == 0 && glyphs > 0 {
        return Err(bad("the metrics header says there are no long metrics"));
    }
    let long = long.min(glyphs);
    let mut advances = Vec::with_capacity(glyphs);
    let mut bearings = Vec::with_capacity(glyphs);
    for g in 0..long {
        let adv = u16_at(mtx, 4 * g).ok_or_else(|| bad("the metrics table is shorter than its header says"))?;
        advances.push(adv);
        bearings.push(i16_at(mtx, 4 * g + 2).unwrap_or(0));
    }
    let last = advances.last().copied().unwrap_or(0);
    for g in long..glyphs {
        advances.push(last);
        bearings.push(i16_at(mtx, 4 * long + 2 * (g - long)).unwrap_or(0));
    }
    Ok(Metrics { advances, bearings })
}

/// The metrics table and its number of long metrics: trailing glyphs that share the last advance
/// keep only their side bearing, as fontTools writes it.
pub(super) fn write_metrics(m: &Metrics) -> (Vec<u8>, u16) {
    let n = m.advances.len();
    let mut long = n;
    if let Some(&last) = m.advances.last() {
        while long > 1 && m.advances.get(long - 2) == Some(&last) {
            long -= 1;
        }
    }
    let mut out = Vec::with_capacity(4 * long + 2 * (n - long));
    for (i, (a, b)) in m.advances.iter().zip(&m.bearings).enumerate() {
        if i < long {
            out.extend_from_slice(&a.to_be_bytes());
        }
        out.extend_from_slice(&b.to_be_bytes());
    }
    (out, u16::try_from(long).unwrap_or(u16::MAX))
}

/// Every glyph of `glyf` as located by `loca`.
pub(super) fn read_glyf(glyf: &[u8], loca: &[u8], long_loca: bool, glyphs: usize) -> Result<Vec<Glyph>> {
    let offset = |i: usize| -> Option<usize> {
        if long_loca { u32_at(loca, 4 * i).map(|v| v as usize) } else { u16_at(loca, 2 * i).map(|v| 2 * usize::from(v)) }
    };
    let mut out = Vec::with_capacity(glyphs);
    for g in 0..glyphs {
        let (Some(start), Some(end)) = (offset(g), offset(g + 1)) else {
            return Err(bad("the loca table is shorter than the glyph count needs"));
        };
        if end <= start {
            out.push(Glyph::Empty);
            continue;
        }
        let data = glyf.get(start..end).ok_or_else(|| bad(format!("glyph {g} lies outside the glyf table")))?;
        out.push(parse_glyph(data).map_err(|e| bad(format!("glyph {g}: {e}")))?);
    }
    Ok(out)
}

fn parse_glyph(b: &[u8]) -> std::result::Result<Glyph, String> {
    let contours = i16_at(b, 0).ok_or("the glyph header is truncated")?;
    let bbox = [i16_at(b, 2), i16_at(b, 4), i16_at(b, 6), i16_at(b, 8)];
    let [Some(x0), Some(y0), Some(x1), Some(y1)] = bbox else { return Err("the glyph header is truncated".into()) };
    let bbox = [x0, y0, x1, y1];
    if contours == 0 {
        return Ok(Glyph::Simple(Simple { bbox, end_pts: Vec::new(), instructions: Vec::new(), flags: Vec::new(), points: Vec::new() }));
    }
    if contours < 0 {
        return parse_composite(b, bbox);
    }
    let n = contours as usize;
    let mut end_pts = Vec::with_capacity(n);
    for i in 0..n {
        end_pts.push(u16_at(b, 10 + 2 * i).ok_or("the contour ends are truncated")?);
    }
    let points = end_pts.last().map_or(0, |e| usize::from(*e) + 1);
    if points > MAX_POINTS {
        return Err("too many points".into());
    }
    let mut at = 10 + 2 * n;
    let ilen = usize::from(u16_at(b, at).ok_or("the instruction length is truncated")?);
    at += 2;
    let instructions = b.get(at..at + ilen).ok_or("the instructions are truncated")?.to_vec();
    at += ilen;
    let mut flags = Vec::with_capacity(points);
    while flags.len() < points {
        let f = *b.get(at).ok_or("the point flags are truncated")?;
        at += 1;
        let mut repeat = 0usize;
        if f & FLAG_REPEAT != 0 {
            repeat = usize::from(*b.get(at).ok_or("the point flags are truncated")?);
            at += 1;
        }
        for _ in 0..=repeat {
            if flags.len() < points {
                flags.push(f);
            }
        }
    }
    let mut read_axis = |short: u8, same: u8| -> std::result::Result<Vec<f64>, String> {
        let mut v = 0i32;
        let mut out = Vec::with_capacity(points);
        for f in &flags {
            if f & short != 0 {
                let d = i32::from(*b.get(at).ok_or("the coordinates are truncated")?);
                at += 1;
                v += if f & same != 0 { d } else { -d };
            } else if f & same == 0 {
                v += i32::from(i16_at(b, at).ok_or("the coordinates are truncated")?);
                at += 2;
            }
            out.push(f64::from(v));
        }
        Ok(out)
    };
    let xs = read_axis(FLAG_X_SHORT, FLAG_X_SAME)?;
    let ys = read_axis(FLAG_Y_SHORT, FLAG_Y_SAME)?;
    let flags = flags.into_iter().map(|f| f & KEPT_POINT_FLAGS).collect();
    Ok(Glyph::Simple(Simple { bbox, end_pts, instructions, flags, points: xs.into_iter().zip(ys).collect() }))
}

fn f2dot14(v: u16) -> f64 {
    f64::from(v as i16) / 16384.0
}

fn parse_composite(b: &[u8], bbox: [i16; 4]) -> std::result::Result<Glyph, String> {
    let mut at = 10;
    let mut components = Vec::new();
    let mut instructions = false;
    loop {
        let flags = u16_at(b, at).ok_or("a component is truncated")?;
        let gid = u16_at(b, at + 2).ok_or("a component is truncated")?;
        at += 4;
        let (a1, a2) = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            let r = (u16_at(b, at), u16_at(b, at + 2));
            at += 4;
            match r {
                (Some(a), Some(c)) if flags & ARGS_ARE_XY_VALUES != 0 => (i32::from(a as i16), i32::from(c as i16)),
                (Some(a), Some(c)) => (i32::from(a), i32::from(c)),
                _ => return Err("a component is truncated".into()),
            }
        } else {
            let r = (b.get(at).copied(), b.get(at + 1).copied());
            at += 2;
            match r {
                (Some(a), Some(c)) if flags & ARGS_ARE_XY_VALUES != 0 => (i32::from(a as i8), i32::from(c as i8)),
                (Some(a), Some(c)) => (i32::from(a), i32::from(c)),
                _ => return Err("a component is truncated".into()),
            }
        };
        let tlen = if flags & WE_HAVE_A_SCALE != 0 {
            2
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            4
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            8
        } else {
            0
        };
        let transform_raw = b.get(at..at + tlen).ok_or("a component transform is truncated")?.to_vec();
        at += tlen;
        let word = |i: usize| f2dot14(u16_at(&transform_raw, 2 * i).unwrap_or(0));
        let transform = match tlen {
            2 => Some([word(0), 0.0, 0.0, word(0)]),
            4 => Some([word(0), 0.0, 0.0, word(1)]),
            8 => Some([word(0), word(1), word(2), word(3)]),
            _ => None,
        };
        let xy = flags & ARGS_ARE_XY_VALUES != 0;
        components.push(Component {
            flags,
            gid,
            dx: if xy { f64::from(a1) } else { 0.0 },
            dy: if xy { f64::from(a2) } else { 0.0 },
            points: if xy { (0, 0) } else { (a1 as u16, a2 as u16) },
            transform_raw,
            transform,
        });
        instructions |= flags & WE_HAVE_INSTRUCTIONS != 0;
        if flags & MORE_COMPONENTS == 0 {
            break;
        }
    }
    let instructions = if instructions {
        let len = usize::from(u16_at(b, at).ok_or("the instruction length is truncated")?);
        Some(b.get(at + 2..at + 2 + len).ok_or("the instructions are truncated")?.to_vec())
    } else {
        None
    };
    Ok(Glyph::Composite(Composite { bbox, components, instructions }))
}

/// `glyf` and `loca` for `glyphs`, each glyph padded to four bytes, and whether `loca` is long
/// (short when every offset fits, as fontTools chooses).
pub(super) fn write_glyf(glyphs: &[Glyph]) -> Result<(Vec<u8>, Vec<u8>, bool)> {
    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(glyphs.len() + 1);
    for (g, glyph) in glyphs.iter().enumerate() {
        offsets.push(glyf.len());
        let bytes = encode_glyph(glyph).map_err(|e| bad(format!("glyph {g} cannot be written: {e}")))?;
        glyf.extend_from_slice(&bytes);
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }
        if glyf.len() > crate::limits::MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the glyf table", limit: crate::limits::MAX_SFNT });
        }
    }
    offsets.push(glyf.len());
    let long = glyf.len() >= 0x20000;
    let mut loca = Vec::with_capacity(offsets.len() * if long { 4 } else { 2 });
    for o in offsets {
        if long {
            loca.extend_from_slice(&u32::try_from(o).unwrap_or(u32::MAX).to_be_bytes());
        } else {
            loca.extend_from_slice(&u16::try_from(o / 2).unwrap_or(u16::MAX).to_be_bytes());
        }
    }
    Ok((glyf, loca, long))
}

fn coord(v: f64) -> std::result::Result<i16, String> {
    let r = ot_round(v);
    if (-32768.0..=32767.0).contains(&r) { Ok(r as i16) } else { Err(format!("the coordinate {r} is outside the 16-bit range")) }
}

fn encode_glyph(glyph: &Glyph) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::new();
    match glyph {
        Glyph::Empty => {}
        Glyph::Simple(s) if s.end_pts.is_empty() => {
            out.extend_from_slice(&0i16.to_be_bytes());
            for v in s.bbox {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
        Glyph::Simple(s) => {
            let n = i16::try_from(s.end_pts.len()).map_err(|_| "too many contours")?;
            out.extend_from_slice(&n.to_be_bytes());
            for v in s.bbox {
                out.extend_from_slice(&v.to_be_bytes());
            }
            for e in &s.end_pts {
                out.extend_from_slice(&e.to_be_bytes());
            }
            let ilen = u16::try_from(s.instructions.len()).map_err(|_| "the instructions are too long")?;
            out.extend_from_slice(&ilen.to_be_bytes());
            out.extend_from_slice(&s.instructions);
            let mut flags = Vec::with_capacity(s.points.len());
            let (mut xs, mut ys) = (Vec::new(), Vec::new());
            let (mut px, mut py) = (0i32, 0i32);
            for (i, (x, y)) in s.points.iter().enumerate() {
                let (x, y) = (i32::from(coord(*x)?), i32::from(coord(*y)?));
                let mut f = s.flags.get(i).copied().unwrap_or(FLAG_ON_CURVE) & KEPT_POINT_FLAGS;
                f |= delta(x - px, FLAG_X_SHORT, FLAG_X_SAME, &mut xs);
                f |= delta(y - py, FLAG_Y_SHORT, FLAG_Y_SAME, &mut ys);
                (px, py) = (x, y);
                flags.push(f);
            }
            // Runs of equal flags use the repeat count.
            let mut i = 0;
            while let Some(&f) = flags.get(i) {
                let mut run = 1;
                while run < 256 && flags.get(i + run) == Some(&f) {
                    run += 1;
                }
                if run > 1 {
                    out.push(f | FLAG_REPEAT);
                    out.push((run - 1) as u8);
                } else {
                    out.push(f);
                }
                i += run;
            }
            out.extend_from_slice(&xs);
            out.extend_from_slice(&ys);
        }
        Glyph::Composite(c) => {
            out.extend_from_slice(&(-1i16).to_be_bytes());
            for v in c.bbox {
                out.extend_from_slice(&v.to_be_bytes());
            }
            let last = c.components.len().saturating_sub(1);
            for (i, comp) in c.components.iter().enumerate() {
                let mut flags = comp.flags & !(ARG_1_AND_2_ARE_WORDS | MORE_COMPONENTS | WE_HAVE_INSTRUCTIONS);
                if i < last {
                    flags |= MORE_COMPONENTS;
                } else if c.instructions.is_some() {
                    flags |= WE_HAVE_INSTRUCTIONS;
                }
                let (a1, a2) = if comp.is_xy() {
                    (i32::from(coord(comp.dx)?), i32::from(coord(comp.dy)?))
                } else {
                    (i32::from(comp.points.0), i32::from(comp.points.1))
                };
                let fits = if comp.is_xy() { (-128..=127).contains(&a1) && (-128..=127).contains(&a2) } else { a1 <= 255 && a2 <= 255 };
                if !fits {
                    flags |= ARG_1_AND_2_ARE_WORDS;
                }
                out.extend_from_slice(&flags.to_be_bytes());
                out.extend_from_slice(&comp.gid.to_be_bytes());
                if fits {
                    out.push(a1 as u8);
                    out.push(a2 as u8);
                } else {
                    out.extend_from_slice(&(a1 as u16).to_be_bytes());
                    out.extend_from_slice(&(a2 as u16).to_be_bytes());
                }
                out.extend_from_slice(&comp.transform_raw);
            }
            if let Some(ins) = &c.instructions {
                let ilen = u16::try_from(ins.len()).map_err(|_| "the instructions are too long")?;
                out.extend_from_slice(&ilen.to_be_bytes());
                out.extend_from_slice(ins);
            }
        }
    }
    Ok(out)
}

/// Writes one coordinate delta and returns its flag bits.
fn delta(d: i32, short: u8, same: u8, out: &mut Vec<u8>) -> u8 {
    if d == 0 {
        same
    } else if (-255..=255).contains(&d) {
        out.push(d.unsigned_abs() as u8);
        if d > 0 { short | same } else { short }
    } else {
        out.extend_from_slice(&(d as i16).to_be_bytes());
        0
    }
}

/// Each glyph's component depth: 0 for simple and empty glyphs, one more than the deepest
/// component for composites. A cycle or a component past the glyph count is refused.
pub(super) fn depths(glyphs: &[Glyph]) -> Result<Vec<usize>> {
    fn depth(glyphs: &[Glyph], g: usize, memo: &mut [Option<usize>], level: usize) -> Result<usize> {
        if let Some(Some(d)) = memo.get(g) {
            return Ok(*d);
        }
        if level > MAX_COMPONENT_DEPTH {
            return Err(bad(format!("glyph {g}: components nest deeper than {MAX_COMPONENT_DEPTH} levels, or refer to themselves")));
        }
        let d = match glyphs.get(g) {
            Some(Glyph::Composite(c)) => {
                let mut deepest = 0;
                for comp in &c.components {
                    let child = usize::from(comp.gid);
                    if child >= glyphs.len() {
                        return Err(bad(format!("glyph {g} uses glyph {child}, past the glyph count")));
                    }
                    deepest = deepest.max(depth(glyphs, child, memo, level + 1)?);
                }
                deepest + 1
            }
            _ => 0,
        };
        if let Some(slot) = memo.get_mut(g) {
            *slot = Some(d);
        }
        Ok(d)
    }
    let mut memo = vec![None; glyphs.len()];
    (0..glyphs.len()).map(|g| depth(glyphs, g, &mut memo, 0)).collect()
}

fn apply_transform(points: &mut [Pair], t: [f64; 4]) {
    for p in points.iter_mut() {
        let (x, y) = *p;
        *p = (x * t[0] + y * t[2], x * t[1] + y * t[3]);
    }
}

/// The points of glyph `g` with its components placed, as fontTools' `getCoordinates` with
/// rounding: simple glyphs' points are integers already, nested composites are not rounded
/// until the end.
fn flatten(glyphs: &[Glyph], g: usize, level: usize, budget: &mut usize) -> Result<Vec<Pair>> {
    if level > MAX_COMPONENT_DEPTH {
        return Err(bad(format!("glyph {g}: components nest deeper than {MAX_COMPONENT_DEPTH} levels")));
    }
    let charge = |budget: &mut usize, n: usize| {
        *budget = budget
            .checked_sub(n)
            .ok_or(TablesError::TooLarge { what: "the component points flattened to compute composite bounds", limit: MAX_FLATTEN_WORK })?;
        Ok::<(), TablesError>(())
    };
    match glyphs.get(g) {
        Some(Glyph::Simple(s)) => {
            charge(budget, s.points.len())?;
            Ok(s.points.clone())
        }
        Some(Glyph::Composite(c)) => {
            let mut all: Vec<Pair> = Vec::new();
            for comp in &c.components {
                charge(budget, 1)?;
                let mut pts = flatten(glyphs, usize::from(comp.gid), level + 1, budget)?;
                if comp.is_xy() {
                    let scaled_offset = comp.flags & SCALED_COMPONENT_OFFSET != 0;
                    match comp.transform {
                        None => pts.iter_mut().for_each(|p| *p = (p.0 + comp.dx, p.1 + comp.dy)),
                        Some(t) if scaled_offset => {
                            pts.iter_mut().for_each(|p| *p = (p.0 + comp.dx, p.1 + comp.dy));
                            apply_transform(&mut pts, t);
                        }
                        Some(t) => {
                            apply_transform(&mut pts, t);
                            pts.iter_mut().for_each(|p| *p = (p.0 + comp.dx, p.1 + comp.dy));
                        }
                    }
                } else {
                    if let Some(t) = comp.transform {
                        apply_transform(&mut pts, t);
                    }
                    let a = all.get(usize::from(comp.points.0)).copied();
                    let b = pts.get(usize::from(comp.points.1)).copied();
                    if let (Some(a), Some(b)) = (a, b) {
                        let (mx, my) = (a.0 - b.0, a.1 - b.1);
                        pts.iter_mut().for_each(|p| *p = (p.0 + mx, p.1 + my));
                    }
                }
                all.extend(pts);
            }
            Ok(all)
        }
        _ => Ok(Vec::new()),
    }
}

fn int_bounds(points: &[Pair]) -> Result<[i16; 4]> {
    let Some(&(x, y)) = points.first() else { return Ok([0; 4]) };
    let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
    for &(x, y) in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let c = |v: f64| coord(v).map_err(bad);
    Ok([c(x0)?, c(y0)?, c(x1)?, c(y1)?])
}

/// The bounds of glyph `g` from its current points, as `Glyph.recalcBounds`. `budget` is shared
/// by every glyph of the instance.
pub(super) fn recalc_bounds(glyphs: &[Glyph], g: usize, budget: &mut usize) -> Result<[i16; 4]> {
    match glyphs.get(g) {
        Some(Glyph::Simple(s)) => int_bounds(&s.points),
        Some(Glyph::Composite(c)) => {
            let simple = c.components.iter().all(|k| k.is_xy() && k.transform.is_none() && k.dx.fract() == 0.0 && k.dy.fract() == 0.0);
            if simple {
                let mut b: Option<[f64; 4]> = None;
                for k in &c.components {
                    let child = glyphs.get(usize::from(k.gid)).map_or([0; 4], Glyph::bbox);
                    if child[0] == child[2] && child[1] == child[3] {
                        continue;
                    }
                    let [x0, y0, x1, y1] = child.map(f64::from);
                    let (x0, y0, x1, y1) = (x0 + k.dx, y0 + k.dy, x1 + k.dx, y1 + k.dy);
                    b = Some(match b {
                        None => [x0, y0, x1, y1],
                        Some(o) => [o[0].min(x0), o[1].min(y0), o[2].max(x1), o[3].max(y1)],
                    });
                }
                let Some(b) = b else { return Ok([0; 4]) };
                int_bounds(&[(b[0], b[1]), (b[2], b[3])])
            } else {
                int_bounds(&flatten(glyphs, g, 0, budget)?)
            }
        }
        _ => Ok([0; 4]),
    }
}

/// Moves every glyph to the location `coords` (normalised, one value per `fvar` axis) by its
/// `gvar` deltas, and sets its advances and side bearings from the phantom points.
pub(super) fn apply_gvar(
    glyphs: &mut [Glyph],
    gvar: &[u8],
    axis_count: usize,
    coords: &[f64],
    hmtx: &mut Metrics,
    mut vmtx: Option<&mut Metrics>,
) -> Result<()> {
    let table = Gvar::read(FontData::new(gvar)).map_err(|e| bad(format!("the gvar table cannot be read: {e}")))?;
    if usize::from(table.axis_count()) != axis_count {
        return Err(bad(format!("the gvar table has {} axes and the fvar table {axis_count}", table.axis_count())));
    }
    let mut order: Vec<(usize, usize)> = depths(glyphs)?.into_iter().enumerate().map(|(g, d)| (d, g)).collect();
    order.sort_unstable();
    let mut budget = MAX_FLATTEN_WORK;
    for (_, g) in order {
        let Some(glyph) = glyphs.get(g) else { continue };
        let [x_min, _, _, y_max] = glyph.bbox().map(f64::from);
        let adv = f64::from(hmtx.advances.get(g).copied().unwrap_or(0));
        let lsb = f64::from(hmtx.bearings.get(g).copied().unwrap_or(0));
        let left = x_min - lsb;
        let (top, bottom) = match vmtx.as_deref() {
            Some(v) => {
                let top = f64::from(v.bearings.get(g).copied().unwrap_or(0)) + y_max;
                (top, top - f64::from(v.advances.get(g).copied().unwrap_or(0)))
            }
            None => (0.0, 0.0),
        };
        let (mut coords0, ends): (Vec<Pair>, Vec<usize>) = match glyph {
            Glyph::Simple(s) => (s.points.clone(), s.end_pts.iter().map(|e| usize::from(*e)).collect()),
            Glyph::Composite(c) => (c.components.iter().map(|k| (k.dx, k.dy)).collect(), (0..c.components.len()).collect()),
            Glyph::Empty => (Vec::new(), Vec::new()),
        };
        coords0.extend([(left, 0.0), (left + adv, 0.0), (0.0, top), (0.0, bottom)]);
        let deltas = glyph_deltas(&table, g, axis_count, coords, &coords0, &ends)?;
        let moved: Vec<Pair> = coords0.iter().zip(&deltas).map(|(p, d)| (p.0 + d.0, p.1 + d.1)).collect();
        let n = moved.len();
        let phantom = |k: usize| moved.get(n - k).copied().unwrap_or((0.0, 0.0));
        let (left, right, top, bottom) = (phantom(4).0, phantom(3).0, phantom(2).1, phantom(1).1);
        match glyphs.get_mut(g) {
            Some(Glyph::Simple(s)) => {
                for (p, m) in s.points.iter_mut().zip(&moved) {
                    *p = (ot_round(m.0), ot_round(m.1));
                }
            }
            Some(Glyph::Composite(c)) => {
                for (k, m) in c.components.iter_mut().zip(&moved) {
                    if k.is_xy() {
                        (k.dx, k.dy) = *m;
                    }
                }
            }
            _ => {}
        }
        let bbox = recalc_bounds(glyphs, g, &mut budget)?;
        match glyphs.get_mut(g) {
            Some(Glyph::Simple(s)) => s.bbox = bbox,
            Some(Glyph::Composite(c)) => c.bbox = bbox,
            _ => {}
        }
        let [x_min, _, _, y_max] = bbox.map(f64::from);
        if let (Some(a), Some(b)) = (hmtx.advances.get_mut(g), hmtx.bearings.get_mut(g)) {
            *a = ot_round(right - left).clamp(0.0, 65535.0) as u16;
            *b = clamp_i16(ot_round(x_min - left));
        }
        if let Some(v) = vmtx.as_deref_mut()
            && let (Some(a), Some(b)) = (v.advances.get_mut(g), v.bearings.get_mut(g))
        {
            *a = ot_round(top - bottom).clamp(0.0, 65535.0) as u16;
            *b = clamp_i16(ot_round(top - y_max));
        }
    }
    Ok(())
}

fn clamp_i16(v: f64) -> i16 {
    v.clamp(-32768.0, 32767.0) as i16
}

/// The summed, scaled deltas of glyph `g` for every point of `coords0` (phantom points last).
fn glyph_deltas(table: &Gvar<'_>, g: usize, axis_count: usize, coords: &[f64], coords0: &[Pair], ends: &[usize]) -> Result<Vec<Pair>> {
    let n = coords0.len();
    let mut total = vec![(0.0, 0.0); n];
    let gid = GlyphId::new(u32::try_from(g).unwrap_or(u32::MAX));
    let Some(data) = table.glyph_variation_data(gid).map_err(|e| bad(format!("the gvar data of glyph {g} cannot be read: {e}")))? else {
        return Ok(total);
    };
    for (count, tuple) in data.tuples().enumerate() {
        if count >= MAX_TUPLE_VARIATIONS {
            return Err(TablesError::TooLarge { what: "the tuple variations of one glyph", limit: MAX_TUPLE_VARIATIONS });
        }
        let peak = tuple.peak();
        let start = tuple.intermediate_start();
        let end = tuple.intermediate_end();
        let region = (0..axis_count).map(|i| {
            let p = peak.get(i).map_or(0.0, |v| f64::from(v.to_bits()) / 16384.0);
            let s = start.as_ref().and_then(|t| t.get(i)).map_or(p.min(0.0), |v| f64::from(v.to_bits()) / 16384.0);
            let e = end.as_ref().and_then(|t| t.get(i)).map_or(p.max(0.0), |v| f64::from(v.to_bits()) / 16384.0);
            (s, p, e)
        });
        let scalar = tuple_scalar(coords, region);
        if scalar == 0.0 {
            continue;
        }
        let mut explicit: Vec<Option<Pair>> = vec![None; n];
        for d in tuple.deltas() {
            if let Some(slot) = explicit.get_mut(usize::from(d.position)) {
                *slot = Some((f64::from(d.x_delta), f64::from(d.y_delta)));
            }
        }
        let full: Vec<Pair> = if explicit.iter().all(Option::is_some) {
            explicit.into_iter().map(|d| d.unwrap_or((0.0, 0.0))).collect()
        } else {
            iup_delta(&explicit, coords0, ends)
        };
        for (t, d) in total.iter_mut().zip(full) {
            t.0 += scalar * d.0;
            t.1 += scalar * d.1;
        }
    }
    Ok(total)
}

/// Sets the overlap flag on the first point of every simple glyph with contours and on the first
/// component of every composite.
pub(super) fn set_overlap_flags(glyphs: &mut [Glyph]) {
    for g in glyphs {
        match g {
            Glyph::Simple(s) => {
                if let Some(f) = s.flags.first_mut() {
                    *f |= FLAG_OVERLAP_SIMPLE;
                }
            }
            Glyph::Composite(c) => {
                if let Some(k) = c.components.first_mut() {
                    k.flags |= OVERLAP_COMPOUND;
                }
            }
            Glyph::Empty => {}
        }
    }
}

/// Whether a glyph has an outline that counts for font-wide bounds (fontTools skips glyphs with
/// no contours).
pub(super) fn has_outline(g: &Glyph) -> bool {
    match g {
        Glyph::Empty => false,
        Glyph::Simple(s) => !s.end_pts.is_empty(),
        Glyph::Composite(_) => true,
    }
}
