//! Outlines between flavours (M12.30): contours read from `CFF `, `CFF2` or `glyf`, curves
//! converted between cubic and quadratic, contours reversed, and simple `glyf` glyphs written.
//!
//! The order follows AFDKO otf2ttf (Apache-2.0), which draws through fontTools' `Cu2QuPen`
//! with `reverse_direction` into a `TTGlyphPen`: curves are converted first, then the contour
//! is reversed with fontTools' `reversedContour` rule (the start point stays first, an implied
//! closing line is never written), and a last point equal to the first is dropped.
//!
//! PostScript outlines are drawn as fontTools draws them (`T2OutlineExtractor`): in font units
//! as the charstring states them, fractions kept, the FontMatrix not applied. An accented
//! glyph drawn with `seac` reports its base and accent instead of drawing them.

use std::collections::HashMap;

use fontcraft_geom::cu2qu_multi::curves_to_quadratic;
use fontcraft_geom::qu2cu::{Curve, quadratic_to_curves};
use fontcraft_geom::{BezPath, CubicBez, Point, Shape as _};
use read_fonts::ps::cff::CffFontRef;
use read_fonts::tables::glyf::{Anchor, CompositeGlyphFlags, Glyf, Glyph};
use read_fonts::tables::loca::{Loca, LocaGlyph};
use read_fonts::types::GlyphId;
use read_fonts::{FontData, FontRead};

use super::cff_table::{Drawing, Programs};
use crate::{Result, TablesError};

/// Deepest component nesting followed.
const MAX_COMPONENT_DEPTH: usize = 32;
/// Most component references followed for one glyph, nesting included.
const MAX_COMPONENT_VISITS: usize = 100_000;
/// Most points one converted glyph may have.
const MAX_GLYPH_POINTS: usize = 65_535;
/// Most contours one glyph may have.
const MAX_CONTOURS: usize = 32_767;
/// Most points a conversion to CFF may produce over all its glyphs, a component's points
/// counted once for every use. All of them are kept until the new outline table is written (in
/// a CFF path operator of up to 56 bytes each), and a glyph being converted is held in two more
/// forms, so a 32-bit target (wasm) gets half.
pub(super) const MAX_CONVERT_POINTS: usize = if usize::BITS >= 64 { 16 << 20 } else { 8 << 20 };
/// Most points a conversion to TrueType may produce over all its glyphs. Each glyph is encoded
/// as soon as it is converted, so this bounds the work, not what is held: a full CJK font
/// (65,535 glyphs) makes about 10.6M.
pub(super) const MAX_TRUETYPE_POINTS: usize = 64 << 20;
/// Most component references one conversion follows over all its glyphs, nesting included.
const MAX_CONVERT_VISITS: usize = 4 << 20;

/// What one conversion may still produce, shared by all its glyphs.
pub(super) struct Budget {
    points: usize,
    visits: usize,
    max_points: usize,
    what: &'static str,
}

impl Budget {
    /// For a conversion to CFF ([`MAX_CONVERT_POINTS`]).
    pub(super) fn new() -> Self {
        Budget {
            points: MAX_CONVERT_POINTS,
            visits: MAX_CONVERT_VISITS,
            max_points: MAX_CONVERT_POINTS,
            what: "the points the converted glyphs hold together",
        }
    }

    /// For a conversion to TrueType ([`MAX_TRUETYPE_POINTS`]).
    pub(super) fn truetype() -> Self {
        Budget {
            points: MAX_TRUETYPE_POINTS,
            visits: MAX_CONVERT_VISITS,
            max_points: MAX_TRUETYPE_POINTS,
            what: "the points the converted glyphs make together",
        }
    }

    /// Charges `n` points produced.
    pub(super) fn points(&mut self, n: usize) -> Result<()> {
        self.points = self.points.checked_sub(n).ok_or(TablesError::TooLarge { what: self.what, limit: self.max_points })?;
        #[cfg(test)]
        POINTS_PEAK.with(|p| p.set(p.get().max(self.max_points.saturating_sub(self.points))));
        Ok(())
    }

