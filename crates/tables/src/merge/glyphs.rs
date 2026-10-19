//! The glyph tables of the result: `glyf`, `loca`, `hmtx`, `vmtx`, and the glyph bounds the
//! font-wide tables are recomputed from (fontTools recomputes them when it saves the merged
//! font).
//!
//! Every glyph's bounds are recomputed from its points and written into its header, as
//! fontTools' `Glyph.recalcBounds` and `tryRecalcBoundsComposite` do on save (MIT, Copyright (c)
//! 2013 Google Inc. and the fontTools authors): a composite whose components are only moved
//! takes the union of its components' bounds, skipping empty ones; any other composite is
//! flattened, with its transforms and matched points, and its rounded extremes taken.

use std::borrow::Cow;

use super::plan::Plan;
use crate::dehint::{DehintOptions, dehint};
use crate::reorder::be::{Data, put_u16, put_u32, set_u16, to_u16};
use crate::reorder::names::num_glyphs;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// One input font's glyphs and metrics.
pub(super) struct Source<'a> {
    glyphs: Vec<Cow<'a, [u8]>>,
    h_metrics: Vec<(u16, i16)>,
    v_metrics: Option<Vec<(u16, i16)>>,
}

impl<'a> Source<'a> {
    /// The glyphs of font `index`. Every font but the first loses its glyph instructions: its
    /// font programs and `cvt ` are not carried over, so the instructions would call into the
    /// first font's.
    pub fn read(font: &'a SfntFont, index: usize) -> Result<Self> {
        let n = num_glyphs(font)?;
        let mut glyphs = split_glyf(font, n)?.into_iter().map(Cow::Borrowed).collect::<Vec<_>>();
        if index > 0 {
            let mut bare = SfntFont::new(font.flavour);
            for tag in [*b"head", *b"maxp", *b"glyf", *b"loca"] {
                if let Some(bytes) = font.get(tag) {
                    bare.insert(tag, bytes.to_vec());
                }
            }
            let options = DehintOptions { keep_font_hints: true, hinting_tables: Vec::new(), ..DehintOptions::default() };
            let stripped = dehint(&bare, &options)?;
            glyphs = split_glyf(&stripped, n)?.into_iter().map(|g| Cow::Owned(g.to_vec())).collect();
        }
        let h_metrics = metrics(font, *b"hhea", *b"hmtx", n)?.ok_or_else(|| TablesError::MissingTable("hmtx".into()))?;
        let v_metrics = metrics(font, *b"vhea", *b"vmtx", n)?;
        Ok(Source { glyphs, h_metrics, v_metrics })
    }

    /// The glyphs of a CFF-flavoured font: its charstrings (subroutines expanded) and metrics.
    pub fn read_cff(font: &'a SfntFont, charstrings: Vec<Vec<u8>>) -> Result<Self> {
        let n = num_glyphs(font)?;
        if charstrings.len() != n {
            return Err(TablesError::invalid(format!("the 'CFF ' table has {} charstrings for {n} glyphs", charstrings.len())));
        }
        let h_metrics = metrics(font, *b"hhea", *b"hmtx", n)?.ok_or_else(|| TablesError::MissingTable("hmtx".into()))?;
        let v_metrics = metrics(font, *b"vhea", *b"vmtx", n)?;
        Ok(Source { glyphs: charstrings.into_iter().map(Cow::Owned).collect(), h_metrics, v_metrics })
    }

    pub fn glyph(&self, gid: u16) -> &[u8] {
        self.glyphs.get(usize::from(gid)).map(|g| g.as_ref()).unwrap_or_default()
    }

    pub fn metric(&self, gid: u16) -> (u16, i16) {
        self.h_metrics.get(usize::from(gid)).copied().unwrap_or_default()
    }
}

