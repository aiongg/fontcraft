//! The `glyf` work behind [`crate::curves`] (M12.29): curve conversion and componentizing. It
//! lives inside [`crate::outlines`] to share that module's glyph reader, composite drawing and
//! the recalculation of bounds, `maxp`, `hhea` and `hmtx` after an edit.
//!
//! Curve conversion redraws each simple glyph as fontTools' qu2cu and cu2qu command-line tools
//! do: the glyph is drawn as pen segments the way `glyf` glyphs draw (the contour starts at its
//! first on-curve point), the segments are converted, written back as points, on-curve points
//! that lie halfway between two off-curve points of the same kind are dropped, and coordinates
//! are rounded (`otRound`). A glyph whose points come out the same keeps its bytes and its
//! instructions; a changed glyph loses its instructions. Composites are left alone, and so are
//! contours of a single point (the reference tools drop them).
//!
//! Componentizing follows AFDKO's ttfcomponentizer (Apache License 2.0, Copyright 2018 Adobe):
//! a composite gets flags `ROUND_XY_TO_GRID`, plus `USE_MY_METRICS` on its first component when
//! the advances agree, and no instructions. Before a glyph is replaced, its new components are
//! drawn and must give the glyph's contours (each contour may start at another point).

use std::collections::HashMap;

use read_fonts::tables::glyf::PointFlags;
use read_fonts::types::Point as RawPoint;

use super::*;
use crate::curves::{ComponentizeOptions, ComponentizeReport, CurveOptions, CurveReport, CurveTarget, ReferenceGlyph};

type P = (f64, f64);

const ON: u8 = 0x01;
const CUBIC: u8 = 0x80;

/// Most steps the search for components may take on one glyph, and on one font.
pub(crate) const MAX_DETECT_STEPS_PER_GLYPH: usize = 100_000;
pub(crate) const MAX_DETECT_STEPS: usize = 50_000_000;
/// Glyphs with more contours are not searched for components (the search recurses per contour).
pub(crate) const MAX_DETECT_CONTOURS: usize = 256;

/// A simple glyph as points with their flags (on-curve, cubic) and contour ends.
#[derive(Clone, Debug, Default, PartialEq)]
struct Points {
    points: Vec<P>,
    flags: Vec<u8>,
    ends: Vec<usize>,
}

impl Points {
    fn read(bytes: &[u8]) -> std::result::Result<Self, String> {
        let g = RawSimple::read(FontData::new(bytes)).map_err(|e| format!("unreadable glyph: {e}"))?;
        let n = g.num_points();
        if n > MAX_GLYPH_POINTS {
            return Err("the glyph has too many points".into());
        }
        let mut raw = vec![RawPoint::<i32>::default(); n];
        let mut raw_flags = vec![PointFlags::default(); n];
        g.read_points_fast(&mut raw, &mut raw_flags).map_err(|e| format!("unreadable glyph: {e}"))?;
        let mut ends = Vec::new();
        let mut last: Option<usize> = None;
        for e in g.end_pts_of_contours() {
            let e = usize::from(e.get());
            if e >= n || last.is_some_and(|l| e <= l) {
                return Err("the glyph's contour ends do not match its points".into());
            }
            ends.push(e);
            last = Some(e);
        }
        let points = raw.iter().map(|p| (f64::from(p.x), f64::from(p.y))).collect();
        let flags = raw_flags
            .iter()
            .map(|f| {
                if f.is_on_curve() {
                    ON
                } else if f.is_off_curve_cubic() {
                    CUBIC
                } else {
                    0
                }
            })
            .collect();
        Ok(Self { points, flags, ends })
    }

    fn contours(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let mut start = 0usize;
        self.ends.iter().map(move |&e| {
            let s = start;
            start = e.saturating_add(1);
            (s, e)
        })
    }

    fn has_cubic(&self) -> bool {
        self.flags.iter().any(|f| f & CUBIC != 0)
    }
}

/// A pen segment, as fontTools' segment pens receive them.
#[derive(Clone, Debug, PartialEq)]
enum Op {
    Move(P),
    Line(P),
    /// Off-curve points and the end point; no end point for a contour of off-curve points only.
    Quad(Vec<P>, Option<P>),
    Cubic(Vec<P>, P),
    Close,
}

impl Op {
    fn last(&self) -> Option<P> {
        match self {
            Op::Move(p) | Op::Line(p) | Op::Cubic(_, p) => Some(*p),
            Op::Quad(_, end) => *end,
            Op::Close => None,
        }
    }
}