    /// Charges `n` component references followed.
    fn visits(&mut self, n: usize) -> Result<()> {
        self.visits = self
            .visits
            .checked_sub(n)
            .ok_or(TablesError::TooLarge { what: "the components the converted glyphs use together", limit: MAX_CONVERT_VISITS })?;
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// The most points any [`Budget`] on this thread has had charged, for tests that measure.
    pub(super) static POINTS_PEAK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What decomposing one `glyf` glyph produces: its points and the glyph references followed
/// (itself included), each component counted once for every use.
#[derive(Clone, Copy, Debug)]
struct Cost {
    points: usize,
    visits: usize,
}

pub(crate) type P = (f64, f64);

/// One segment of a closed contour, by its end point.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Seg {
    Line(P),
    /// Off-curve points, with implied on-curve points between them, and the end point.
    Quad(Vec<P>, P),
    Cubic(P, P, P),
}

impl Seg {
    fn end(&self) -> P {
        match self {
            Seg::Line(p) | Seg::Quad(_, p) | Seg::Cubic(_, _, p) => *p,
        }
    }
}

/// A closed contour: its last segment ends at `start` (an implied closing line is explicit).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Contour {
    pub start: P,
    pub segs: Vec<Seg>,
}

impl Contour {
    /// The same contour drawn the other way round, still starting at `start`.
    pub(super) fn translated(&self, dx: f64, dy: f64) -> Contour {
        let t = |p: &P| (p.0 + dx, p.1 + dy);
        let segs = self
            .segs
            .iter()
            .map(|s| match s {
                Seg::Line(p) => Seg::Line(t(p)),
                Seg::Quad(offs, p) => Seg::Quad(offs.iter().map(t).collect(), t(p)),
                Seg::Cubic(c1, c2, p) => Seg::Cubic(t(c1), t(c2), t(p)),
            })
            .collect();
        Contour { start: t(&self.start), segs }
    }

    pub(crate) fn reversed(&self) -> Contour {
        let mut segs = Vec::with_capacity(self.segs.len());
        for i in (0..self.segs.len()).rev() {
            let from = if i == 0 { self.start } else { self.segs.get(i - 1).map_or(self.start, Seg::end) };
            segs.push(match self.segs.get(i) {
                Some(Seg::Line(_)) | None => Seg::Line(from),
                Some(Seg::Quad(offs, _)) => Seg::Quad(offs.iter().rev().copied().collect(), from),
                Some(Seg::Cubic(c1, c2, _)) => Seg::Cubic(*c2, *c1, from),
            });
        }
        Contour { start: self.start, segs }
    }
}

/// Collects drawn contours, closing each one explicitly.
#[derive(Default)]
pub(super) struct Collect {
    contours: Vec<Contour>,
    open: Option<Contour>,
    points: usize,
    too_many: bool,
}

impl Collect {
    fn finish_open(&mut self) {
        if let Some(mut c) = self.open.take() {
            if c.segs.is_empty() {
                return;
            }
            if c.segs.last().map(Seg::end) != Some(c.start) {
                c.segs.push(Seg::Line(c.start));
            }
            self.contours.push(c);
        }
    }

    fn push(&mut self, s: Seg) {
        self.points += 3;
        if self.points > MAX_GLYPH_POINTS * 4 {
            self.too_many = true;
            return;
        }
        match &mut self.open {
            Some(c) => c.segs.push(s),
            None => self.open = Some(Contour { start: (0.0, 0.0), segs: vec![s] }),
        }
    }

    pub(super) fn done(mut self) -> Result<Vec<Contour>> {
        self.finish_open();
        if self.too_many || self.contours.len() > MAX_CONTOURS {
            return Err(TablesError::TooLarge { what: "the points of one glyph", limit: MAX_GLYPH_POINTS });
        }
        Ok(self.contours)
    }
}

impl Collect {
    pub(super) fn move_to(&mut self, p: P) {
        self.finish_open();
        self.open = Some(Contour { start: p, segs: Vec::new() });
    }
    pub(super) fn line_to(&mut self, p: P) {
        self.push(Seg::Line(p));
    }
    pub(super) fn curve_to(&mut self, c1: P, c2: P, p: P) {
        self.push(Seg::Cubic(c1, c2, p));
    }
    pub(super) fn close(&mut self) {
        self.finish_open();
    }
}

/// The glyphs of a `CFF ` or static `CFF2` table as contours.
pub(super) struct PsOutlines<'a> {
    font: CffFontRef<'a>,
    programs: Programs,
    /// Charset names to glyphs, the first glyph of a repeated name; filled on first use.
    by_name: std::cell::OnceCell<HashMap<String, usize>>,
    /// The contours of each `seac` part, drawn once: many accented glyphs name the same part,
    /// and a part's hints cost drawing time without adding contours. At most one entry per
    /// standard-encoding code.
    parts: std::cell::RefCell<HashMap<usize, std::rc::Rc<Vec<Contour>>>>,
}

