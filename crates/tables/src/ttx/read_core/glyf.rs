//! `glyf` and `loca` from `TTGlyph` elements, as fontTools' `table__g_l_y_f.compile` writes
//! them: glyph bounds recalculated (unless bounds are kept), point flags compressed greedily,
//! odd-length glyphs padded when that keeps `loca` short; and `maxp.recalc`.

use std::collections::{HashMap, HashSet};

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

use super::super::Element;
use super::super::read::{Num, Out, ReadContext, attr, float_to_fixed, font_metrics, get_u16, int_lit, num_lit, ot_round, put_i16, put_u16};
use super::program_of;

const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME: u8 = 0x10;
const Y_SAME: u8 = 0x20;
const OVERLAP_SIMPLE: u8 = 0x40;
const CUBIC: u8 = 0x80;

const ARG_1_AND_2_ARE_WORDS: i64 = 0x0001;
const ARGS_ARE_XY_VALUES: i64 = 0x0002;
const WE_HAVE_A_SCALE: i64 = 0x0008;
const MORE_COMPONENTS: i64 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: i64 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: i64 = 0x0080;
const WE_HAVE_INSTRUCTIONS: i64 = 0x0100;
const SCALED_COMPONENT_OFFSET: i64 = 0x0800;
const UNSCALED_COMPONENT_OFFSET: i64 = 0x1000;
/// ROUND_XY_TO_GRID, NON_OVERLAPPING, USE_MY_METRICS, OVERLAP_COMPOUND and the two offset flags.
const KEPT_FLAGS: i64 = 0x0004 | 0x0010 | 0x0200 | 0x0400 | SCALED_COMPONENT_OFFSET | UNSCALED_COMPONENT_OFFSET;

/// Deepest component nesting followed; fontTools stops only at Python's recursion limit.
const MAX_COMPONENT_DEPTH: usize = 64;
/// Most work done in all flattening composite glyphs to compute their bounds in one table: one
/// unit per component visited (empty glyphs included) and one per point copied.
const MAX_FLATTEN_WORK: usize = 20_000_000;
/// Most component references followed in all by `maxp_recalc`.
const MAX_COMPONENT_VISITS: usize = 20_000_000;

struct Component {
    glyph: String,
    x: Num,
    y: Num,
    anchor: Option<(i64, i64)>,
    transform: Option<[[f64; 2]; 2]>,
    flags: i64,
}

#[derive(Default)]
struct Glyph {
    /// fontTools' `numberOfContours`: contours, -1 for a composite, 0 for an empty glyph.
    contours: i32,
    coords: Vec<(f64, f64)>,
    flags: Vec<u8>,
    end_points: Vec<i64>,
    components: Vec<Component>,
    program: Option<Vec<u8>>,
    bounds: [i64; 4],
}

/// What [`super::super::import`] needs of a glyph for `maxp` and `hhea`.
pub(crate) struct GlyphStat {
    pub contours: i32,
    pub bounds: [i64; 4],
    pub points: usize,
    pub components: Vec<usize>,
}

/// The compiled `glyf`, its `loca` offsets and what each glyph holds, in glyph order.
pub(crate) struct Glyf {
    pub data: Vec<u8>,
    pub locations: Vec<u32>,
    pub glyphs: Vec<GlyphStat>,
}

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what.into())
}

fn glyph_from(element: &Element) -> Result<Glyph> {
    let mut g = Glyph::default();
    for (slot, name) in g.bounds.iter_mut().zip(["xMin", "yMin", "xMax", "yMax"]) {
        *slot = int_lit(element.attribute(name).unwrap_or("0"))?;
    }
    for child in &element.children {
        match child.name.as_str() {
            "contour" => {
                if g.contours < 0 {
                    return Err(bad("can't mix composites and contours in glyph"));
                }
                g.contours = g.contours.checked_add(1).ok_or_else(|| bad("too many contours"))?;
                for pt in child.children.iter().filter(|p| p.name == "pt") {
                    g.coords.push((num_lit(attr(pt, "x")?)?.as_f64(), num_lit(attr(pt, "y")?)?.as_f64()));
                    let flag_of = |name: &str| pt.attribute(name).map(|v| int_lit(v).map(|n| n != 0)).transpose();
                    let mut flag = u8::from(flag_of("on")?.ok_or_else(|| bad("a point has no 'on' attribute"))?);
                    if flag_of("overlap")?.unwrap_or(false) {
                        flag |= OVERLAP_SIMPLE;
                    }
                    if flag_of("cubic")?.unwrap_or(false) {
                        flag |= CUBIC;
                    }
                    g.flags.push(flag);
                }
                g.end_points.push(g.coords.len() as i64 - 1);
            }
            "component" => {
                if g.contours > 0 {
                    return Err(bad("can't mix composites and contours in glyph"));
                }
                g.contours = -1;
                g.components.push(component_from(child)?);
            }
            "instructions" => g.program = Some(program_of(child)?.unwrap_or_default()),
            _ => {}
        }
    }
    Ok(g)
}

