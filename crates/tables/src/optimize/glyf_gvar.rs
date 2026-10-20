//! The `glyf` and `gvar` steps of `font.optimize`: implied on-curve points and `gvar` delta
//! optimisation (IUP). Ports of fontTools' `ttLib.tables._g_l_y_f.dropImpliedOnCurvePoints` and
//! of `TupleVariation.calcInferredDeltas`, `roundDeltas` and `optimize` as the instancer runs
//! them (MIT, Copyright 2013 Google, Inc. and the fontTools authors). The optimiser itself is
//! `write-fonts`' port of `varLib.iup.iup_delta_optimize`.
//!
//! Every glyph's tuples are first expanded to a delta per point (inferred deltas filled in by
//! IUP) and rounded. Then:
//!
//! - An on-curve point of a simple glyph is dropped when it lies halfway between its off-curve
//!   neighbours (before or after rounding) in the default outline and in the outline each tuple
//!   moves it to, and both neighbours have the same flags. A glyph with instructions keeps its
//!   points, since the instructions number them.
//! - With [`DeltaMode::Optimize`] each tuple keeps only the deltas IUP cannot infer within half a
//!   unit; a tuple that moves nothing is dropped. [`DeltaMode::All`] writes every delta;
//!   [`DeltaMode::Keep`] keeps the points each tuple listed, except in a glyph that lost points.
//!
//! The optimiser is quadratic in the points of a contour: contours past
//! [`MAX_IUP_CONTOUR_POINTS`], and glyphs met after [`MAX_IUP_WORK`] is spent, keep their points.

use fontcraft_geom::{Point, Vec2};
use read_fonts::tables::gvar::Gvar as ReadGvar;
use read_fonts::types::{F2Dot14, GlyphId};
use read_fonts::{FontData, FontRead};
use write_fonts::tables::gvar::iup::iup_delta_optimize;
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar as WriteGvar};
use write_fonts::tables::variations::Tent;

use crate::instance::iup::{Pair, iup_delta};
use crate::limits::{MAX_SFNT, MAX_TUPLE_VARIATIONS};
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// Most points one contour may have for its deltas to be optimised.
pub const MAX_IUP_CONTOUR_POINTS: usize = 4_096;
/// Most optimiser work (the squared point counts of the contours of every tuple) in one font.
pub const MAX_IUP_WORK: usize = 1 << 30;
/// Most deltas held at once (every tuple of every glyph, expanded to all its points).
pub const MAX_HELD_DELTAS: usize = 1 << 25;
/// fontTools' tolerance: an inferred delta within half a unit of the real one is left out.
const IUP_TOLERANCE: f64 = 0.5;

const ON: u8 = 0x01;
const REPEAT: u8 = 0x08;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const X_SAME: u8 = 0x10;
const Y_SAME: u8 = 0x20;
/// The flag bits that belong to a point rather than to its encoding (on-curve, overlap, cubic).
const KEPT_FLAGS: u8 = 0x01 | 0x40 | 0x80;

/// What happens to the explicit points of each `gvar` tuple.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaMode {
    /// The points each tuple lists, unless its glyph lost points (then every point).
    Keep,
    /// Only the points IUP cannot infer.
    Optimize,
    /// Every point.
    All,
}

/// The new tables, and what was done.
#[derive(Debug, Default)]
pub struct GlyfGvarResult {
    /// `glyf`, `loca`, `head` and `maxp` when points were dropped; `gvar` when the font has one.
    pub tables: Vec<(Tag, Vec<u8>)>,
    pub dropped_points: usize,
    /// Glyphs that kept points because of their instructions.
    pub hinted_kept: usize,
    /// Glyphs whose deltas were not optimised because of the work limits.
    pub iup_skipped: usize,
}

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// A simple glyph: its points with their kept flags, contour ends and instructions.
#[derive(Clone, Debug, PartialEq)]
struct Simple {
    bbox: [u8; 8],
    ends: Vec<usize>,
    instructions: Vec<u8>,
    flags: Vec<u8>,
    points: Vec<(i32, i32)>,
}

enum Shape {
    Empty,
    Simple(Simple),
    /// The glyph ids of the components.
    Composite(Vec<u16>),
}

fn parse_glyph(b: &[u8]) -> std::result::Result<Shape, String> {
    if b.is_empty() {
        return Ok(Shape::Empty);
    }
    let contours = u16_at(b, 0).ok_or("truncated header")? as i16;
    if contours < 0 {
        return components(b).map(Shape::Composite);
    }
    parse_simple(b, contours as usize).map(Shape::Simple)
}