impl<'a> PsOutlines<'a> {
    pub(super) fn new(data: &'a [u8], upem: u16) -> Result<Self> {
        let font = CffFontRef::new(data, 0, Some(i32::from(upem))).map_err(|e| TablesError::Invalid(format!("the CFF table cannot be read: {e}")))?;
        let programs = Programs::read(data, data.first() == Some(&2))?;
        if programs.len() != font.num_glyphs() as usize {
            return Err(TablesError::Invalid("the CFF table counts its glyphs two ways".into()));
        }
        Ok(PsOutlines { font, programs, by_name: std::cell::OnceCell::new(), parts: std::cell::RefCell::default() })
    }

    pub(super) fn glyph_count(&self) -> usize {
        self.programs.len()
    }

    #[cfg(test)]
    pub(super) fn is_cid(&self) -> bool {
        self.font.is_cid()
    }

    /// The charset name of a glyph (`cidNNNNN` in a CID-keyed font); `None` without a charset.
    pub(super) fn name(&self, gid: usize) -> Option<String> {
        let charset = self.font.charset()?;
        let sid = charset.string_id(GlyphId::new(u32::try_from(gid).ok()?))?;
        if self.font.is_cid() {
            return Some(format!("cid{:05}", sid.to_u16()));
        }
        self.font.string(sid).map(|b| b.iter().map(|&c| char::from(c)).collect())
    }

    /// The glyph's own contours and, for a `seac` glyph, its base and accent.
    pub(super) fn draw(&self, gid: usize) -> Result<Drawing> {
        self.programs.draw(gid)
    }

    /// The glyph's contours, a `seac` base and accent drawn in.
    pub(super) fn contours(&mut self, gid: usize) -> Result<Vec<Contour>> {
        self.flattened(gid)
    }

    /// Contours with the `seac` base and accent drawn in (they may not be accented themselves).
    pub(super) fn flattened(&self, gid: usize) -> Result<Vec<Contour>> {
        let drawing = self.draw(gid)?;
        let mut contours = drawing.contours;
        if let Some(seac) = drawing.seac {
            for (code, dx, dy) in [(seac.base, 0.0, 0.0), (seac.accent, seac.adx, seac.ady)] {
                let Some(part) = self.standard_glyph(code) else { continue };
                let Some(inner) = self.part_contours(part)? else {
                    return Err(TablesError::Invalid(format!("glyph {gid}: a seac part is itself accented")));
                };
                contours.extend(inner.iter().map(|c| c.translated(dx, dy)));
            }
        }
        Ok(contours)
    }

    /// The contours of a `seac` part, from the cache when drawn before; `None` when the part
    /// is itself accented.
    fn part_contours(&self, part: usize) -> Result<Option<std::rc::Rc<Vec<Contour>>>> {
        if let Some(c) = self.parts.try_borrow().ok().and_then(|m| m.get(&part).cloned()) {
            return Ok(Some(c));
        }
        let inner = self.draw(part)?;
        if inner.seac.is_some() {
            return Ok(None);
        }
        let contours = std::rc::Rc::new(inner.contours);
        if let Ok(mut m) = self.parts.try_borrow_mut() {
            m.insert(part, contours.clone());
        }
        Ok(Some(contours))
    }

    /// Whether the glyph is drawn with `seac`.
    pub(super) fn is_accented(&self, gid: usize) -> bool {
        self.programs.is_accented(gid)
    }

    /// The glyph a `seac` names by its standard-encoding code, found by charset name.
    pub(super) fn standard_glyph(&self, code: u8) -> Option<usize> {
        let name = read_fonts::ps::encoding::PredefinedEncoding::Standard.name(code);
        let by_name = self.by_name.get_or_init(|| {
            let mut map = HashMap::new();
            for gid in 0..self.glyph_count() {
                if let Some(n) = self.name(gid) {
                    map.entry(n).or_insert(gid);
                }
            }
            map
        });
        by_name.get(name).copied()
    }
}

/// The glyphs of a `glyf` table as contours, components decomposed.
pub(super) struct GlyfOutlines<'a> {
    glyf: Glyf<'a>,
    loca: Loca<'a>,
    count: usize,
    /// The cost of each glyph, once known.
    costs: std::cell::RefCell<Vec<Option<Cost>>>,
}

impl<'a> GlyfOutlines<'a> {
    pub(super) fn new(glyf: &'a [u8], loca: &'a [u8], long_loca: bool, count: usize) -> Result<Self> {
        let bad = |e: read_fonts::ReadError| TablesError::Invalid(format!("the glyf or loca table cannot be read: {e}"));
        let glyf = Glyf::read(FontData::new(glyf)).map_err(bad)?;
        let loca = Loca::read_with_args(FontData::new(loca), long_loca).map_err(bad)?;
        Ok(GlyfOutlines { glyf, loca, count, costs: std::cell::RefCell::new(vec![None; count]) })
    }