/// fontTools' `str2fl(s, 14)`: the value rounded to 2.14.
fn f2dot14(text: &str) -> Result<f64> {
    Ok(float_to_fixed(num_lit(text)?.as_f64(), 14)? as f64 / 16384.0)
}

fn component_from(e: &Element) -> Result<Component> {
    let glyph = attr(e, "glyphName")?.to_string();
    let (x, y, anchor) = if e.attribute("firstPt").is_some() {
        (Num::Int(0), Num::Int(0), Some((int_lit(attr(e, "firstPt")?)?, int_lit(attr(e, "secondPt")?)?)))
    } else {
        (num_lit(attr(e, "x")?)?, num_lit(attr(e, "y")?)?, None)
    };
    let transform = if e.attribute("scale01").is_some() {
        Some([[f2dot14(attr(e, "scalex")?)?, f2dot14(attr(e, "scale01")?)?], [f2dot14(attr(e, "scale10")?)?, f2dot14(attr(e, "scaley")?)?]])
    } else if e.attribute("scalex").is_some() {
        Some([[f2dot14(attr(e, "scalex")?)?, 0.0], [0.0, f2dot14(attr(e, "scaley")?)?]])
    } else if let Some(s) = e.attribute("scale") {
        let s = f2dot14(s)?;
        Some([[s, 0.0], [0.0, s]])
    } else {
        None
    };
    Ok(Component { glyph, x, y, anchor, transform, flags: int_lit(attr(e, "flags")?)? })
}

/// Reads every `TTGlyph` and compiles the table in glyph order.
pub(crate) fn compile_glyf(element: &Element, ctx: &ReadContext) -> Result<Glyf> {
    let mut glyphs: HashMap<&str, Glyph> = HashMap::new();
    for e in element.children.iter().filter(|c| c.name == "TTGlyph") {
        glyphs.insert(attr(e, "name")?, glyph_from(e)?);
    }
    let order = ctx.order()?;
    if ctx.recalc {
        let mut done: HashSet<String> = HashSet::new();
        let mut budget = MAX_FLATTEN_WORK;
        for name in order {
            recalc_bounds(&mut glyphs, name, &mut done, 0, &mut budget)?;
        }
    }
    let mut chunks: Vec<Vec<u8>> = Vec::with_capacity(order.len());
    let mut stats = Vec::with_capacity(order.len());
    for name in order {
        let g = glyphs.get(name.as_str()).ok_or_else(|| bad(format!("the 'glyf' table has no glyph '{name}'")))?;
        chunks.push(compile_glyph(g, ctx)?);
        let components = g.components.iter().map(|c| ctx.glyph_id(&c.glyph).map(|id| id as usize)).collect::<Result<_>>()?;
        stats.push(GlyphStat { contours: g.contours, bounds: g.bounds, points: g.coords.len(), components });
    }
    let total: usize = chunks.iter().map(Vec::len).sum();
    let odd = chunks.iter().filter(|c| c.len() % 2 == 1).count();
    if total < 0x20000 && odd > 0 && total + odd < 0x20000 {
        for c in chunks.iter_mut().filter(|c| c.len() % 2 == 1) {
            c.push(0);
        }
    }
    let mut locations = Vec::with_capacity(chunks.len() + 1);
    let mut data = Vec::new();
    for c in &chunks {
        locations.push(u32::try_from(data.len()).map_err(|_| TablesError::TooLarge { what: "the 'glyf' table", limit: u32::MAX as usize })?);
        data.extend_from_slice(c);
        if data.len() > crate::limits::MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the 'glyf' table", limit: crate::limits::MAX_SFNT });
        }
    }
    locations.push(data.len() as u32);
    if data.is_empty() {
        data.push(0);
    }
    Ok(Glyf { data, locations, glyphs: stats })
}