fn components(b: &[u8]) -> std::result::Result<Vec<u16>, String> {
    let mut at = 10usize;
    let mut out = Vec::new();
    loop {
        let flags = u16_at(b, at).ok_or("truncated component")?;
        out.push(u16_at(b, at + 2).ok_or("truncated component")?);
        if out.len() > usize::from(u16::MAX) {
            return Err("too many components".into());
        }
        at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
        at += if flags & 0x0008 != 0 {
            2
        } else if flags & 0x0040 != 0 {
            4
        } else if flags & 0x0080 != 0 {
            8
        } else {
            0
        };
        if flags & 0x0020 == 0 {
            return Ok(out);
        }
    }
}

fn parse_simple(b: &[u8], contours: usize) -> std::result::Result<Simple, String> {
    let mut bbox = [0u8; 8];
    bbox.copy_from_slice(b.get(2..10).ok_or("truncated header")?);
    let mut ends = Vec::with_capacity(contours);
    for c in 0..contours {
        let e = usize::from(u16_at(b, 10 + 2 * c).ok_or("truncated contour ends")?);
        if ends.last().is_some_and(|last| e <= *last) {
            return Err("contour ends that do not increase".into());
        }
        ends.push(e);
    }
    let n = ends.last().map_or(0, |e| e + 1);
    let at = 10 + 2 * contours;
    let ilen = usize::from(u16_at(b, at).ok_or("truncated instructions")?);
    let instructions = b.get(at + 2..at + 2 + ilen).ok_or("truncated instructions")?.to_vec();
    let mut at = at + 2 + ilen;
    let mut raw = Vec::with_capacity(n);
    while raw.len() < n {
        let f = *b.get(at).ok_or("truncated flags")?;
        at += 1;
        let mut count = 1usize;
        if f & REPEAT != 0 {
            count += usize::from(*b.get(at).ok_or("truncated flags")?);
            at += 1;
        }
        raw.extend(std::iter::repeat_n(f, count.min(n - raw.len())));
    }
    let mut points = vec![(0i32, 0i32); n];
    for (axis, short, same) in [(0usize, X_SHORT, X_SAME), (1, Y_SHORT, Y_SAME)] {
        let mut v = 0i32;
        for (f, p) in raw.iter().zip(points.iter_mut()) {
            if f & short != 0 {
                let d = i32::from(*b.get(at).ok_or("truncated coordinates")?);
                at += 1;
                v += if f & same != 0 { d } else { -d };
            } else if f & same == 0 {
                v += i32::from(u16_at(b, at).ok_or("truncated coordinates")? as i16);
                at += 2;
            }
            if axis == 0 {
                p.0 = v;
            } else {
                p.1 = v;
            }
        }
    }
    let flags = raw.iter().map(|f| f & KEPT_FLAGS).collect();
    Ok(Simple { bbox, ends, instructions, flags, points })
}

fn encode_simple(s: &Simple) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let contours = u16::try_from(s.ends.len()).map_err(|_| "too many contours")?;
    out.extend_from_slice(&contours.to_be_bytes());
    out.extend_from_slice(&s.bbox);
    for e in &s.ends {
        out.extend_from_slice(&u16::try_from(*e).map_err(|_| "too many points")?.to_be_bytes());
    }
    out.extend_from_slice(&u16::try_from(s.instructions.len()).map_err(|_| "instructions too long")?.to_be_bytes());
    out.extend_from_slice(&s.instructions);
    let mut flags = Vec::with_capacity(s.points.len());
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    let mut last = (0i32, 0i32);
    for (p, f) in s.points.iter().zip(&s.flags) {
        let mut flag = *f;
        for (d, short, same, bytes) in [(p.0 - last.0, X_SHORT, X_SAME, &mut xs), (p.1 - last.1, Y_SHORT, Y_SAME, &mut ys)] {
            if d == 0 {
                flag |= same;
            } else if (-255..=255).contains(&d) {
                flag |= short | if d > 0 { same } else { 0 };
                bytes.push(d.unsigned_abs() as u8);
            } else {
                let d = i16::try_from(d).map_err(|_| "a coordinate step beyond the 16-bit range")?;
                bytes.extend_from_slice(&d.to_be_bytes());
            }
        }
        flags.push(flag);
        last = *p;
    }
    let mut i = 0usize;
    while let Some(&f) = flags.get(i) {
        let run = flags.get(i..).map_or(1, |rest| rest.iter().take(256).take_while(|g| **g == f).count());
        if run > 1 {
            out.push(f | REPEAT);
            out.push((run - 1) as u8);
        } else {
            out.push(f);
        }
        i += run.max(1);
    }
    out.extend(xs);
    out.extend(ys);
    Ok(out)
}