    /// The `xMin` a glyph's header states; `None` for a glyph without data.
    pub(super) fn x_min(&self, gid: usize) -> Option<i16> {
        match self.loca.get(GlyphId::new(u32::try_from(gid).ok()?), &self.glyf)? {
            LocaGlyph::Glyph(g) => Some(g.x_min()),
            LocaGlyph::Empty => None,
        }
    }

    /// The glyph's contours, their points charged to `budget`; `instructions` is set when it
    /// (or a component) has a glyph program.
    pub(super) fn contours(&self, gid: usize, instructions: &mut bool, budget: &mut Budget) -> Result<Vec<Contour>> {
        // The glyph is charged before it is decomposed, so a chain of components that multiplies
        // its points is refused before any of them are made.
        let cost = self.cost(gid, 0)?;
        budget.points(cost.points)?;
        budget.visits(cost.visits)?;
        let mut out = Vec::new();
        let mut visits = 0usize;
        self.add(gid, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 0, &mut visits, &mut out, instructions)?;
        Ok(out)
    }

    /// What decomposing glyph `gid` produces, by the point counts the glyphs state.
    fn cost(&self, gid: usize, depth: usize) -> Result<Cost> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err(TablesError::Invalid(format!("glyph {gid}: components nest deeper than {MAX_COMPONENT_DEPTH} (or form a cycle)")));
        }
        if let Some(known) = self.costs.borrow().get(gid).copied().flatten() {
            return Ok(known);
        }
        if gid >= self.count {
            return Err(TablesError::Invalid(format!("a component names glyph {gid}, beyond the glyph count")));
        }
        let cost = match self.loca.get(GlyphId::new(gid as u32), &self.glyf) {
            Some(LocaGlyph::Empty) => Cost { points: 0, visits: 1 },
            Some(LocaGlyph::Glyph(Glyph::Simple(s))) => Cost { points: s.num_points(), visits: 1 },
            Some(LocaGlyph::Glyph(Glyph::Composite(c))) => {
                let mut total = Cost { points: 0, visits: 1 };
                for comp in c.components() {
                    let part = self.cost(usize::from(comp.glyph.to_u16()), depth + 1)?;
                    total = Cost { points: total.points.saturating_add(part.points), visits: total.visits.saturating_add(part.visits) };
                }
                total
            }
            None => return Err(TablesError::Invalid(format!("glyph {gid} cannot be read"))),
        };
        if let Some(slot) = self.costs.borrow_mut().get_mut(gid) {
            *slot = Some(cost);
        }
        Ok(cost)
    }

    fn add(&self, gid: usize, m: [f64; 6], depth: usize, visits: &mut usize, out: &mut Vec<Contour>, instructions: &mut bool) -> Result<()> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err(TablesError::Invalid(format!("glyph {gid}: components nest deeper than {MAX_COMPONENT_DEPTH} (or form a cycle)")));
        }
        *visits += 1;
        if *visits > MAX_COMPONENT_VISITS {
            return Err(TablesError::TooLarge { what: "the components one glyph uses", limit: MAX_COMPONENT_VISITS });
        }
        if gid >= self.count {
            return Err(TablesError::Invalid(format!("a component names glyph {gid}, beyond the glyph count")));
        }
        let id = GlyphId::new(gid as u32);
        let glyph = match self.loca.get(id, &self.glyf) {
            Some(LocaGlyph::Empty) => return Ok(()),
            Some(LocaGlyph::Glyph(g)) => g,
            None => return Err(TablesError::Invalid(format!("glyph {gid} cannot be read"))),
        };
        let at = |x: f64, y: f64| -> P { (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]) };
        match glyph {
            Glyph::Simple(s) => {
                if s.instruction_length() > 0 {
                    *instructions = true;
                }
                let points: Vec<_> = s.points().collect();
                if points.len() > MAX_GLYPH_POINTS {
                    return Err(TablesError::TooLarge { what: "the points of one glyph", limit: MAX_GLYPH_POINTS });
                }
                let mut start = 0usize;
                for end in s.end_pts_of_contours() {
                    let end = usize::from(end.get());
                    let pts = points.get(start..=end).ok_or_else(|| TablesError::Invalid(format!("glyph {gid}: a contour ends past its points")))?;
                    let pts: Vec<(P, bool)> = pts.iter().map(|p| (at(f64::from(p.x), f64::from(p.y)), p.on_curve)).collect();
                    if let Some(c) = truetype_contour(&pts) {
                        out.push(c);
                    }
                    start = end + 1;
                }
                if out.len() > MAX_CONTOURS {
                    return Err(TablesError::TooLarge { what: "the contours of one glyph", limit: MAX_CONTOURS });
                }
            }
            Glyph::Composite(c) => {
                if c.instructions().is_some_and(|i| !i.is_empty()) {
                    *instructions = true;
                }
                for comp in c.components() {
                    let Anchor::Offset { x, y } = comp.anchor else {
                        return Err(TablesError::Unsupported(format!("glyph {gid}: a component placed by point numbers")));
                    };
                    let t = comp.transform;
                    let (a, b, cc, d) = (t.xx.to_f64(), t.yx.to_f64(), t.xy.to_f64(), t.yy.to_f64());
                    let (dx, dy) = if comp.flags.contains(CompositeGlyphFlags::SCALED_COMPONENT_OFFSET) {
                        (a * f64::from(x) + cc * f64::from(y), b * f64::from(x) + d * f64::from(y))
                    } else {
                        (f64::from(x), f64::from(y))
                    };
                    // The component's matrix first, then this glyph's.
                    let inner = [a, b, cc, d, dx, dy];
                    let combined = [
                        m[0] * inner[0] + m[2] * inner[1],
                        m[1] * inner[0] + m[3] * inner[1],
                        m[0] * inner[2] + m[2] * inner[3],
                        m[1] * inner[2] + m[3] * inner[3],
                        m[0] * inner[4] + m[2] * inner[5] + m[4],
                        m[1] * inner[4] + m[3] * inner[5] + m[5],
                    ];
                    self.add(usize::from(comp.glyph.to_u16()), combined, depth + 1, visits, out, instructions)?;
                }
            }
        }
        Ok(())
    }
}