fn mid(a: P, b: P) -> P {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// One contour drawn as `glyf` glyphs draw (fontTools' `Glyph.draw`).
fn draw_contour(pts: &[P], flags: &[u8]) -> std::result::Result<Vec<Op>, String> {
    let n = pts.len();
    let on = |i: usize| flags.get(i).is_some_and(|f| f & ON != 0);
    let cubic = |i: usize| flags.get(i).is_some_and(|f| f & CUBIC != 0);
    let mut ops = Vec::new();
    if !(0..n).any(on) {
        let all_cubic = (0..n).all(cubic);
        if !all_cubic && (0..n).any(cubic) {
            return Err("a contour mixes cubic and quadratic off-curve points".into());
        }
        if all_cubic {
            if n % 2 == 1 {
                return Err("a contour has an odd number of cubic off-curve points".into());
            }
            let (Some(&l), Some(&f)) = (pts.last(), pts.first()) else { return Ok(ops) };
            ops.push(Op::Move(mid(l, f)));
            for i in (0..n).step_by(2) {
                let (Some(&p1), Some(&p2), Some(&p4)) = (pts.get(i), pts.get(i + 1), pts.get(if i + 2 < n { i + 2 } else { 0 })) else { break };
                ops.push(Op::Cubic(vec![p1, p2], mid(p2, p4)));
            }
        } else {
            ops.push(Op::Quad(pts.to_vec(), None));
        }
        ops.push(Op::Close);
        return Ok(ops);
    }
    // Rotate so the contour ends at its first on-curve point, which the move goes to.
    let first_on = (0..n).position(on).unwrap_or(0) + 1;
    let order: Vec<usize> = (first_on..n).chain(0..first_on).collect();
    let rp: Vec<P> = order.iter().filter_map(|&i| pts.get(i).copied()).collect();
    let rf: Vec<u8> = order.iter().filter_map(|&i| flags.get(i).copied()).collect();
    if let Some(&last) = rp.last() {
        ops.push(Op::Move(last));
    }
    let mut at = 0usize;
    while at < rp.len() {
        let Some(next_on) = rf.iter().skip(at).position(|f| f & ON != 0).map(|k| k + 1) else { break };
        if next_on == 1 {
            // The closing line is implied by the close.
            if rp.len() - at > 1
                && let Some(&p) = rp.get(at)
            {
                ops.push(Op::Line(p));
            }
        } else {
            let offs = rf.get(at..at + next_on - 1).unwrap_or(&[]);
            let is_cubic = offs.iter().any(|f| f & CUBIC != 0);
            if is_cubic && !offs.iter().all(|f| f & CUBIC != 0) {
                return Err("a segment mixes cubic and quadratic off-curve points".into());
            }
            let seg = rp.get(at..at + next_on).unwrap_or(&[]);
            if is_cubic {
                let count = next_on;
                if count < 3 || count % 2 == 0 {
                    return Err("a cubic segment has an odd number of off-curve points".into());
                }
                let mut i = 0usize;
                while i + 3 < count {
                    let (Some(&p1), Some(&p2), Some(&p4)) = (seg.get(i), seg.get(i + 1), seg.get(i + 2)) else { break };
                    ops.push(Op::Cubic(vec![p1, p2], mid(p2, p4)));
                    i += 2;
                }
                if let Some(tail) = seg.get(count - 3..count)
                    && let [a, b, c] = tail
                {
                    ops.push(Op::Cubic(vec![*a, *b], *c));
                }
            } else if let Some((&end, offs)) = seg.split_last() {
                ops.push(Op::Quad(offs.to_vec(), Some(end)));
            }
        }
        at += next_on;
    }
    ops.push(Op::Close);
    Ok(ops)
}

fn isclose(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs())
}