/// fontTools' `_is_mid_point`: `p1` halfway between `p0` and `p2`, before or after rounding.
fn is_mid_point(p0: Pair, p1: Pair, p2: Pair) -> bool {
    let close = |a: f64, b: f64| a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs());
    let m = ((p0.0 + p2.0) * 0.5, (p0.1 + p2.1) * 0.5);
    (close(m.0, p1.0) && close(m.1, p1.1))
        || (ot_round(p0.0) + ot_round(p2.0) == ot_round(p1.0) * 2.0 && ot_round(p0.1) + ot_round(p2.1) == ot_round(p1.1) * 2.0)
}

/// One tuple of a glyph: its tents, which points it listed, and every point's delta.
struct TupleIn {
    tents: Vec<Tent>,
    explicit: Vec<bool>,
    full: Vec<Pair>,
}

/// The on-curve points of `s` that every location implies.
fn implied_points(s: &Simple, tuples: &[TupleIn]) -> Vec<bool> {
    let n = s.points.len();
    let mut drop = vec![false; n];
    let point = |k: usize, delta: Option<&[Pair]>| {
        let (x, y) = s.points.get(k).copied().unwrap_or((0, 0));
        let (dx, dy) = delta.and_then(|d| d.get(k)).copied().unwrap_or((0.0, 0.0));
        (f64::from(x) + dx, f64::from(y) + dy)
    };
    let flag = |k: usize| s.flags.get(k).copied().unwrap_or(0);
    let mut start = 0usize;
    for &last in &s.ends {
        for i in start..=last {
            if flag(i) & ON == 0 {
                continue;
            }
            let prv = if i > start { i - 1 } else { last };
            let nxt = if i < last { i + 1 } else { start };
            if flag(prv) & ON != 0 || flag(prv) != flag(nxt) {
                continue;
            }
            let at = |d: Option<&[Pair]>| is_mid_point(point(prv, d), point(i, d), point(nxt, d));
            if at(None)
                && tuples.iter().all(|t| at(Some(&t.full)))
                && let Some(slot) = drop.get_mut(i)
            {
                *slot = true;
            }
        }
        start = last + 1;
    }
    drop
}

/// Removes the `drop` points from `s` and from every tuple.
fn apply_drop(s: &mut Simple, tuples: &mut [TupleIn], drop: &[bool]) {
    let mut removed = 0usize;
    let mut k = 0usize;
    let mut ends = Vec::with_capacity(s.ends.len());
    for &e in &s.ends {
        while k <= e {
            removed += usize::from(drop.get(k).copied().unwrap_or(false));
            k += 1;
        }
        // Off-curve neighbours stay, so every contour keeps points.
        ends.push(e.saturating_sub(removed));
    }
    s.ends = ends;
    retain_kept(&mut s.points, drop);
    retain_kept(&mut s.flags, drop);
    for t in tuples {
        // Phantom points come after the outline points and are never dropped.
        retain_kept(&mut t.full, drop);
        retain_kept(&mut t.explicit, drop);
    }
}

fn retain_kept<T>(v: &mut Vec<T>, drop: &[bool]) {
    let mut i = 0usize;
    v.retain(|_| {
        i += 1;
        !drop.get(i - 1).copied().unwrap_or(false)
    });
}

/// The squared point counts of the contours (each phantom point and component is a contour).
fn iup_cost(ends: &[usize], n: usize) -> Option<usize> {
    let mut start = 0usize;
    let mut cost = 0usize;
    for &e in ends {
        let len = e + 1 - start;
        if len > MAX_IUP_CONTOUR_POINTS {
            return None;
        }
        cost = cost.saturating_add(len * len);
        start = e + 1;
    }
    Some(cost.saturating_add(n.saturating_sub(start)))
}