fn split_glyf(font: &SfntFont, n: usize) -> Result<Vec<&[u8]>> {
    let (Some(glyf), Some(loca), Some(head)) = (font.get(*b"glyf"), font.get(*b"loca"), font.get(*b"head")) else {
        return Err(TablesError::MissingTable("glyf".into()));
    };
    let long = Data::new(*b"head", head).i16(50)? != 0;
    let l = Data::new(*b"loca", loca);
    let g = Data::new(*b"glyf", glyf);
    let at = |i: usize| -> Result<usize> { if long { l.u32_usize(4 * i) } else { Ok(2 * usize::from(l.u16(2 * i)?)) } };
    let mut out = Vec::with_capacity(n);
    let mut start = at(0)?;
    for i in 1..=n {
        let end = at(i)?;
        if end < start {
            return Err(l.bad("has decreasing offsets"));
        }
        out.push(g.slice(start, end - start)?);
        start = end;
    }
    Ok(out)
}

/// (advance, side bearing) of every glyph from `hmtx` or `vmtx`; `None` without the tables.
fn metrics(font: &SfntFont, hea: [u8; 4], mtx: [u8; 4], n: usize) -> Result<Option<Vec<(u16, i16)>>> {
    let (Some(header), Some(table)) = (font.get(hea), font.get(mtx)) else { return Ok(None) };
    let long = usize::from(Data::new(hea, header).u16(34)?);
    let d = Data::new(mtx, table);
    if long == 0 || long > n {
        return Err(d.bad(format!("has {long} long metrics for {n} glyphs")));
    }
    let mut out = Vec::with_capacity(n);
    let mut advance = 0u16;
    for g in 0..n {
        if g < long {
            advance = d.u16(4 * g)?;
            out.push((advance, d.i16(4 * g + 2)?));
        } else {
            out.push((advance, d.i16(4 * long + 2 * (g - long))?));
        }
    }
    Ok(Some(out))
}

/// (advance, side bearing) of every glyph.
pub(super) type Metrics = Vec<(u16, i16)>;

/// The glyph tables of the result.
pub(super) struct Built {
    pub count: usize,
    pub glyf: Vec<u8>,
    pub loca: Vec<u8>,
    /// `head.indexToLocFormat`: 1 for long offsets.
    pub long_loca: bool,
    pub hmtx: Vec<u8>,
    pub h_metrics: Metrics,
    /// `vmtx` and its metrics, when every font has vertical metrics.
    pub vertical: Option<(Vec<u8>, Metrics)>,
    /// (xMin, yMin, xMax, yMax) of every glyph with outlines.
    pub bounds: Vec<Option<[i16; 4]>>,
}