/// `loca` from the offsets, and whether it uses long offsets.
pub(crate) fn compile_loca(locations: &[u32]) -> (Vec<u8>, bool) {
    let max = locations.iter().copied().max().unwrap_or(0);
    if max < 0x20000 && locations.iter().all(|l| l % 2 == 0) {
        (locations.iter().flat_map(|l| ((l / 2) as u16).to_be_bytes()).collect(), false)
    } else {
        (locations.iter().flat_map(|l| l.to_be_bytes()).collect(), true)
    }
}

fn int_bounds(points: &[(f64, f64)]) -> Result<[i64; 4]> {
    let Some(first) = points.first() else { return Ok([0; 4]) };
    let mut b = [first.0, first.1, first.0, first.1];
    for (x, y) in points {
        b = [b[0].min(*x), b[1].min(*y), b[2].max(*x), b[3].max(*y)];
    }
    Ok([ot_round(b[0])?, ot_round(b[1])?, ot_round(b[2])?, ot_round(b[3])?])
}

fn is_integer_translate(c: &Component) -> bool {
    let whole = |n: Num| match n {
        Num::Int(_) => true,
        Num::Float(v) => v.fract() == 0.0,
    };
    c.anchor.is_none() && c.transform.is_none() && whole(c.x) && whole(c.y)
}

fn recursion(name: &str) -> TablesError {
    bad(format!("glyph '{name}' contains a recursive component reference"))
}

/// fontTools' `Glyph.recalcBounds`, with the `boundsDone` set of the table compile. `budget` is
/// shared by every glyph of the table.
fn recalc_bounds(glyphs: &mut HashMap<&str, Glyph>, name: &str, done: &mut HashSet<String>, depth: usize, budget: &mut usize) -> Result<()> {
    if depth > MAX_COMPONENT_DEPTH {
        return Err(recursion(name));
    }
    let g = glyphs.get(name).ok_or_else(|| bad(format!("the 'glyf' table has no glyph '{name}'")))?;
    if g.contours < 0 && g.components.iter().all(is_integer_translate) {
        let parts: Vec<(String, f64, f64)> = g.components.iter().map(|c| (c.glyph.clone(), c.x.as_f64(), c.y.as_f64())).collect();
        let mut bounds: Option<[f64; 4]> = None;
        for (child, x, y) in parts {
            if !done.contains(&child) {
                recalc_bounds(glyphs, &child, done, depth + 1, budget)?;
                done.insert(child.clone());
            }
            let b = glyphs.get(child.as_str()).map(|c| c.bounds).ok_or_else(|| bad(format!("the 'glyf' table has no glyph '{child}'")))?;
            if b[1] == b[3] && b[0] == b[2] {
                continue;
            }
            let (lo, hi) = ((b[0] as f64 + x, b[1] as f64 + y), (b[2] as f64 + x, b[3] as f64 + y));
            bounds = Some(match bounds {
                None => [lo.0, lo.1, hi.0, hi.1],
                Some(a) => [a[0].min(lo.0), a[1].min(lo.1), a[2].max(hi.0), a[3].max(hi.1)],
            });
        }
        let b = bounds.unwrap_or([0.0; 4]);
        let b = [b[0] as i64, b[1] as i64, b[2] as i64, b[3] as i64];
        if let Some(g) = glyphs.get_mut(name) {
            g.bounds = b;
        }
        return Ok(());
    }
    if g.contours == 0 {
        return Ok(());
    }
    let (points, _) = coordinates(glyphs, name, 0, budget)?;
    let b = int_bounds(&points)?;
    if let Some(g) = glyphs.get_mut(name) {
        g.bounds = b;
    }
    Ok(())
}

fn apply(t: &[[f64; 2]; 2], points: &mut [(f64, f64)]) {
    for p in points {
        *p = (p.0 * t[0][0] + p.1 * t[1][0], p.0 * t[0][1] + p.1 * t[1][1]);
    }
}

fn translate(dx: f64, dy: f64, points: &mut [(f64, f64)]) {
    for p in points {
        *p = (p.0 + dx, p.1 + dy);
    }
}

/// Points and the end point of each contour.
type Coordinates = (Vec<(f64, f64)>, Vec<i64>);