/// fontTools' `Qu2CuPen.filterContour`. Returns the contour and the cubic curves it made.
fn qu2cu_contour(ops: Vec<Op>, max_err: f64, all_cubic: bool) -> std::result::Result<(Vec<Op>, usize), String> {
    let mut out: Vec<Op> = Vec::with_capacity(ops.len());
    let mut run: Vec<Vec<P>> = Vec::new();
    let mut current: Option<P> = None;
    let mut made = 0usize;
    let mut flush = |run: &mut Vec<Vec<P>>, out: &mut Vec<Op>| -> std::result::Result<(), String> {
        if run.is_empty() {
            return Ok(());
        }
        let curves = fontcraft_geom::qu2cu::quadratic_to_curves(run, max_err, all_cubic).map_err(|e| e.to_string())?;
        run.clear();
        for c in curves {
            match c {
                fontcraft_geom::qu2cu::Curve::Cubic([_, a, b, end]) => {
                    made += 1;
                    out.push(Op::Cubic(vec![a, b], end));
                }
                fontcraft_geom::qu2cu::Curve::Quadratic([_, a, end]) => out.push(Op::Quad(vec![a], Some(end))),
            }
        }
        Ok(())
    };
    for op in ops {
        let next = op.last();
        match &op {
            Op::Quad(offs, end) if all_cubic || (offs.len() > 1 && end.is_some()) => {
                let Some(end) = end else { return Err("a contour without on-curve points cannot be made all cubic".into()) };
                let start = current.ok_or("a curve does not start at a point")?;
                let mut spline = Vec::with_capacity(offs.len() + 2);
                spline.push(start);
                spline.extend_from_slice(offs);
                spline.push(*end);
                run.push(spline);
            }
            _ => {
                flush(&mut run, &mut out)?;
                out.push(op);
            }
        }
        current = next;
    }
    flush(&mut run, &mut out)?;
    if all_cubic {
        return Ok((out, made));
    }
    // Quadratic pieces whose shared on-curve point is implied join into one spline again.
    let mut joined: Vec<Op> = Vec::with_capacity(out.len());
    for op in out {
        if let Op::Quad(offs, end) = &op
            && let Some(Op::Quad(prev_offs, Some(pt1))) = joined.last()
            && let (Some(&pt0), Some(&pt2)) = (prev_offs.last(), offs.first())
            && isclose(pt2.0 - pt1.0, pt1.0 - pt0.0)
            && isclose(pt2.1 - pt1.1, pt1.1 - pt0.1)
        {
            let mut merged = prev_offs.clone();
            merged.extend_from_slice(offs);
            if let Some(last) = joined.last_mut() {
                *last = Op::Quad(merged, *end);
            }
            continue;
        }
        joined.push(op);
    }
    Ok((joined, made))
}

/// fontTools' `Cu2QuPen` on one contour: each cubic becomes a quadratic spline within
/// `max_err`; with `mixed`, a cubic that one quadratic cannot replace stays. Returns the contour
/// and the cubic curves converted.
fn cu2qu_contour(ops: Vec<Op>, max_err: f64, mixed: bool) -> std::result::Result<(Vec<Op>, usize), String> {
    let mut out = Vec::with_capacity(ops.len());
    let mut current: Option<P> = None;
    let mut done = 0usize;
    for op in ops {
        let next = op.last();
        match &op {
            Op::Cubic(offs, end) => {
                let (Some(start), [a, b]) = (current, offs.as_slice()) else { return Err("a cubic curve does not start at a point".into()) };
                let pt = |p: P| fontcraft_geom::Point::new(p.0, p.1);
                let cubic = fontcraft_geom::CubicBez::new(pt(start), pt(*a), pt(*b), pt(*end));
                let spline = fontcraft_geom::cubics_to_quadratic_splines(&[cubic], max_err)
                    .and_then(|mut s| s.pop())
                    .ok_or_else(|| format!("no quadratic spline of at most 100 pieces comes within {max_err} units of a curve"))?;
                let points = spline.points();
                if mixed && points.len() > 3 {
                    out.push(op);
                } else {
                    let offs: Vec<P> = points.get(1..points.len().saturating_sub(1)).unwrap_or(&[]).iter().map(|p| (p.x, p.y)).collect();
                    out.push(Op::Quad(offs, Some(*end)));
                    done += 1;
                }
            }
            _ => out.push(op),
        }
        current = next;
    }
    Ok((out, done))
}

/// fontTools' `TTGlyphPen`, keeping one-point contours.
#[derive(Default)]
struct Pen {
    out: Points,
}

impl Pen {
    fn add(&mut self, p: P, flag: u8) {
        self.out.points.push(p);
        self.out.flags.push(flag);
    }