/// A TrueType contour from its points: implied on-curve points between off-curve ones, a
/// contour of off-curve points only starting at the midpoint of its last and first points.
fn truetype_contour(pts: &[(P, bool)]) -> Option<Contour> {
    let mid = |a: P, b: P| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let first_on = pts.iter().position(|p| p.1);
    let (start, order): (P, Vec<(P, bool)>) = match first_on {
        Some(k) => {
            let rotated: Vec<(P, bool)> = pts.iter().skip(k + 1).chain(pts.iter().take(k)).copied().collect();
            (pts.get(k)?.0, rotated)
        }
        None => {
            let (last, first) = (pts.last()?.0, pts.first()?.0);
            (mid(last, first), pts.to_vec())
        }
    };
    let mut segs = Vec::new();
    let mut offs: Vec<P> = Vec::new();
    for (p, on) in order {
        if on {
            segs.push(if offs.is_empty() { Seg::Line(p) } else { Seg::Quad(std::mem::take(&mut offs), p) });
        } else {
            offs.push(p);
        }
    }
    if !offs.is_empty() {
        segs.push(Seg::Quad(offs, start));
    } else if segs.last().map(Seg::end) != Some(start) {
        segs.push(Seg::Line(start));
    }
    // A one-point contour draws nothing.
    if segs.iter().all(|s| matches!(s, Seg::Line(p) if *p == start)) {
        return None;
    }
    Some(Contour { start, segs })
}

/// Cubic curves as quadratic splines within `max_error` font units (fontTools' cu2qu).
pub(super) fn to_quadratic(c: &Contour, max_error: f64) -> Result<Contour> {
    let mut segs = Vec::with_capacity(c.segs.len());
    let mut from = c.start;
    for s in &c.segs {
        match s {
            Seg::Cubic(c1, c2, p) => {
                let cubic = CubicBez::new(pt(from), pt(*c1), pt(*c2), pt(*p));
                let quads = curves_to_quadratic(&[cubic], max_error)
                    .and_then(|mut v| v.pop())
                    .ok_or_else(|| TablesError::Invalid("a curve cannot be approximated within the error bound".into()))?;
                segs.push(Seg::Quad(quads.iter().map(|q| (q.p1.x, q.p1.y)).collect(), *p));
            }
            other => segs.push(other.clone()),
        }
        from = s.end();
    }
    Ok(Contour { start: c.start, segs })
}

/// How quadratic curves become cubic ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QuadToCubic {
    /// Runs of quadratic curves merged into fewer cubic curves within the error bound
    /// (fontTools' qu2cu, every curve cubic).
    #[default]
    Optimized,
    /// Each quadratic curve raised to the one cubic curve that draws it exactly.
    Exact,
}