/// The glyph tables of the result. With `cff` the sources hold charstrings: `glyf` and `loca`
/// stay empty and the bounds are left for the `CFF ` table to give.
pub(super) fn build(sources: &[Source<'_>], plan: &Plan, cff: bool, warnings: &mut Vec<String>) -> Result<Built> {
    let mut datas: Vec<Vec<u8>> = Vec::with_capacity(if cff { 0 } else { plan.slots.len() });
    let mut h_metrics = Vec::with_capacity(plan.slots.len());
    for slot in &plan.slots {
        let source = sources.get(slot.font).ok_or_else(|| TablesError::invalid("a glyph comes from a font that was not given"))?;
        if cff {
            h_metrics.push(source.metric(slot.gid));
            continue;
        }
        let mut data = source.glyph(slot.gid).to_vec();
        remap_components(&mut data, |old| {
            plan.new_id(slot.font, old).ok_or_else(|| {
                TablesError::BadOptions(format!(
                    "glyph `{}` (font {}) uses a component the result does not take: add the component to the map",
                    slot.name,
                    slot.font + 1
                ))
            })
        })?;
        datas.push(data);
        h_metrics.push(source.metric(slot.gid));
    }
    let bounds = if cff { vec![None; plan.slots.len()] } else { recalc_bounds(&mut datas, warnings)? };
    let mut glyf: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::with_capacity(datas.len() + 1);
    for data in &datas {
        offsets.push(glyf.len());
        glyf.extend_from_slice(data);
        while !glyf.len().is_multiple_of(4) {
            glyf.push(0);
        }
    }
    offsets.push(glyf.len());
    if glyf.len() > crate::limits::MAX_SFNT {
        return Err(TablesError::TooLarge { what: "the merged glyf table", limit: crate::limits::MAX_SFNT });
    }
    // fontTools' rule: short offsets whenever they reach.
    let long_loca = glyf.len() / 2 > usize::from(u16::MAX);
    let mut loca = Vec::with_capacity(offsets.len() * if long_loca { 4 } else { 2 });
    for at in offsets {
        if long_loca {
            put_u32(&mut loca, u32::try_from(at).map_err(|_| TablesError::invalid("the merged glyf table is too large"))?);
        } else {
            put_u16(&mut loca, to_u16(at / 2, "a short loca offset")?);
        }
    }
    let vertical = if sources.iter().all(|s| s.v_metrics.is_some()) {
        let v: Vec<(u16, i16)> = plan
            .slots
            .iter()
            .map(|s| sources.get(s.font).and_then(|src| src.v_metrics.as_ref()).and_then(|m| m.get(usize::from(s.gid))).copied().unwrap_or_default())
            .collect();
        Some((write_metrics(&v)?.0, v))
    } else {
        if sources.iter().any(|s| s.v_metrics.is_some()) {
            warnings.push("'vmtx' and 'vhea' are left out: not every font has vertical metrics".into());
        }
        None
    };
    let (hmtx, _) = write_metrics(&h_metrics)?;
    Ok(Built { count: plan.slots.len(), glyf, loca, long_loca, hmtx, h_metrics, vertical, bounds })
}

/// `hmtx` or `vmtx` bytes and the count of long metrics: trailing glyphs with the advance of the
/// last share it.
pub(super) fn write_metrics(all: &[(u16, i16)]) -> Result<(Vec<u8>, u16)> {
    let final_advance = all.last().map_or(0, |m| m.0);
    let mut count = all.len();
    while count >= 2 && all.get(count - 2).is_some_and(|m| m.0 == final_advance) {
        count -= 1;
    }
    let mut out = Vec::with_capacity(4 * count + 2 * (all.len() - count));
    for (i, (advance, side)) in all.iter().enumerate() {
        if i < count {
            put_u16(&mut out, *advance);
        }
        out.extend_from_slice(&side.to_be_bytes());
    }
    Ok((out, to_u16(count, "the long metrics count")?))
}

/// The long-metric count [`write_metrics`] gives.
pub(super) fn long_metric_count(all: &[(u16, i16)]) -> Result<u16> {
    let final_advance = all.last().map_or(0, |m| m.0);
    let mut count = all.len();
    while count >= 2 && all.get(count - 2).is_some_and(|m| m.0 == final_advance) {
        count -= 1;
    }
    to_u16(count, "the long metrics count")
}

/// The bounding box a glyph's header stores; `None` for an empty glyph.
fn glyph_bounds(data: &[u8]) -> Result<Option<[i16; 4]>> {
    if data.is_empty() {
        return Ok(None);
    }
    let d = Data::new(*b"glyf", data);
    if d.i16(0)? == 0 {
        return Ok(None);
    }
    Ok(Some([d.i16(2)?, d.i16(4)?, d.i16(6)?, d.i16(8)?]))
}

/// Deepest component nesting followed when bounds are recomputed.
const MAX_COMPONENT_DEPTH: usize = 64;
/// Most work spent flattening composites with transformed or point-matched components, over the
/// whole merged font: one unit per component visited and one per point copied.
const MAX_FLATTEN_WORK: usize = 20_000_000;

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;

/// One component as the bounds need it.
struct Component {
    flags: u16,
    gid: usize,
    /// Offsets with [`ARGS_ARE_XY_VALUES`], else the matched point numbers.
    args: (i32, i32),
    /// `[xx, xy, yx, yy]`, when the component stores a scale or matrix.
    transform: Option<[f64; 4]>,
}

impl Component {
    fn is_xy(&self) -> bool {
        self.flags & ARGS_ARE_XY_VALUES != 0
    }
}

/// A glyph's outline as the bounds need it.
enum Outline {
    /// No data, or no contours: no bounds of its own.
    Empty,
    Simple(Vec<(f64, f64)>),
    Composite(Vec<Component>),
}

/// Where the bounds computation of a glyph stands.
#[derive(Clone, Copy)]
enum Done {
    Todo,
    Busy,
    /// `None`: the glyph cannot be decoded, nests too deep, refers to itself or ran out of work.
    Bounds(Option<[i16; 4]>),
}

/// Recomputes the bounds of every glyph in `datas` (in the result's glyph order, components
/// already renumbered) and writes them into the glyph headers. A glyph whose bounds cannot be
/// recomputed keeps the ones it stores, with one warning for all of them.
fn recalc_bounds(datas: &mut [Vec<u8>], warnings: &mut Vec<String>) -> Result<Vec<Option<[i16; 4]>>> {
    let outlines: Vec<Option<Outline>> = datas.iter().map(|d| parse_outline(d)).collect();
    let mut done = vec![Done::Todo; outlines.len()];
    let mut budget = MAX_FLATTEN_WORK;
    let mut out = Vec::with_capacity(datas.len());
    let mut kept = 0usize;
    for (g, data) in datas.iter_mut().enumerate() {
        let stored = glyph_bounds(data)?;
        if matches!(outlines.get(g), Some(Some(Outline::Empty))) {
            out.push(stored);
            continue;
        }
        match bounds_of(&outlines, g, 0, &mut done, &mut budget) {
            Some(b) => {
                for (i, v) in b.iter().enumerate() {
                    set_u16(data, 2 + 2 * i, *v as u16)?;
                }
                out.push(Some(b));
            }
            None => {
                kept += 1;
                out.push(stored);
            }
        }
    }
    if kept > 0 {
        warnings.push(format!("the bounds of {kept} glyphs could not be recomputed, so they keep the bounds they store"));
    }
    Ok(out)
}

/// The bounds of glyph `g`, as `Glyph.recalcBounds`; 0s for an empty glyph.
fn bounds_of(outlines: &[Option<Outline>], g: usize, level: usize, done: &mut [Done], budget: &mut usize) -> Option<[i16; 4]> {
    match done.get(g).copied() {
        Some(Done::Bounds(b)) => return b,
        Some(Done::Busy) | None => return None,
        Some(Done::Todo) => {}
    }
    if level > MAX_COMPONENT_DEPTH {
        return None;
    }
    if let Some(slot) = done.get_mut(g) {
        *slot = Done::Busy;
    }
    let b = match outlines.get(g)? {
        None => None,
        Some(Outline::Empty) => Some([0; 4]),
        Some(Outline::Simple(points)) => int_bounds(points),
        Some(Outline::Composite(components)) if components.iter().all(|c| c.is_xy() && c.transform.is_none()) => {
            let mut b: Option<[f64; 4]> = None;
            let mut failed = false;
            for c in components {
                let Some(child) = bounds_of(outlines, c.gid, level + 1, done, budget) else {
                    failed = true;
                    break;
                };
                // Empty components leave the composite's bounds alone.
                if child[0] == child[2] && child[1] == child[3] {
                    continue;
                }
                let [x0, y0, x1, y1] = child.map(f64::from);
                let (dx, dy) = (f64::from(c.args.0), f64::from(c.args.1));
                b = Some(match b {
                    None => [x0 + dx, y0 + dy, x1 + dx, y1 + dy],
                    Some(o) => [o[0].min(x0 + dx), o[1].min(y0 + dy), o[2].max(x1 + dx), o[3].max(y1 + dy)],
                });
            }
            if failed { None } else { b.map_or(Some([0; 4]), |b| int_bounds(&[(b[0], b[1]), (b[2], b[3])])) }
        }
        Some(Outline::Composite(_)) => flatten(outlines, g, level, budget).and_then(|points| int_bounds(&points)),
    };
    if let Some(slot) = done.get_mut(g) {
        *slot = Done::Bounds(b);
    }
    b
}

/// The points of glyph `g` with its components placed, as fontTools' `getCoordinates`.
fn flatten(outlines: &[Option<Outline>], g: usize, level: usize, budget: &mut usize) -> Option<Vec<(f64, f64)>> {
    if level > MAX_COMPONENT_DEPTH {
        return None;
    }
    match outlines.get(g)?.as_ref()? {
        Outline::Empty => Some(Vec::new()),
        Outline::Simple(points) => {
            *budget = budget.checked_sub(points.len())?;
            Some(points.clone())
        }
        Outline::Composite(components) => {
            let mut all: Vec<(f64, f64)> = Vec::new();
            for c in components {
                *budget = budget.checked_sub(1)?;
                let mut points = flatten(outlines, c.gid, level + 1, budget)?;
                let transform = |points: &mut Vec<(f64, f64)>, t: [f64; 4]| {
                    for p in points.iter_mut() {
                        *p = (p.0 * t[0] + p.1 * t[2], p.0 * t[1] + p.1 * t[3]);
                    }
                };
                let shift = |points: &mut Vec<(f64, f64)>, dx: f64, dy: f64| {
                    for p in points.iter_mut() {
                        *p = (p.0 + dx, p.1 + dy);
                    }
                };
                if c.is_xy() {
                    let (dx, dy) = (f64::from(c.args.0), f64::from(c.args.1));
                    match c.transform {
                        None => shift(&mut points, dx, dy),
                        // The offset is scaled too.
                        Some(t) if c.flags & SCALED_COMPONENT_OFFSET != 0 => {
                            shift(&mut points, dx, dy);
                            transform(&mut points, t);
                        }
                        Some(t) => {
                            transform(&mut points, t);
                            shift(&mut points, dx, dy);
                        }
                    }
                } else {
                    if let Some(t) = c.transform {
                        transform(&mut points, t);
                    }
                    let parent = usize::try_from(c.args.0).ok().and_then(|i| all.get(i).copied());
                    let child = usize::try_from(c.args.1).ok().and_then(|i| points.get(i).copied());
                    if let (Some(a), Some(b)) = (parent, child) {
                        shift(&mut points, a.0 - b.0, a.1 - b.1);
                    }
                }
                all.append(&mut points);
            }
            Some(all)
        }
    }
}

/// The rounded extremes of `points` (otRound), clamped to what a glyph header holds; 0s for no
/// points. `None` when a value is not finite.
fn int_bounds(points: &[(f64, f64)]) -> Option<[i16; 4]> {
    let Some(&(x, y)) = points.first() else { return Some([0; 4]) };
    let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
    for &(x, y) in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let round = |v: f64| -> Option<i16> {
        let r = (v + 0.5).floor();
        r.is_finite().then(|| r.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16)
    };
    Some([round(x0)?, round(y0)?, round(x1)?, round(y1)?])
}

/// The outline of one glyph's data; `None` when it cannot be decoded.
fn parse_outline(b: &[u8]) -> Option<Outline> {
    if b.is_empty() {
        return Some(Outline::Empty);
    }
    let d = Data::new(*b"glyf", b);
    let contours = d.i16(0).ok()?;
    match contours {
        0 => Some(Outline::Empty),
        n if n > 0 => parse_simple(&d, usize::from(n.unsigned_abs())).map(Outline::Simple),
        _ => parse_composite(&d).map(Outline::Composite),
    }
}

fn parse_simple(d: &Data<'_>, contours: usize) -> Option<Vec<(f64, f64)>> {
    const X_SHORT: u8 = 0x02;
    const Y_SHORT: u8 = 0x04;
    const REPEAT: u8 = 0x08;
    const X_SAME: u8 = 0x10;
    const Y_SAME: u8 = 0x20;
    let last_end = d.u16(10 + 2 * (contours - 1)).ok()?;
    // At most 65,536 points: the contour ends are 16-bit.
    let points = usize::from(last_end) + 1;
    let mut at = 10 + 2 * contours;
    at += 2 + usize::from(d.u16(at).ok()?);
    let mut flags = Vec::with_capacity(points);
    while flags.len() < points {
        let f = d.u8(at).ok()?;
        at += 1;
        let repeat = if f & REPEAT != 0 {
            at += 1;
            usize::from(d.u8(at - 1).ok()?)
        } else {
            0
        };
        for _ in 0..=repeat {
            if flags.len() < points {
                flags.push(f);
            }
        }
    }
    let mut axis = |short: u8, same: u8| -> Option<Vec<f64>> {
        let mut v = 0i32;
        let mut out = Vec::with_capacity(points);
        for f in &flags {
            if f & short != 0 {
                let delta = i32::from(d.u8(at).ok()?);
                at += 1;
                v += if f & same != 0 { delta } else { -delta };
            } else if f & same == 0 {
                v += i32::from(d.i16(at).ok()?);
                at += 2;
            }
            out.push(f64::from(v));
        }
        Some(out)
    };
    let xs = axis(X_SHORT, X_SAME)?;
    let ys = axis(Y_SHORT, Y_SAME)?;
    Some(xs.into_iter().zip(ys).collect())
}

fn parse_composite(d: &Data<'_>) -> Option<Vec<Component>> {
    let f2dot14 = |at: usize| -> Option<f64> { Some(f64::from(d.i16(at).ok()?) / 16384.0) };
    let mut at = 10usize;
    let mut out = Vec::new();
    loop {
        let flags = d.u16(at).ok()?;
        let gid = usize::from(d.u16(at + 2).ok()?);
        at += 4;
        let xy = flags & ARGS_ARE_XY_VALUES != 0;
        let args = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            let (a, b) = (d.u16(at).ok()?, d.u16(at + 2).ok()?);
            at += 4;
            if xy { (i32::from(a as i16), i32::from(b as i16)) } else { (i32::from(a), i32::from(b)) }
        } else {
            let (a, b) = (d.u8(at).ok()?, d.u8(at + 1).ok()?);
            at += 2;
            if xy { (i32::from(a as i8), i32::from(b as i8)) } else { (i32::from(a), i32::from(b)) }
        };
        let transform = if flags & WE_HAVE_A_SCALE != 0 {
            let s = f2dot14(at)?;
            at += 2;
            Some([s, 0.0, 0.0, s])
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            let t = [f2dot14(at)?, 0.0, 0.0, f2dot14(at + 2)?];
            at += 4;
            Some(t)
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            let t = [f2dot14(at)?, f2dot14(at + 2)?, f2dot14(at + 4)?, f2dot14(at + 6)?];
            at += 8;
            Some(t)
        } else {
            None
        };
        out.push(Component { flags, gid, args, transform });
        if flags & MORE_COMPONENTS == 0 {
            return Some(out);
        }
    }
}