    fn replay(&mut self, ops: &[Op]) {
        for op in ops {
            match op {
                Op::Move(p) | Op::Line(p) => self.add(*p, ON),
                Op::Quad(offs, end) => {
                    for p in offs {
                        self.add(*p, 0);
                    }
                    if let Some(e) = end {
                        self.add(*e, ON);
                    }
                }
                Op::Cubic(offs, end) => {
                    for p in offs {
                        self.add(*p, CUBIC);
                    }
                    self.add(*end, ON);
                }
                Op::Close => self.close(),
            }
        }
    }

    fn close(&mut self) {
        let start = self.out.ends.last().map_or(0, |e| e + 1);
        let Some(mut end) = self.out.points.len().checked_sub(1) else { return };
        if end < start {
            return;
        }
        if end > start && self.out.points.get(start) == self.out.points.get(end) {
            self.out.points.pop();
            self.out.flags.pop();
            end -= 1;
        }
        self.out.ends.push(end);
    }
}

fn is_mid_point(p0: P, p1: P, p2: P) -> bool {
    let m = ((p0.0 + p2.0) * 0.5, (p0.1 + p2.1) * 0.5);
    let r = |v: f64| ot_round(v);
    (isclose(m.0, p1.0) && isclose(m.1, p1.1)) || (r(p0.0) + r(p2.0) == r(p1.0) * 2.0 && r(p0.1) + r(p2.1) == r(p1.1) * 2.0)
}

/// fontTools' `dropImpliedOnCurvePoints` on one glyph.
fn drop_implied(g: &mut Points) {
    let mut drop = vec![false; g.points.len()];
    let flag = |i: usize| g.flags.get(i).copied().unwrap_or(0);
    let point = |i: usize| g.points.get(i).copied().unwrap_or((0.0, 0.0));
    for (start, last) in g.contours() {
        for i in start..=last {
            if flag(i) & ON == 0 {
                continue;
            }
            let prv = if i > start { i - 1 } else { last };
            let nxt = if i < last { i + 1 } else { start };
            if flag(prv) & ON != 0 || flag(prv) != flag(nxt) {
                continue;
            }
            if is_mid_point(point(prv), point(i), point(nxt))
                && let Some(d) = drop.get_mut(i)
            {
                *d = true;
            }
        }
    }
    if !drop.contains(&true) {
        return;
    }
    let mut out = Points::default();
    let mut k = 0usize;
    for (start, last) in g.contours() {
        let before = out.points.len();
        for i in start..=last {
            if drop.get(i).copied().unwrap_or(false) {
                continue;
            }
            out.points.push(point(i));
            out.flags.push(flag(i));
            k += 1;
        }
        // A contour keeps its slot even when all its points go, as in the reference.
        out.ends.push(if out.points.len() > before { k - 1 } else { before.saturating_sub(1) });
    }
    *g = out;
}