/// Quadratic splines as cubic curves.
pub(super) fn to_cubic(c: &Contour, how: QuadToCubic, max_error: f64) -> Result<Contour> {
    let mid = |a: P, b: P| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let mut segs: Vec<Seg> = Vec::with_capacity(c.segs.len());
    let mut from = c.start;
    let mut run: Vec<Vec<(f64, f64)>> = Vec::new();
    let flush = |run: &mut Vec<Vec<(f64, f64)>>, segs: &mut Vec<Seg>| -> Result<()> {
        if run.is_empty() {
            return Ok(());
        }
        let curves = quadratic_to_curves(run, max_error, true).map_err(|e| TablesError::Invalid(format!("quadratic to cubic: {e}")))?;
        for curve in curves {
            segs.push(match curve {
                Curve::Cubic([_, c1, c2, p]) => Seg::Cubic(c1, c2, p),
                Curve::Quadratic([p0, q, p]) => raise(p0, q, p),
            });
        }
        run.clear();
        Ok(())
    };
    for s in &c.segs {
        match (s, how) {
            (Seg::Quad(offs, p), QuadToCubic::Exact) => {
                let mut a = from;
                for (i, q) in offs.iter().enumerate() {
                    let b = match offs.get(i + 1) {
                        Some(next) => mid(*q, *next),
                        None => *p,
                    };
                    segs.push(raise(a, *q, b));
                    a = b;
                }
            }
            (Seg::Quad(offs, p), QuadToCubic::Optimized) => {
                let mut spline = Vec::with_capacity(offs.len() + 2);
                spline.push(from);
                spline.extend_from_slice(offs);
                spline.push(*p);
                run.push(spline);
            }
            (other, _) => {
                flush(&mut run, &mut segs)?;
                segs.push(other.clone());
            }
        }
        from = s.end();
    }
    flush(&mut run, &mut segs)?;
    Ok(Contour { start: c.start, segs })
}

/// The cubic curve that draws the quadratic curve `p0 q p1`.
fn raise(p0: P, q: P, p1: P) -> Seg {
    let c1 = (p0.0 + 2.0 / 3.0 * (q.0 - p0.0), p0.1 + 2.0 / 3.0 * (q.1 - p0.1));
    let c2 = (p1.0 + 2.0 / 3.0 * (q.0 - p1.0), p1.1 + 2.0 / 3.0 * (q.1 - p1.1));
    Seg::Cubic(c1, c2, p1)
}

fn pt(p: P) -> Point {
    Point::new(p.0, p.1)
}

/// The drawing steps of cubic contours for the CFF writer: the implied closing line left out.
pub(crate) fn path_ops(contours: &[Contour]) -> Result<Vec<fontcraft_cff::PathOp>> {
    use fontcraft_cff::PathOp;
    let mut ops = Vec::new();
    for c in contours {
        ops.push(PathOp::MoveTo(c.start));
        let n = c.segs.len();
        for (i, s) in c.segs.iter().enumerate() {
            match s {
                Seg::Line(p) if i + 1 == n && *p == c.start => {}
                Seg::Line(p) => ops.push(PathOp::LineTo(*p)),
                Seg::Cubic(c1, c2, p) => ops.push(PathOp::CurveTo(*c1, *c2, *p)),
                Seg::Quad(..) => return Err(TablesError::Invalid("a quadratic curve reached the CFF writer".into())),
            }
        }
    }
    Ok(ops)
}

/// The bounds of the drawn contours (curve extremes, not control points).
pub(super) fn drawn_bounds(contours: &[Contour]) -> Option<[f64; 4]> {
    let mut path = BezPath::new();
    for c in contours {
        path.move_to(pt(c.start));
        for s in &c.segs {
            match s {
                Seg::Line(p) => path.line_to(pt(*p)),
                Seg::Cubic(c1, c2, p) => path.curve_to(pt(*c1), pt(*c2), pt(*p)),
                Seg::Quad(offs, p) => {
                    for q in quads(offs, *p) {
                        path.quad_to(pt(q.0), pt(q.1));
                    }
                }
            }
        }
        path.close_path();
    }
    if contours.is_empty() {
        return None;
    }
    let r = path.bounding_box();
    Some([r.x0, r.y0, r.x1, r.y1]).filter(|b| b.iter().all(|v| v.is_finite()))
}