/// fontTools' `Glyph.getCoordinates(round=otRound)`: the points of a glyph with its components
/// flattened, and the end point of each contour.
fn coordinates(glyphs: &HashMap<&str, Glyph>, name: &str, depth: usize, budget: &mut usize) -> Result<Coordinates> {
    if depth > MAX_COMPONENT_DEPTH {
        return Err(recursion(name));
    }
    let charge = |budget: &mut usize, n: usize| {
        *budget = budget
            .checked_sub(n)
            .ok_or(TablesError::TooLarge { what: "the component points flattened to compute composite bounds", limit: MAX_FLATTEN_WORK })?;
        Ok::<(), TablesError>(())
    };
    let g = glyphs.get(name).ok_or_else(|| bad(format!("the 'glyf' table has no glyph '{name}'")))?;
    if g.contours > 0 {
        charge(budget, g.coords.len())?;
        return Ok((g.coords.clone(), g.end_points.clone()));
    }
    let mut all: Vec<(f64, f64)> = Vec::new();
    let mut ends: Vec<i64> = Vec::new();
    for c in &g.components {
        charge(budget, 1)?;
        let child = glyphs.get(c.glyph.as_str()).ok_or_else(|| bad(format!("the 'glyf' table has no glyph '{}'", c.glyph)))?;
        let (mut pts, child_ends) = coordinates(glyphs, &c.glyph, depth + 1, budget)?;
        if child.contours > 0 {
            for p in &mut pts {
                *p = (ot_round(p.0)? as f64, ot_round(p.1)? as f64);
            }
        }
        if let Some((first, second)) = c.anchor {
            if let Some(t) = &c.transform {
                apply(t, &mut pts);
            }
            let p1 = usize::try_from(first).ok().and_then(|i| all.get(i)).copied().ok_or_else(|| bad("a component anchor point is out of range"))?;
            let p2 = usize::try_from(second).ok().and_then(|i| pts.get(i)).copied().ok_or_else(|| bad("a component anchor point is out of range"))?;
            translate(p1.0 - p2.0, p1.1 - p2.1, &mut pts);
        } else {
            let (dx, dy) = (c.x.as_f64(), c.y.as_f64());
            match &c.transform {
                None => translate(dx, dy, &mut pts),
                Some(t) => {
                    // fontTools' default (SCALE_COMPONENT_OFFSET_DEFAULT) does not scale the offset.
                    if c.flags & SCALED_COMPONENT_OFFSET != 0 {
                        translate(dx, dy, &mut pts);
                        apply(t, &mut pts);
                    } else {
                        apply(t, &mut pts);
                        translate(dx, dy, &mut pts);
                    }
                }
            }
        }
        let offset = all.len() as i64;
        ends.extend(child_ends.iter().map(|e| e + offset));
        all.extend(pts);
    }
    Ok((all, ends))
}

fn compile_glyph(g: &Glyph, ctx: &ReadContext) -> Result<Vec<u8>> {
    if g.contours == 0 {
        return Ok(Vec::new());
    }
    let mut out = Out::default();
    out.i16(i64::from(g.contours), "numberOfContours")?;
    for (v, what) in g.bounds.iter().zip(["xMin", "yMin", "xMax", "yMax"]) {
        out.i16(*v, what)?;
    }
    if g.contours < 0 {
        let last = g.components.len().saturating_sub(1);
        for (i, c) in g.components.iter().enumerate() {
            let more = i != last;
            let instructions = !more && g.program.is_some();
            compile_component(&mut out, c, more, instructions, ctx)?;
        }
        if let Some(code) = &g.program {
            out.i16(code.len() as i64, "the instruction length")?;
            out.bytes(code);
        }
        return Ok(out.0);
    }
    for e in &g.end_points {
        out.u16(*e, "a contour end point")?;
    }
    let code = g.program.clone().unwrap_or_default();
    out.i16(code.len() as i64, "the instruction length")?;
    out.bytes(&code);
    let mut flags_out: Vec<u8> = Vec::new();
    let mut xs = Out::default();
    let mut ys = Out::default();
    let mut last_flag: Option<u8> = None;
    let mut repeat = 0u8;
    let (mut px, mut py) = (0i64, 0i64);
    for ((x, y), flag) in g.coords.iter().zip(&g.flags) {
        let (ax, ay) = (ot_round(*x)?, ot_round(*y)?);
        let (dx, dy) = (ax - px, ay - py);
        (px, py) = (ax, ay);
        let mut flag = *flag;
        for (d, short, same, out) in [(dx, X_SHORT, X_SAME, &mut xs), (dy, Y_SHORT, Y_SAME, &mut ys)] {
            if d == 0 {
                flag |= same;
            } else if (-255..=255).contains(&d) {
                flag |= short;
                if d > 0 {
                    flag |= same;
                }
                out.u8(d.abs(), "a coordinate")?;
            } else {
                out.i16(d, "a coordinate delta")?;
            }
        }
        if Some(flag) == last_flag && repeat != 255 {
            repeat += 1;
            if repeat == 1 {
                flags_out.push(flag);
            } else {
                let len = flags_out.len();
                if let Some(f) = flags_out.get_mut(len.saturating_sub(2)) {
                    *f = flag | REPEAT;
                }
                if let Some(r) = flags_out.last_mut() {
                    *r = repeat;
                }
            }
        } else {
            repeat = 0;
            flags_out.push(flag);
        }
        last_flag = Some(flag);
    }
    out.bytes(&flags_out);
    out.bytes(&xs.0);
    out.bytes(&ys.0);
    Ok(out.0)
}