/// The glyph data of `font`, one slice per glyph.
fn glyph_slices(font: &SfntFont, count: usize) -> Result<Vec<&[u8]>> {
    let head = font.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?;
    let long = u16_at(head, 50).ok_or_else(|| bad("the head table is truncated"))? != 0;
    let loca = font.get(*b"loca").ok_or_else(|| TablesError::MissingTable("loca".into()))?;
    let glyf = font.get(*b"glyf").ok_or_else(|| TablesError::MissingTable("glyf".into()))?;
    let offset = |i: usize| -> Option<usize> {
        if long {
            let b = loca.get(4 * i..4 * i + 4)?;
            usize::try_from(u32::from_be_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?])).ok()
        } else {
            u16_at(loca, 2 * i).map(|v| 2 * usize::from(v))
        }
    };
    let mut out = Vec::with_capacity(count);
    for g in 0..count {
        let (Some(a), Some(b)) = (offset(g), offset(g + 1)) else { return Err(bad("the loca table is shorter than the glyph count")) };
        // As fontTools reads it: data past the table's end is cut off.
        let b = b.min(glyf.len());
        out.push(glyf.get(a.min(b)..b).unwrap_or(&[]));
    }
    Ok(out)
}

/// `glyf` and `loca` for `glyphs`, with `head.indexToLocFormat`, as fontTools compiles them:
/// unpadded, but odd glyphs padded when that allows short offsets.
fn build_glyf(glyphs: &[Vec<u8>]) -> Result<(Vec<u8>, Vec<u8>, bool)> {
    let total: usize = glyphs.iter().map(Vec::len).sum();
    let odd = glyphs.iter().filter(|g| g.len() % 2 == 1).count();
    let short = total + odd < 0x20000;
    let mut glyf = Vec::with_capacity(total + odd);
    let mut loca = Vec::with_capacity(4 * (glyphs.len() + 1));
    let put = |at: usize, loca: &mut Vec<u8>| {
        if short {
            loca.extend_from_slice(&((at / 2) as u16).to_be_bytes());
        } else {
            loca.extend_from_slice(&(at as u32).to_be_bytes());
        }
    };
    for g in glyphs {
        put(glyf.len(), &mut loca);
        glyf.extend_from_slice(g);
        if short && g.len() % 2 == 1 {
            glyf.push(0);
        }
    }
    put(glyf.len(), &mut loca);
    if glyf.len() > MAX_SFNT {
        return Err(TablesError::TooLarge { what: "the glyf table", limit: MAX_SFNT });
    }
    Ok((glyf, loca, !short))
}

/// `maxPoints` and `maxCompositePoints` worked out again, as fontTools' `maxp.recalc` does.
/// `shapes` gives each glyph's outline points and components.
fn recalc_maxp(maxp: &[u8], shapes: &[(usize, Vec<u16>)]) -> Vec<u8> {
    let mut out = maxp.to_vec();
    if u16_at(maxp, 0) != Some(1) || maxp.len() < 32 {
        return out;
    }
    let mut memo: Vec<Option<usize>> = vec![None; shapes.len()];
    let mut busy = vec![false; shapes.len()];
    let mut max_simple = 0usize;
    let mut max_composite = 0usize;
    for (g, (points, comps)) in shapes.iter().enumerate() {
        if comps.is_empty() {
            max_simple = max_simple.max(*points);
        } else {
            max_composite = max_composite.max(flat_points(g, shapes, &mut memo, &mut busy, 0));
        }
    }
    let clamp = |v: usize| v.min(usize::from(u16::MAX)) as u16;
    out.splice(6..8, clamp(max_simple).to_be_bytes());
    out.splice(10..12, clamp(max_composite).to_be_bytes());
    out
}

/// Deepest component nesting followed when counting points.
const MAX_COMPONENT_DEPTH: usize = 64;

/// The points of glyph `g` with its components flattened; a cycle, a missing glyph or nesting
/// past [`MAX_COMPONENT_DEPTH`] counts nothing.
fn flat_points(g: usize, shapes: &[(usize, Vec<u16>)], memo: &mut [Option<usize>], busy: &mut [bool], depth: usize) -> usize {
    if let Some(v) = memo.get(g).copied().flatten() {
        return v;
    }
    let Some((points, comps)) = shapes.get(g) else { return 0 };
    if comps.is_empty() {
        return *points;
    }
    if depth >= MAX_COMPONENT_DEPTH || busy.get(g).copied().unwrap_or(true) {
        return 0;
    }
    if let Some(b) = busy.get_mut(g) {
        *b = true;
    }
    let mut sum = 0usize;
    for c in comps {
        sum = sum.saturating_add(flat_points(usize::from(*c), shapes, memo, busy, depth + 1));
    }
    if let Some(b) = busy.get_mut(g) {
        *b = false;
    }
    if let Some(m) = memo.get_mut(g) {
        *m = Some(sum);
    }
    sum
}