/// The bytes of a simple glyph with cubic flags, without instructions.
fn encode_points(g: &Points) -> std::result::Result<Vec<u8>, String> {
    if g.points.is_empty() {
        return Ok(Vec::new());
    }
    if g.points.len() > MAX_GLYPH_POINTS || g.ends.len() > i16::MAX as usize {
        return Err("the glyph would have more points than a TrueType glyph can hold".into());
    }
    let xy: Vec<(i16, i16)> = g.points.iter().map(|p| Ok((coord(p.0)?, coord(p.1)?))).collect::<std::result::Result<_, String>>()?;
    let bbox = [
        xy.iter().map(|p| p.0).min().unwrap_or(0),
        xy.iter().map(|p| p.1).min().unwrap_or(0),
        xy.iter().map(|p| p.0).max().unwrap_or(0),
        xy.iter().map(|p| p.1).max().unwrap_or(0),
    ];
    let mut out = Vec::with_capacity(xy.len().saturating_mul(5).saturating_add(16));
    out.extend_from_slice(&(g.ends.len() as i16).to_be_bytes());
    for v in bbox {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for &e in &g.ends {
        out.extend_from_slice(&(e as u16).to_be_bytes());
    }
    out.extend_from_slice(&0u16.to_be_bytes());
    let (mut flags, mut xs, mut ys) = (Vec::with_capacity(xy.len()), Vec::new(), Vec::new());
    let (mut px, mut py) = (0i32, 0i32);
    for (&(x, y), &f) in xy.iter().zip(&g.flags) {
        let mut flag = f & (ON | CUBIC);
        flag |= delta(i32::from(x) - px, 0x02, 0x10, &mut xs);
        flag |= delta(i32::from(y) - py, 0x04, 0x20, &mut ys);
        (px, py) = (i32::from(x), i32::from(y));
        flags.push(flag);
    }
    let mut i = 0usize;
    while let Some(&flag) = flags.get(i) {
        let run = flags.iter().skip(i + 1).take(255).take_while(|&&f| f == flag).count();
        if run > 0 {
            out.push(flag | 0x08);
            out.push(run as u8);
        } else {
            out.push(flag);
        }
        i = i.saturating_add(run).saturating_add(1);
    }
    out.extend_from_slice(&xs);
    out.extend_from_slice(&ys);
    Ok(out)
}

/// One glyph converted, or `None` when its points stay the same.
fn convert_glyph(bytes: &[u8], target: CurveTarget, all_cubic: bool, mixed: bool) -> std::result::Result<Option<(Vec<u8>, usize)>, String> {
    let before = Points::read(bytes)?;
    let mut pen = Pen::default();
    let mut segments = 0usize;
    for (start, last) in before.contours() {
        let pts = before.points.get(start..=last).unwrap_or(&[]);
        let flags = before.flags.get(start..=last).unwrap_or(&[]);
        if pts.len() == 1 {
            pen.replay(&[Op::Move(pts.first().copied().unwrap_or((0.0, 0.0))), Op::Close]);
            continue;
        }
        let ops = draw_contour(pts, flags)?;
        let (ops, n) = match target {
            CurveTarget::Cubic { max_error } => qu2cu_contour(ops, max_error, all_cubic)?,
            CurveTarget::Quadratic { max_error, .. } => cu2qu_contour(ops, max_error, mixed)?,
        };
        segments += n;
        pen.replay(&ops);
    }
    let mut after = pen.out;
    drop_implied(&mut after);
    for p in &mut after.points {
        *p = (ot_round(p.0), ot_round(p.1));
    }
    if after == before {
        return Ok(None);
    }
    Ok(Some((encode_points(&after)?, segments)))
}

impl Work {
    /// Replaces a glyph's bytes with a simple glyph already encoded.
    fn set_encoded(&mut self, gid: usize, bytes: Vec<u8>) -> std::result::Result<(), String> {
        let parsed = parse(&bytes);
        if let Parsed::Bad(why) = parsed {
            return Err(why);
        }
        if let Some(g) = self.glyphs.get_mut(gid) {
            *g = Glyph { bytes, parsed, moved: true };
            self.bytes_changed = true;
        }
        Ok(())
    }

    fn any_cubic(&self) -> bool {
        self.glyphs.iter().any(|g| matches!(g.parsed, Parsed::Simple { .. }) && Points::read(&g.bytes).is_ok_and(|p| p.has_cubic()))
    }
}

fn selection_options(glyphs: &[String], exclude: &[String], ignore_errors: bool) -> OutlineOptions {
    let mut o = OutlineOptions::new(OutlineAction::Decompose);
    o.glyphs = glyphs.to_vec();
    o.exclude_glyphs = exclude.to_vec();
    o.ignore_errors = ignore_errors;
    o
}

fn refuse_variable_and_cff(font: &SfntFont, what: &str) -> Result<()> {
    if font.get(*b"glyf").is_none() {
        if font.is_cff() {
            return Err(TablesError::Unsupported(format!("{what} on CFF outlines; use `convert` to change the outline format")));
        }
        return Err(TablesError::MissingTable("glyf".into()));
    }
    if font.get(*b"gvar").is_some() {
        return Err(TablesError::Unsupported(format!("{what} on a variable font: instantiate it, or convert the sources")));
    }
    Ok(())
}

/// The font with its `glyf` curves converted.
pub(crate) fn convert(font: &SfntFont, o: &CurveOptions) -> Result<CurveReport> {
    refuse_variable_and_cff(font, "curve conversion")?;
    let mut work = Work::read(font)?;
    let options = selection_options(&o.glyphs, &o.exclude_glyphs, o.ignore_errors);
    let selected = work.selection(&options)?;
    let mut report = Report::default();
    let mut segments = 0usize;
    for gid in selected {
        let bytes = match work.glyphs.get(gid) {
            Some(Glyph { parsed: Parsed::Simple { .. }, bytes, .. }) => bytes.clone(),
            _ => continue,
        };
        let done = match convert_glyph(&bytes, o.target, o.all_cubic, o.mixed) {
            Ok(Some((bytes, n))) => work.set_encoded(gid, bytes).map(|()| Some(n)),
            Ok(None) => Ok(None),
            Err(reason) => Err(reason),
        };
        match done {
            Ok(Some(n)) => {
                segments += n;
                report.changed.insert(gid);
            }
            Ok(None) => {}
            Err(reason) => work.fail(gid, reason, &options, &mut report)?,
        }
    }
    let cubic_left = work.any_cubic();
    let changed: Vec<String> = report.changed.iter().filter_map(|&gid| work.names.get(gid).cloned()).collect();
    let mut out = work.finish(font)?;
    if !changed.is_empty()
        && let Some(head) = out.get(*b"head")
    {
        // `glyphDataFormat` 1 says the `glyf` table may hold cubic curves.
        let mut head = head.to_vec();
        put_u16(&mut head, 52, u16::from(cubic_left));
        out.insert(*b"head", head);
    }
    Ok(CurveReport { result: OutlineResult { font: out, changed }, failed: report.failed, segments })
}

// ── Componentize ────────────────────────────────────────────────────────────────────────────────

type CKey = Vec<(i64, i64, bool)>;

/// The contour from the start point that gives the smallest key: the same for a contour however
/// it starts, and for the contour moved by whole units.
fn canonical(c: &[Pt]) -> CKey {
    let k = key(c);
    let Some(min) = k.iter().min().copied() else { return k };
    let n = k.len();
    (0..n).filter(|&i| k.get(i) == Some(&min)).map(|i| k.iter().skip(i).chain(k.iter().take(i)).copied().collect::<CKey>()).min().unwrap_or(k)
}

fn moved(c: &CKey, dx: i64, dy: i64) -> CKey {
    c.iter().map(|&(x, y, on)| (x.saturating_add(dx), y.saturating_add(dy), on)).collect()
}

fn shape(c: &CKey) -> CKey {
    let (x0, y0) = c.first().map_or((0, 0), |p| (p.0, p.1));
    moved(c, -x0, -y0)
}

fn same_contours(a: &[Vec<Pt>], b: &[Vec<Pt>]) -> bool {
    let mut ka: Vec<CKey> = a.iter().map(|c| canonical(c)).collect();
    let mut kb: Vec<CKey> = b.iter().map(|c| canonical(c)).collect();
    ka.sort();
    kb.sort();
    ka == kb
}

impl Work {
    /// Replaces `gid` with `comps` when they draw its contours; the reason otherwise.
    fn try_componentize(&mut self, gid: usize, comps: Vec<Comp>) -> std::result::Result<(), String> {
        let current = self.draw(gid)?;
        let mut visits = 0usize;
        let drawn: Outline = self.parts_at(&comps, 0, &mut visits)?.into_iter().flatten().collect();
        if !same_contours(&current, &drawn) {
            return Err("its components do not give the glyph's outline".into());
        }
        let old = self.glyphs.get(gid).cloned();
        let was_changed = self.bytes_changed;
        self.set_composite(gid, comps, Vec::new())?;
        // A component that leads back to the glyph would make a loop: undo.
        if let Err(why) = self.draw(gid) {
            if let (Some(slot), Some(old)) = (self.glyphs.get_mut(gid), old) {
                *slot = old;
            }
            self.bytes_changed = was_changed;
            return Err(why);
        }
        Ok(())
    }

    fn advance(&self, gid: usize) -> Option<u16> {
        self.metrics.get(gid).map(|m| m.0)
    }
}

/// The font with glyphs rebuilt from components.
pub(crate) fn componentize(font: &SfntFont, o: &ComponentizeOptions) -> Result<ComponentizeReport> {
    refuse_variable_and_cff(font, "componentizing")?;
    let mut work = Work::read(font)?;
    let options = selection_options(&o.glyphs, &o.exclude_glyphs, true);
    let selected = work.selection(&options)?;
    let mut report = Report::default();
    match &o.reference {
        Some(reference) => from_reference(&mut work, &selected, reference, o.allow_transforms, &mut report),
        None => {
            if o.allow_transforms {
                return Err(TablesError::BadOptions("allowTransforms applies to components read from a reference source".into()));
            }
            detect(&mut work, &selected, &mut report)?;
        }
    }
    let changed: Vec<String> = report.changed.iter().filter_map(|&gid| work.names.get(gid).cloned()).collect();
    let out = work.finish(font)?;
    Ok(ComponentizeReport { result: OutlineResult { font: out, changed }, skipped: report.failed })
}

fn from_reference(work: &mut Work, selected: &[usize], reference: &BTreeMap<String, ReferenceGlyph>, allow_transforms: bool, report: &mut Report) {
    let index: HashMap<String, usize> = work.names.iter().enumerate().rev().map(|(i, n)| (n.clone(), i)).collect();
    let mut plans: Vec<(usize, &ReferenceGlyph)> = Vec::new();
    for (name, r) in reference {
        if let Some(&gid) = index.get(name.as_str())
            && selected.binary_search(&gid).is_ok()
            && !r.components.is_empty()
        {
            plans.push((gid, r));
        }
    }
    plans.sort_by_key(|p| p.0);
    for (gid, r) in plans {
        let outcome = match reference_components(work, &index, gid, r, allow_transforms) {
            Ok(None) => continue,
            Ok(Some(comps)) => work.try_componentize(gid, comps),
            Err(reason) => Err(reason),
        };
        match outcome {
            Ok(()) => {
                report.changed.insert(gid);
            }
            Err(reason) => report.failed.push(GlyphFailure { glyph: work.name(gid), reason }),
        }
    }
}

/// The components the reference gives `gid`; `None` when the glyph already has them.
fn reference_components(
    work: &Work,
    index: &HashMap<String, usize>,
    gid: usize,
    r: &ReferenceGlyph,
    allow_transforms: bool,
) -> std::result::Result<Option<Vec<Comp>>, String> {
    if r.has_contours {
        return Err("the reference glyph mixes contours and components".into());
    }
    let mut comps = Vec::with_capacity(r.components.len());
    for (i, c) in r.components.iter().enumerate() {
        let [xx, xy, yx, yy, dx, dy] = c.transform;
        if !allow_transforms && [xx, xy, yx, yy] != [1.0, 0.0, 0.0, 1.0] {
            return Err("a component is scaled, rotated or flipped".into());
        }
        let &cgid = index.get(c.glyph.as_str()).ok_or_else(|| format!("the font has no glyph `{}`", c.glyph))?;
        if ![xx, xy, yx, yy, dx, dy].iter().all(|v| v.is_finite()) {
            return Err("a component transform is not a number".into());
        }
        let mut flags = ROUND_XY;
        if i == 0 && r.same_advance_as_first {
            flags |= USE_MY_METRICS;
        }
        // The transform is in fontTools' order (xx, xy, yx, yy), which is the file's order.
        comps.push(Comp { flags, gid: cgid as u16, anchor: Anchor::Offset(ot_round(dx), ot_round(dy)), m: [xx, xy, yx, yy] });
    }
    if let Some(Glyph { parsed: Parsed::Composite { comps: have, .. }, .. }) = work.glyphs.get(gid)
        && have.iter().map(|c| (c.gid, c.anchor, c.m)).eq(comps.iter().map(|c| (c.gid, c.anchor, c.m)))
    {
        return Ok(None);
    }
    Ok(Some(comps))
}

/// A simple glyph's contours as keys, for the search.
struct Shapes {
    contours: Vec<CKey>,
}

fn detect(work: &mut Work, selected: &[usize], report: &mut Report) -> Result<()> {
    let mut simple: BTreeMap<usize, Shapes> = BTreeMap::new();
    for (gid, g) in work.glyphs.iter().enumerate().skip(1) {
        if let Parsed::Simple { outline, .. } = &g.parsed
            && !outline.is_empty()
            && outline.iter().all(|c| !c.is_empty())
        {
            simple.insert(gid, Shapes { contours: outline.iter().map(|c| canonical(c)).collect() });
        }
    }
    // Where each contour shape occurs: (glyph, contour).
    let mut by_shape: HashMap<CKey, Vec<(usize, usize)>> = HashMap::new();
    for (&gid, s) in &simple {
        for (k, c) in s.contours.iter().enumerate() {
            by_shape.entry(shape(c)).or_default().push((gid, k));
        }
    }
    let mut used_as_base = vec![false; work.glyphs.len()];
    let mut converted = vec![false; work.glyphs.len()];
    let mut steps_total = 0usize;
    for &gid in selected {
        let Some(target) = simple.get(&gid).filter(|t| t.contours.len() <= MAX_DETECT_CONTOURS) else { continue };
        if used_as_base.get(gid).copied().unwrap_or(true) {
            continue;
        }
        let mut steps = 0usize;
        let mut cover = Vec::new();
        let mut taken = vec![false; target.contours.len()];
        let ok = search(&simple, &by_shape, gid, target, &mut taken, &mut cover, &converted, &mut steps);
        steps_total = steps_total.saturating_add(steps);
        if steps_total > MAX_DETECT_STEPS {
            return Err(TablesError::invalid("finding components takes too long on this font: give a reference source or fewer glyphs"));
        }
        if !ok || cover.is_empty() {
            continue;
        }
        let advance = work.advance(gid);
        let comps: Vec<Comp> = cover
            .iter()
            .enumerate()
            .map(|(i, &(base, dx, dy))| {
                let mut flags = ROUND_XY;
                if i == 0 && advance.is_some() && work.advance(base) == advance {
                    flags |= USE_MY_METRICS;
                }
                Comp { flags, gid: base as u16, anchor: Anchor::Offset(dx as f64, dy as f64), m: IDENTITY }
            })
            .collect();
        match work.try_componentize(gid, comps) {
            Ok(()) => {
                report.changed.insert(gid);
                if let Some(c) = converted.get_mut(gid) {
                    *c = true;
                }
                for &(base, _, _) in &cover {
                    if let Some(u) = used_as_base.get_mut(base) {
                        *u = true;
                    }
                }
            }
            Err(reason) => report.failed.push(GlyphFailure { glyph: work.name(gid), reason }),
        }
    }
    Ok(())
}

/// Covers the target's contours not yet taken with whole glyphs moved by whole units, larger
/// glyphs first. `cover` gets (glyph, dx, dy) for each glyph used.
#[allow(clippy::too_many_arguments)]
fn search(
    simple: &BTreeMap<usize, Shapes>,
    by_shape: &HashMap<CKey, Vec<(usize, usize)>>,
    gid: usize,
    target: &Shapes,
    taken: &mut [bool],
    cover: &mut Vec<(usize, i64, i64)>,
    converted: &[bool],
    steps: &mut usize,
) -> bool {
    let Some(first) = taken.iter().position(|t| !t) else { return true };
    let Some(contour) = target.contours.get(first) else { return false };
    let Some(candidates) = by_shape.get(&shape(contour)) else { return false };
    *steps = steps.saturating_add(candidates.len());
    let mut ordered: Vec<(usize, usize)> = candidates
        .iter()
        .copied()
        .filter(|&(base, _)| base != gid && !converted.get(base).copied().unwrap_or(true))
        .filter(|&(base, _)| simple.get(&base).is_some_and(|s| s.contours.len() <= taken.iter().filter(|t| !**t).count()))
        // Of two equal glyphs, the later one refers to the earlier.
        .filter(|&(base, _)| base < gid || simple.get(&base).is_none_or(|s| s.contours.len() < target.contours.len()))
        .collect();
    ordered.sort_by_key(|&(base, k)| (std::cmp::Reverse(simple.get(&base).map_or(0, |s| s.contours.len())), base, k));
    let (cx, cy) = contour.first().map_or((0, 0), |p| (p.0, p.1));
    for (base, k) in ordered {
        *steps = steps.saturating_add(1);
        if *steps > MAX_DETECT_STEPS_PER_GLYPH {
            return false;
        }
        let Some(b) = simple.get(&base) else { continue };
        let Some(bk) = b.contours.get(k) else { continue };
        let (bx, by) = bk.first().map_or((0, 0), |p| (p.0, p.1));
        let (dx, dy) = (cx - bx, cy - by);
        // Every contour of the base, moved, must be an untaken contour of the target.
        let mut claimed: Vec<usize> = Vec::with_capacity(b.contours.len());
        let fits = b.contours.iter().all(|bc| {
            let want = moved(bc, dx, dy);
            match target
                .contours
                .iter()
                .enumerate()
                .position(|(i, tc)| !taken.get(i).copied().unwrap_or(true) && !claimed.contains(&i) && *tc == want)
            {
                Some(i) => {
                    claimed.push(i);
                    true
                }
                None => false,
            }
        });
        if !fits {
            continue;
        }
        for &i in &claimed {
            if let Some(t) = taken.get_mut(i) {
                *t = true;
            }
        }
        cover.push((base, dx, dy));
        if search(simple, by_shape, gid, target, taken, cover, converted, steps) {
            return true;
        }
        cover.pop();
        for &i in &claimed {
            if let Some(t) = taken.get_mut(i) {
                *t = false;
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "outlines_curves_tests.rs"]
mod tests;