fn compile_component(out: &mut Out, c: &Component, more: bool, instructions: bool, ctx: &ReadContext) -> Result<()> {
    let mut flags = c.flags & KEPT_FLAGS;
    if more {
        flags |= MORE_COMPONENTS;
    }
    if instructions {
        flags |= WE_HAVE_INSTRUCTIONS;
    }
    let mut args = Out::default();
    if let Some((first, second)) = c.anchor {
        if (0..=255).contains(&first) && (0..=255).contains(&second) {
            args.u8(first, "firstPt")?;
            args.u8(second, "secondPt")?;
        } else {
            args.u16(first, "firstPt")?;
            args.u16(second, "secondPt")?;
            flags |= ARG_1_AND_2_ARE_WORDS;
        }
    } else {
        let (x, y) = (c.x.round()?, c.y.round()?);
        flags |= ARGS_ARE_XY_VALUES;
        if (-128..=127).contains(&x) && (-128..=127).contains(&y) {
            args.i8(x, "a component offset")?;
            args.i8(y, "a component offset")?;
        } else {
            args.i16(x, "a component offset")?;
            args.i16(y, "a component offset")?;
            flags |= ARG_1_AND_2_ARE_WORDS;
        }
    }
    if let Some(t) = &c.transform {
        let f = |v: f64| float_to_fixed(v, 14);
        let m = [[f(t[0][0])?, f(t[0][1])?], [f(t[1][0])?, f(t[1][1])?]];
        if m[0][1] != 0 || m[1][0] != 0 {
            flags |= WE_HAVE_A_TWO_BY_TWO;
            for v in [m[0][0], m[0][1], m[1][0], m[1][1]] {
                args.i16(v, "a component transform")?;
            }
        } else if m[0][0] != m[1][1] {
            flags |= WE_HAVE_AN_X_AND_Y_SCALE;
            args.i16(m[0][0], "a component scale")?;
            args.i16(m[1][1], "a component scale")?;
        } else {
            flags |= WE_HAVE_A_SCALE;
            args.i16(m[0][0], "a component scale")?;
        }
    }
    out.u16(flags, "component flags")?;
    out.u16(i64::from(ctx.glyph_id(&c.glyph)?), "a component glyph")?;
    out.bytes(&args.0);
    Ok(())
}

/// What `maxp.recalc` computes.
pub(crate) struct MaxpRecalc {
    pub bounds: [i64; 4],
    pub max_points: i64,
    pub max_contours: i64,
    pub max_composite_points: i64,
    pub max_composite_contours: i64,
    pub max_component_elements: i64,
    pub max_component_depth: i64,
    pub all_xmin_is_lsb: bool,
}

/// Points, contours and component depth below a composite glyph (0 when it uses no other
/// composite), memoised per glyph index.
type CompositeMemo = HashMap<usize, (i64, i64, i64)>;