/// The single quadratic curves of a spline: `(control, end)` pairs.
fn quads(offs: &[P], end: P) -> Vec<(P, P)> {
    let mid = |a: P, b: P| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    offs.iter().enumerate().map(|(i, q)| (*q, offs.get(i + 1).map_or(end, |n| mid(*q, *n)))).collect()
}

/// A simple glyph's points, rounded: `(x, y, on-curve)` per contour.
pub(super) type TtContour = Vec<(i32, i32, bool)>;

/// The glyf points of quadratic contours, as `TTGlyphPen` writes them: the implied closing line
/// left out, and a last point equal to the first dropped.
pub(super) fn truetype_points(contours: &[Contour]) -> Result<Vec<TtContour>> {
    let round = |v: f64| -> Result<i32> {
        let r = (v + 0.5).floor();
        if !(r.is_finite() && (-32_768.0..=32_767.0).contains(&r)) {
            return Err(TablesError::Unsupported(format!("a coordinate of {v} does not fit in glyf")));
        }
        Ok(r as i32)
    };
    let mut out = Vec::with_capacity(contours.len());
    for c in contours {
        let mut pts: Vec<(P, bool)> = vec![(c.start, true)];
        let n = c.segs.len();
        for (i, s) in c.segs.iter().enumerate() {
            match s {
                Seg::Line(p) if i + 1 == n && *p == c.start => {}
                Seg::Line(p) => pts.push((*p, true)),
                Seg::Quad(offs, p) => {
                    pts.extend(offs.iter().map(|q| (*q, false)));
                    pts.push((*p, true));
                }
                Seg::Cubic(..) => return Err(TablesError::Invalid("a cubic curve reached the glyf writer".into())),
            }
        }
        if pts.len() > 1 && pts.last().map(|p| p.0) == Some(c.start) {
            pts.pop();
        }
        if pts.len() < 2 {
            continue;
        }
        out.push(pts.iter().map(|(p, on)| Ok((round(p.0)?, round(p.1)?, *on))).collect::<Result<Vec<_>>>()?);
    }
    Ok(out)
}