/// Drops implied on-curve points (`drop_implied`) and rewrites `gvar` by `mode`.
pub fn run(font: &SfntFont, drop_implied: bool, mode: DeltaMode) -> Result<GlyfGvarResult> {
    let maxp = font.get(*b"maxp").ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    let count = usize::from(u16_at(maxp, 4).ok_or_else(|| bad("the maxp table is truncated"))?);
    let slices = glyph_slices(font, count)?;
    let gvar_bytes = font.get(*b"gvar");
    let gvar = match gvar_bytes {
        Some(b) => Some(ReadGvar::read(FontData::new(b)).map_err(|e| bad(format!("the gvar table cannot be read: {e}")))?),
        None => None,
    };
    let axis_count = gvar.as_ref().map_or(0, ReadGvar::axis_count);
    if let Some(g) = &gvar
        && usize::from(g.glyph_count()) != count
    {
        return Err(bad(format!("the gvar table has {} glyphs and maxp {count}", g.glyph_count())));
    }
    let mut result = GlyfGvarResult::default();
    let mut new_glyphs: Vec<Option<Vec<u8>>> = vec![None; count];
    let mut shapes: Vec<(usize, Vec<u16>)> = Vec::with_capacity(count);
    let mut variations = Vec::with_capacity(if gvar.is_some() { count } else { 0 });
    let mut held = 0usize;
    let mut work_left = MAX_IUP_WORK;
    for (g, bytes) in slices.iter().enumerate() {
        let shape = parse_glyph(bytes).map_err(|e| bad(format!("glyph {g} cannot be read: {e}")))?;
        let (mut simple, comps) = match shape {
            Shape::Empty => (None, Vec::new()),
            Shape::Simple(s) => (Some(s), Vec::new()),
            Shape::Composite(c) => (None, c),
        };
        let outline_points = simple.as_ref().map_or(comps.len(), |s| s.points.len());
        let n = outline_points + 4;
        let mut tuples = Vec::new();
        if let Some(table) = &gvar {
            let gid = GlyphId::new(g as u32);
            let data = table.glyph_variation_data(gid).map_err(|e| bad(format!("the gvar data of glyph {g} cannot be read: {e}")))?;
            let mut coords: Vec<Pair> = match &simple {
                Some(s) => s.points.iter().map(|&(x, y)| (f64::from(x), f64::from(y))).collect(),
                None => vec![(0.0, 0.0); comps.len()],
            };
            coords.extend([(0.0, 0.0); 4]);
            let ends: Vec<usize> = match &simple {
                Some(s) => s.ends.clone(),
                None => (0..comps.len()).collect(),
            };
            for (k, tuple) in data.iter().flat_map(|d| d.tuples()).enumerate() {
                if k >= MAX_TUPLE_VARIATIONS {
                    return Err(TablesError::TooLarge { what: "the tuple variations of one glyph", limit: MAX_TUPLE_VARIATIONS });
                }
                held = held.saturating_add(n);
                if held > MAX_HELD_DELTAS {
                    return Err(TablesError::TooLarge { what: "the gvar deltas", limit: MAX_HELD_DELTAS });
                }
                let peak = tuple.peak();
                let (start, end) = (tuple.intermediate_start(), tuple.intermediate_end());
                let tents = (0..usize::from(axis_count))
                    .map(|i| {
                        let p = peak.get(i).unwrap_or(F2Dot14::ZERO);
                        match (start.as_ref().and_then(|t| t.get(i)), end.as_ref().and_then(|t| t.get(i))) {
                            (Some(s), Some(e)) => Tent::new(p, Some((s, e))),
                            _ => Tent::new(p, None),
                        }
                    })
                    .collect();
                let mut explicit: Vec<Option<Pair>> = vec![None; n];
                for d in tuple.deltas() {
                    let slot = explicit
                        .get_mut(usize::from(d.position))
                        .ok_or_else(|| bad(format!("glyph {g} has a delta for point {}, past its points", d.position)))?;
                    *slot = Some((f64::from(d.x_delta), f64::from(d.y_delta)));
                }
                let listed: Vec<bool> = explicit.iter().map(Option::is_some).collect();
                let full = if listed.iter().all(|e| *e) {
                    explicit.iter().map(|d| d.unwrap_or((0.0, 0.0))).collect()
                } else {
                    iup_delta(&explicit, &coords, &ends)
                };
                tuples.push(TupleIn { tents, explicit: listed, full });
            }
        }
        let mut changed = false;
        if drop_implied && let Some(s) = simple.as_mut() {
            let drop = implied_points(s, &tuples);
            let dropped = drop.iter().filter(|d| **d).count();
            if dropped > 0 {
                if s.instructions.is_empty() {
                    apply_drop(s, &mut tuples, &drop);
                    let encoded = encode_simple(s).map_err(|e| bad(format!("glyph {g} cannot be written: {e}")))?;
                    if let Some(slot) = new_glyphs.get_mut(g) {
                        *slot = Some(encoded);
                    }
                    result.dropped_points += dropped;
                    changed = true;
                } else {
                    result.hinted_kept += 1;
                }
            }
        }
        shapes.push((simple.as_ref().map_or(0, |s| s.points.len()), comps.clone()));
        if gvar.is_none() {
            continue;
        }
        // Phantom points and component offsets are contours of one point: where they lie does
        // not change what IUP infers.
        let mut coords: Vec<Point> = match &simple {
            Some(s) => s.points.iter().map(|&(x, y)| Point::new(f64::from(x), f64::from(y))).collect(),
            None => vec![Point::new(0.0, 0.0); comps.len()],
        };
        coords.extend([Point::new(0.0, 0.0); 4]);
        let ends: Vec<usize> = match &simple {
            Some(s) => s.ends.clone(),
            None => (0..comps.len()).collect(),
        };
        let cost = iup_cost(&ends, coords.len()).map(|c| c.saturating_mul(tuples.len()));
        let mut optimise = mode == DeltaMode::Optimize;
        if optimise {
            match cost.and_then(|c| work_left.checked_sub(c)) {
                Some(left) => work_left = left,
                None => {
                    optimise = false;
                    result.iup_skipped += 1;
                }
            }
        }
        let mut out = Vec::with_capacity(tuples.len());
        for t in tuples {
            let mut rounded = Vec::with_capacity(t.full.len());
            for &(x, y) in &t.full {
                let (x, y) = (ot_round(x), ot_round(y));
                if !(-32768.0..=32767.0).contains(&x) || !(-32768.0..=32767.0).contains(&y) {
                    return Err(bad(format!("glyph {g} has a delta outside −32,768 to 32,767")));
                }
                rounded.push((x as i16, y as i16));
            }
            let deltas: Vec<GlyphDelta> = if optimise {
                if rounded.iter().all(|d| *d == (0, 0)) {
                    continue;
                }
                let vecs: Vec<Vec2> = rounded.iter().map(|&(x, y)| Vec2::new(f64::from(x), f64::from(y))).collect();
                iup_delta_optimize(vecs, coords.clone(), IUP_TOLERANCE, &ends)
                    .map_err(|e| bad(format!("the deltas of glyph {g} cannot be optimised: {e:?}")))?
            } else {
                // A glyph that lost points lists every delta: IUP over the remaining points
                // would infer others than before.
                let every = mode == DeltaMode::All || changed;
                rounded.iter().zip(&t.explicit).map(|(&(x, y), e)| GlyphDelta::new(x, y, *e || every)).collect()
            };
            out.push(GlyphDeltas::new(t.tents, deltas));
        }
        variations.push(GlyphVariations::new(GlyphId::new(g as u32), out));
    }
    if result.dropped_points > 0 {
        let glyphs: Vec<Vec<u8>> = new_glyphs.into_iter().zip(&slices).map(|(new, old)| new.unwrap_or_else(|| old.to_vec())).collect();
        let (glyf, loca, long) = build_glyf(&glyphs)?;
        let mut head = font.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?.to_vec();
        if head.len() < 54 {
            return Err(bad("the head table is truncated"));
        }
        head.splice(50..52, u16::from(long).to_be_bytes());
        result.tables.push((*b"glyf", glyf));
        result.tables.push((*b"loca", loca));
        result.tables.push((*b"head", head));
        result.tables.push((*b"maxp", recalc_maxp(maxp, &shapes)));
    }
    if gvar.is_some() {
        let table = WriteGvar::new(variations, axis_count).map_err(|e| bad(format!("the gvar table cannot be built: {e:?}")))?;
        let bytes = write_fonts::dump_table(&table).map_err(|e| bad(format!("the gvar table could not be written: {e}")))?;
        if bytes.len() > MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the gvar table", limit: MAX_SFNT });
        }
        result.tables.push((*b"gvar", bytes));
    }
    Ok(result)
}

#[cfg(test)]
#[path = "glyf_gvar_tests.rs"]
mod tests;