/// Points, contours and depth of a composite glyph, as `getCompositeMaxpValues` counts them.
/// `memo` and `visits` are shared by every glyph of the table.
fn composite_values(glyphs: &[GlyphStat], index: usize, depth: i64, memo: &mut CompositeMemo, visits: &mut usize) -> Result<(i64, i64, i64)> {
    let too_deep = || bad("a composite glyph nests its components too deeply");
    if let Some(&(points, contours, below)) = memo.get(&index) {
        if depth.saturating_add(below) > MAX_COMPONENT_DEPTH as i64 {
            return Err(too_deep());
        }
        return Ok((points, contours, depth + below));
    }
    if depth > MAX_COMPONENT_DEPTH as i64 {
        return Err(too_deep());
    }
    let g = glyphs.get(index).ok_or_else(|| bad("a component names a glyph past the glyph order"))?;
    let (mut points, mut contours, mut max_depth) = (0i64, 0i64, depth);
    for c in &g.components {
        *visits = visits
            .checked_sub(1)
            .ok_or(TablesError::TooLarge { what: "the component references followed to recalculate 'maxp'", limit: MAX_COMPONENT_VISITS })?;
        let base = glyphs.get(*c).ok_or_else(|| bad("a component names a glyph past the glyph order"))?;
        let (p, n) = if base.contours == 0 {
            continue;
        } else if base.contours > 0 {
            (base.points as i64, i64::from(base.contours))
        } else {
            let (p, n, d) = composite_values(glyphs, *c, depth + 1, memo, visits)?;
            max_depth = max_depth.max(d);
            (p, n)
        };
        points = points.saturating_add(p);
        contours = contours.saturating_add(n);
    }
    memo.insert(index, (points, contours, max_depth - depth));
    Ok((points, contours, max_depth))
}

/// fontTools' `maxp.recalc` over the compiled glyphs and the horizontal metrics.
pub(crate) fn maxp_recalc(glyphs: &[GlyphStat], hmtx: Option<&[(i64, i64)]>) -> Result<MaxpRecalc> {
    let mut r = MaxpRecalc {
        bounds: [i64::MAX, i64::MAX, i64::MIN, i64::MIN],
        max_points: 0,
        max_contours: 0,
        max_composite_points: 0,
        max_composite_contours: 0,
        max_component_elements: 0,
        max_component_depth: 0,
        all_xmin_is_lsb: true,
    };
    let mut memo = CompositeMemo::new();
    let mut visits = MAX_COMPONENT_VISITS;
    for (i, g) in glyphs.iter().enumerate() {
        if g.contours == 0 {
            continue;
        }
        let lsb = hmtx.and_then(|m| m.get(i)).map(|m| m.1).ok_or_else(|| bad("the 'hmtx' table is needed to recalculate 'maxp'"))?;
        if lsb != g.bounds[0] {
            r.all_xmin_is_lsb = false;
        }
        r.bounds = [r.bounds[0].min(g.bounds[0]), r.bounds[1].min(g.bounds[1]), r.bounds[2].max(g.bounds[2]), r.bounds[3].max(g.bounds[3])];
        if g.contours > 0 {
            r.max_points = r.max_points.max(g.points as i64);
            r.max_contours = r.max_contours.max(i64::from(g.contours));
        } else {
            let (p, n, d) = composite_values(glyphs, i, 1, &mut memo, &mut visits)?;
            r.max_composite_points = r.max_composite_points.max(p);
            r.max_composite_contours = r.max_composite_contours.max(n);
            r.max_component_elements = r.max_component_elements.max(g.components.len() as i64);
            r.max_component_depth = r.max_component_depth.max(d);
        }
    }
    if r.bounds[0] == i64::MAX {
        r.bounds = [0; 4];
    }
    Ok(r)
}

/// Writes `r` into the font as fontTools' `maxp.recalc` does: the six maxima of a version 1.0
/// `maxp`, and the `head` bounds and `flags` bit 1 (set when every glyph with outlines has its
/// left side bearing at its `xMin`, cleared otherwise). Says whether the font has a `head`.
pub(crate) fn apply_maxp_recalc(font: &mut SfntFont, r: &MaxpRecalc) -> Result<bool> {
    if font.get(*b"maxp").is_some_and(|d| d.len() >= 32) {
        for (at, v) in [
            (6, r.max_points),
            (8, r.max_contours),
            (10, r.max_composite_points),
            (12, r.max_composite_contours),
            (28, r.max_component_elements),
            (30, r.max_component_depth),
        ] {
            put_u16(font, *b"maxp", at, v, "a maxp maximum")?;
        }
    }
    if font.get(*b"head").is_none() {
        return Ok(false);
    }
    for (at, v) in [(36, r.bounds[0]), (38, r.bounds[1]), (40, r.bounds[2]), (42, r.bounds[3])] {
        put_i16(font, *b"head", at, v, "the font bounding box")?;
    }
    let flags = i64::from(get_u16(font, *b"head", 16).unwrap_or(0));
    let flags = if r.all_xmin_is_lsb { flags | 2 } else { flags & !2 };
    put_u16(font, *b"head", 16, flags, "head.flags")?;
    Ok(true)
}