/// A simple `glyf` glyph without instructions, and its bounds (`None` for an empty glyph,
/// which has no data).
pub(super) fn encode_simple(contours: &[TtContour]) -> Result<(Vec<u8>, Option<[i16; 4]>)> {
    let points: usize = contours.iter().map(Vec::len).sum();
    if points == 0 {
        return Ok((Vec::new(), None));
    }
    if points > MAX_GLYPH_POINTS || contours.len() > MAX_CONTOURS {
        return Err(TablesError::TooLarge { what: "the points of one glyph", limit: MAX_GLYPH_POINTS });
    }
    let all = || contours.iter().flatten();
    let fit = |v: i32| i16::try_from(v).map_err(|_| TablesError::Unsupported(format!("a coordinate of {v} does not fit in glyf")));
    let bbox = [
        fit(all().map(|p| p.0).min().unwrap_or(0))?,
        fit(all().map(|p| p.1).min().unwrap_or(0))?,
        fit(all().map(|p| p.0).max().unwrap_or(0))?,
        fit(all().map(|p| p.1).max().unwrap_or(0))?,
    ];
    let mut out = Vec::with_capacity(12 + points * 5);
    out.extend_from_slice(&(contours.len() as i16).to_be_bytes());
    for v in bbox {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut end = 0usize;
    for c in contours {
        end += c.len();
        out.extend_from_slice(&((end - 1) as u16).to_be_bytes());
    }
    out.extend_from_slice(&0u16.to_be_bytes());
    let mut flags = Vec::with_capacity(points);
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let (mut px, mut py) = (0i32, 0i32);
    for &(x, y, on) in all() {
        let mut f = u8::from(on);
        let (dx, dy) = (x - px, y - py);
        (px, py) = (x, y);
        if dx == 0 {
            f |= 0x10;
        } else if (-255..=255).contains(&dx) {
            f |= 0x02;
            if dx > 0 {
                f |= 0x10;
            }
            xs.push(dx.unsigned_abs() as u8);
        } else {
            xs.extend_from_slice(&fit(dx)?.to_be_bytes());
        }
        if dy == 0 {
            f |= 0x20;
        } else if (-255..=255).contains(&dy) {
            f |= 0x04;
            if dy > 0 {
                f |= 0x20;
            }
            ys.push(dy.unsigned_abs() as u8);
        } else {
            ys.extend_from_slice(&fit(dy)?.to_be_bytes());
        }
        flags.push(f);
    }
    let mut i = 0usize;
    while let Some(&f) = flags.get(i) {
        let mut repeat = 0usize;
        while repeat < 255 && flags.get(i + 1 + repeat) == Some(&f) {
            repeat += 1;
        }
        if repeat > 0 {
            out.push(f | 0x08);
            out.push(repeat as u8);
        } else {
            out.push(f);
        }
        i += repeat + 1;
    }
    out.extend_from_slice(&xs);
    out.extend_from_slice(&ys);
    Ok((out, Some(bbox)))
}

/// A composite `glyf` glyph of components placed by offset, as fontTools' `TTGlyphPen` writes
/// them: offsets rounded to the grid, no instructions.
pub(super) fn encode_composite(components: &[(u16, i32, i32)], bbox: [i16; 4]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(10 + components.len() * 8);
    out.extend_from_slice(&(-1i16).to_be_bytes());
    for v in bbox {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for (i, &(gid, x, y)) in components.iter().enumerate() {
        // ARGS_ARE_XY_VALUES and ROUND_XY_TO_GRID; MORE_COMPONENTS but on the last.
        let mut flags: u16 = 0x0002 | 0x0004;
        if i + 1 < components.len() {
            flags |= 0x0020;
        }
        let small = |v: i32| (-128..=127).contains(&v);
        let args: Vec<u8> = if small(x) && small(y) {
            vec![x as i8 as u8, y as i8 as u8]
        } else {
            flags |= 0x0001;
            let fit = |v: i32| i16::try_from(v).map_err(|_| TablesError::Unsupported(format!("a component offset of {v} does not fit in glyf")));
            [fit(x)?.to_be_bytes(), fit(y)?.to_be_bytes()].concat()
        };
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(&gid.to_be_bytes());
        out.extend_from_slice(&args);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Contour {
        // Counter-clockwise, as CFF draws an outer contour.
        Contour { start: (0.0, 0.0), segs: vec![Seg::Line((100.0, 0.0)), Seg::Line((100.0, 100.0)), Seg::Line((0.0, 100.0)), Seg::Line((0.0, 0.0))] }
    }

    #[test]
    fn reversing_keeps_the_start_point_and_drops_the_implied_closing_line() {
        let r = square().reversed();
        assert_eq!(r.start, (0.0, 0.0));
        assert_eq!(truetype_points(&[r]).unwrap(), vec![vec![(0, 0, true), (0, 100, true), (100, 100, true), (100, 0, true)]]);
        let curved = Contour { start: (0.0, 0.0), segs: vec![Seg::Line((100.0, 0.0)), Seg::Cubic((100.0, 50.0), (50.0, 100.0), (0.0, 0.0))] };
        let r = curved.reversed();
        assert_eq!(r.segs, vec![Seg::Cubic((50.0, 100.0), (100.0, 50.0), (100.0, 0.0)), Seg::Line((0.0, 0.0))]);
    }

    #[test]
    fn exact_raising_draws_the_same_curve() {
        let c = Contour { start: (0.0, 0.0), segs: vec![Seg::Quad(vec![(50.0, 100.0), (150.0, 100.0)], (200.0, 0.0)), Seg::Line((0.0, 0.0))] };
        let cubic = to_cubic(&c, QuadToCubic::Exact, 1.0).unwrap();
        assert_eq!(cubic.segs.len(), 3);
        // The implied on-curve point (100, 100) is kept as an on-curve point.
        assert_eq!(cubic.segs.first().map(Seg::end), Some((100.0, 100.0)));
        let merged = to_cubic(&c, QuadToCubic::Optimized, 1.0).unwrap();
        assert_eq!(merged.segs.len(), 2, "the two quadratic pieces of one spline become one cubic");
        let b1 = drawn_bounds(&[c]).unwrap();
        let b2 = drawn_bounds(&[merged]).unwrap();
        for (a, b) in b1.iter().zip(&b2) {
            assert!((a - b).abs() < 1.0, "{b1:?} vs {b2:?}");
        }
    }

    #[test]
    fn simple_glyphs_round_trip_through_the_glyf_reader() {
        let contour = vec![(0, 0, true), (0, 700, true), (300, 900, false), (600, 700, true), (600, 0, true)];
        let (bytes, bbox) = encode_simple(std::slice::from_ref(&contour)).unwrap();
        assert_eq!(bbox, Some([0, 0, 600, 900]));
        let glyph = read_fonts::tables::glyf::SimpleGlyph::read(FontData::new(&bytes)).unwrap();
        let read: Vec<(i32, i32, bool)> = glyph.points().map(|p| (i32::from(p.x), i32::from(p.y), p.on_curve)).collect();
        assert_eq!(read, contour);
    }
}