/// Rewrites the component glyph ids of a composite glyph in place.
fn remap_components(glyph: &mut [u8], map: impl Fn(u16) -> Result<u16>) -> Result<()> {
    if glyph.len() < 10 || Data::new(*b"glyf", glyph).i16(0)? >= 0 {
        return Ok(());
    }
    let mut at = 10usize;
    loop {
        let d = Data::new(*b"glyf", glyph);
        let flags = d.u16(at)?;
        let new = map(d.u16(at + 2)?)?;
        set_u16(glyph, at + 2, new)?;
        at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
        if flags & 0x0008 != 0 {
            at += 2;
        } else if flags & 0x0040 != 0 {
            at += 4;
        } else if flags & 0x0080 != 0 {
            at += 8;
        }
        if flags & 0x0020 == 0 {
            return Ok(());
        }
        if at >= glyph.len() {
            return Err(TablesError::invalid("a composite glyph is cut short"));
        }
    }
}

/// `OS/2.xAvgCharWidth` as fontTools recomputes it: the rounded mean of the advances that are
/// not zero.
pub(super) fn average_advance(metrics: &[(u16, i16)]) -> u16 {
    let widths: Vec<u64> = metrics.iter().filter(|m| m.0 > 0).map(|m| u64::from(m.0)).collect();
    if widths.is_empty() {
        return 0;
    }
    let sum: u64 = widths.iter().sum();
    let n = widths.len() as u64;
    // Round half up, as fontTools' otRound does for positive values.
    u16::try_from((2 * sum + n) / (2 * n)).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn header(contours: i16, stored: [i16; 4]) -> Vec<u8> {
        std::iter::once(contours).chain(stored).flat_map(i16::to_be_bytes).collect()
    }

    /// One contour of on-curve points with word coordinates.
    fn simple(points: &[(i16, i16)], stored: [i16; 4]) -> Vec<u8> {
        let mut b = header(1, stored);
        b.extend_from_slice(&(points.len() as u16 - 1).to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend(std::iter::repeat_n(0x01u8, points.len()));
        for axis in [0, 1] {
            let mut last = 0i16;
            for p in points {
                let v = if axis == 0 { p.0 } else { p.1 };
                b.extend_from_slice(&(v - last).to_be_bytes());
                last = v;
            }
        }
        b
    }

    /// Components (glyph, dx, dy, uniform scale) with word offsets.
    fn composite(parts: &[(u16, i16, i16, Option<f64>)], stored: [i16; 4]) -> Vec<u8> {
        let mut b = header(-1, stored);
        for (i, (gid, dx, dy, scale)) in parts.iter().enumerate() {
            let mut flags = ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES;
            if scale.is_some() {
                flags |= WE_HAVE_A_SCALE;
            }
            if i + 1 < parts.len() {
                flags |= MORE_COMPONENTS;
            }
            for v in [flags, *gid, *dx as u16, *dy as u16] {
                b.extend_from_slice(&v.to_be_bytes());
            }
            if let Some(s) = scale {
                b.extend_from_slice(&((s * 16384.0) as i16).to_be_bytes());
            }
        }
        b
    }

    /// The stored bounds of a glyph whose top was written as 0 (as in a corpus font whose
    /// underscore and comma accent store yMax 0) are recomputed from its points, and composites
    /// follow: moved components by their components' bounds, scaled ones by their placed points
    /// rounded as otRound rounds.
    #[test]
    fn merged_glyph_bounds_are_recomputed_from_the_outlines() {
        let bar = [(40, -125), (460, -125), (460, -55), (40, -55)];
        let mut datas = vec![
            simple(&bar, [40, -125, 460, 0]),
            Vec::new(),
            composite(&[(0, -80, -200, None), (1, 0, 0, None)], [0, -345, 104, 0]),
            composite(&[(0, 10, 0, Some(0.5))], [0, 0, 0, 0]),
            composite(&[(4, 0, 0, None)], [1, 2, 3, 4]),
        ];
        let mut warnings = Vec::new();
        let bounds = recalc_bounds(&mut datas, &mut warnings).unwrap();
        assert_eq!(bounds, [Some([40, -125, 460, -55]), None, Some([-40, -325, 380, -255]), Some([30, -62, 240, -27]), Some([1, 2, 3, 4]),]);
        for (g, b) in bounds.iter().enumerate() {
            if let Some(b) = b {
                assert_eq!(glyph_bounds(&datas[g]).unwrap().as_ref(), Some(b), "glyph {g}'s header holds its bounds");
            }
        }
        assert_eq!(warnings.len(), 1, "the glyph that refers to itself keeps its stored bounds: {warnings:?}");
    }
}