/// fontTools' `maxp.recalc` on a compiled TrueType font, as its `maxp` compiler runs it when a
/// font whose `glyf` table was loaded is saved with `recalcBBoxes`: see [`apply_maxp_recalc`].
/// The glyph statistics come from the `glyf` and `loca` bytes, the side bearings from `hmtx`.
/// `maxp.numGlyphs` is the glyph count read; a font without `glyf` is left as it is.
pub(crate) fn recalc_compiled_maxp(font: &mut SfntFont) -> Result<()> {
    let Some(glyf) = font.get(*b"glyf") else { return Ok(()) };
    let count = usize::from(get_u16(font, *b"maxp", 4).ok_or_else(|| bad("the 'maxp' table is missing or cut short"))?);
    let long = get_u16(font, *b"head", 50).ok_or_else(|| bad("the 'head' table is missing or cut short"))? != 0;
    let loca = font.get(*b"loca").ok_or_else(|| TablesError::MissingTable("loca".into()))?;
    let glyphs = compiled_glyph_stats(glyf, loca, long, count)?;
    let hmtx = font_metrics(font, false, count);
    let r = maxp_recalc(&glyphs, hmtx.as_deref())?;
    apply_maxp_recalc(font, &r)?;
    Ok(())
}

/// What [`maxp_recalc`] needs of each glyph of a compiled `glyf` table. A glyph that `loca`
/// places past the end of `glyf` is cut at the end, and one it gives a negative length is
/// empty, as fontTools' slicing of the table makes them.
fn compiled_glyph_stats(glyf: &[u8], loca: &[u8], long: bool, count: usize) -> Result<Vec<GlyphStat>> {
    let offset = |i: usize| -> Option<usize> {
        if long {
            let at = i.checked_mul(4)?;
            let b = loca.get(at..at.checked_add(4)?)?;
            usize::try_from(u32::from_be_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?])).ok()
        } else {
            let at = i.checked_mul(2)?;
            let b = loca.get(at..at.checked_add(2)?)?;
            Some(usize::from(u16::from_be_bytes([*b.first()?, *b.get(1)?])) * 2)
        }
    };
    let mut out = Vec::with_capacity(count);
    for gid in 0..count {
        let (Some(start), Some(end)) = (offset(gid), offset(gid + 1)) else {
            return Err(bad("the 'loca' table is cut short"));
        };
        let start = start.min(glyf.len());
        let end = end.min(glyf.len()).max(start);
        let data = glyf.get(start..end).unwrap_or_default();
        out.push(compiled_glyph_stat(data).map_err(|e| bad(format!("glyph {gid}: {e}")))?);
    }
    Ok(out)
}

fn compiled_glyph_stat(data: &[u8]) -> std::result::Result<GlyphStat, &'static str> {
    let u16_at = |at: usize| data.get(at..at.saturating_add(2)).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(u16::from_be_bytes);
    let i16_at = |at: usize| u16_at(at).map(|v| v as i16);
    let mut stat = GlyphStat { contours: 0, bounds: [0; 4], points: 0, components: Vec::new() };
    if data.is_empty() {
        return Ok(stat);
    }
    let cut = "the glyph is cut short";
    stat.contours = i32::from(i16_at(0).ok_or(cut)?);
    for (slot, at) in stat.bounds.iter_mut().zip([2, 4, 6, 8]) {
        *slot = i64::from(i16_at(at).ok_or(cut)?);
    }
    if stat.contours > 0 {
        let last = 10 + 2 * (stat.contours as usize - 1);
        stat.points = usize::from(u16_at(last).ok_or(cut)?) + 1;
    } else if stat.contours < 0 {
        // Each component record takes at least 6 bytes, so the count stays below the glyph's size.
        let mut at = 10usize;
        loop {
            let flags = i64::from(u16_at(at).ok_or(cut)?);
            stat.components.push(usize::from(u16_at(at + 2).ok_or(cut)?));
            at += 4 + if flags & ARG_1_AND_2_ARE_WORDS != 0 { 4 } else { 2 };
            at += if flags & WE_HAVE_A_SCALE != 0 {
                2
            } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                4
            } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
                8
            } else {
                0
            };
            if flags & MORE_COMPONENTS == 0 {
                break;
            }
        }
    }
    Ok(stat)
}
