//! Stem detection (M13.24; M13.19, M13.34, M13.35 and M13.36 use it).
//!
//! A stem is a pair of roughly parallel, opposite-facing straight or nearly straight stretches of
//! outline with ink between them: the vertical bars of `H`, the horizontal bar of `e`, the
//! diagonals of `V`. For curves the detector also reports the thickest horizontal and vertical
//! cross-sections (the sides of `o`), flagged as round.
//!
//! How stems are found:
//!
//! - **Straight stems.** Every line, and every curve whose control points lie within
//!   [`STRAIGHT_TOLERANCE`] of its chord, is an edge. Two edges within `angle_tolerance` of
//!   parallel form a stem when their stretches overlap by at least `min_length`, the distance
//!   between them is in range and no longer than the overlap (so the top and bottom of a tall bar
//!   are not a stem), the area between them is ink at nine sample points, and just outside each
//!   edge it is not. Edges of one stem split by a junction (the right side of `H`'s left bar,
//!   cut by the crossbar) give one stem.
//! - **Round stems.** At each point of a curve where the tangent is vertical (or horizontal), a
//!   ray goes into the ink at right angles to the tangent. The outline it meets first is the far
//!   side, if its tangent there is within [`ROUND_PARTNER_DEGREES`] of the near side's. The two
//!   measurements of one bowl side (from outside in and from inside out) give one stem, the
//!   thicker. A point where a curve continues a straight edge belongs to that edge's stem.
//!
//! Ink is the nonzero fill of the closed contours. Open contours have no inside, so they take no
//! part. The work is bounded: at most [`MAX_EDGES`] edges are paired (the longest), and the ink
//! tests stop after [`MAX_WORK`] segment visits, returning the stems found so far. A caller that
//! runs many detections (a filter over many glyphs) also passes its own budget, which every
//! detection draws from.

use std::collections::{HashMap, HashSet};

use fontcraft_geom::{BezPath, CubicBez, Line, ParamCurve, ParamCurveExtrema, PathEl, PathSeg, Point, QuadBez, Shape, Vec2};
use kurbo::ParamCurveDeriv;

/// Most stems reported for one outline.
pub const MAX_STEMS: usize = 10_000;
/// Most straight edges paired with each other; beyond it the longest are kept.
pub const MAX_EDGES: usize = 4_096;
/// Most segment visits the ink tests and rays of one detection make.
pub const MAX_WORK: usize = 50_000_000;
/// A curve whose control points lie this close to its chord, in font units, is an edge.
pub const STRAIGHT_TOLERANCE: f64 = 1.0;
/// The far side of a round stem may lean this far from the near side, in degrees.
pub const ROUND_PARTNER_DEGREES: f64 = 30.0;
/// How far beside an edge the ink tests look, in font units.
const PROBE: f64 = 0.5;
/// Two widths this close, in font units, are the same stem's.
const SAME_WIDTH: f64 = 0.5;

/// Which way a stem runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StemDirection {
    /// Runs up and down: its width is measured horizontally.
    Vertical,
    /// Runs left and right: its width is measured vertically.
    Horizontal,
    /// Any other angle: its width is measured at right angles to it.
    Diagonal,
}

/// One stem.
#[derive(Clone, Debug, PartialEq)]
pub struct Stem {
    pub direction: StemDirection,
    /// The two edges, as the parts of the outline that face each other; `first` is the left (or
    /// lower) edge. A round stem's edges have no length: each is the point where the width was
    /// measured.
    pub first: Line,
    pub second: Line,
    /// The distance between the edges, in font units.
    pub width: f64,
    /// Measured across curves (the sides of `o`), not between straight edges.
    pub round: bool,
}

/// What [`detect_stems`] reports.
#[derive(Clone, Debug, PartialEq)]
pub struct StemOptions {
    /// Narrowest and widest stem, in font units.
    pub min_width: f64,
    pub max_width: f64,
    /// Shortest overlap of the two edges, in font units.
    pub min_length: f64,
    /// How far from parallel two edges may be, in degrees.
    pub angle_tolerance: f64,
    /// Report only stems running this way; `None`: every direction.
    pub direction: Option<StemDirection>,
}

impl Default for StemOptions {
    fn default() -> Self {
        Self { min_width: 1.0, max_width: 1_000.0, min_length: 10.0, angle_tolerance: 5.0, direction: None }
    }
}

/// The stems of `outline` (closed contours, resolved), widest overlap first, at most
/// [`MAX_STEMS`]. Round stems come after the straight ones, thickest first.
pub fn detect_stems(outline: &BezPath, options: &StemOptions) -> Vec<Stem> {
    let parts = parts_of_bez(outline);
    find(&parts, outline, options).into_iter().map(|f| f.stem).collect()
}

/// One Bézier piece of an outline, with the outline's nodes at its ends when the caller knows
/// them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Part {
    pub seg: PathSeg,
    pub contour: usize,
    /// The node the piece starts and ends at; `None` for a point between the pieces of a
    /// quadratic spline, or when the outline came as a plain path.
    pub start: Option<usize>,
    pub end: Option<usize>,
}

/// Where one side of a stem lies on the outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Side {
    /// A whole straight part.
    Edge(usize),
    /// A point of a curved part, at parameter `t`.
    At(usize, f64),
}

/// A stem and the parts it was measured between: `sides[0]` holds `first`'s.
#[derive(Clone, Debug)]
pub(crate) struct Found {
    pub stem: Stem,
    pub sides: [Vec<Side>; 2],
    /// The length over which the edges face each other; zero for a round stem.
    pub overlap: f64,
}

/// The pieces of the closed subpaths of `path`; nothing when a coordinate is not finite.
fn parts_of_bez(path: &BezPath) -> Vec<Part> {
    let mut out = Vec::new();
    let mut contour = 0usize;
    let mut started = false;
    let (mut first, mut last) = (Point::ZERO, Point::ZERO);
    let mut pending: Vec<PathSeg> = Vec::new();
    for el in path.elements() {
        if out.len() + pending.len() >= crate::MAX_SEGMENTS {
            break;
        }
        let seg = match *el {
            PathEl::MoveTo(p) => {
                // An open subpath has no inside: its pieces are dropped.
                pending.clear();
                if started {
                    contour += 1;
                }
                started = true;
                first = p;
                last = p;
                continue;
            }
            PathEl::LineTo(p) => PathSeg::Line(Line::new(last, p)),
            PathEl::QuadTo(a, p) => PathSeg::Quad(QuadBez::new(last, a, p)),
            PathEl::CurveTo(a, b, p) => PathSeg::Cubic(CubicBez::new(last, a, b, p)),
            PathEl::ClosePath => {
                if last.distance(first) > 0.0 {
                    pending.push(PathSeg::Line(Line::new(last, first)));
                }
                last = first;
                out.extend(pending.drain(..).map(|seg| Part { seg, contour, start: None, end: None }));
                continue;
            }
        };
        last = seg.end();
        pending.push(seg);
    }
    if out.iter().any(|p| !finite(&p.seg)) {
        return Vec::new();
    }
    out
}

fn finite(seg: &PathSeg) -> bool {
    let ok = |p: Point| p.x.is_finite() && p.y.is_finite();
    match seg {
        PathSeg::Line(l) => ok(l.p0) && ok(l.p1),
        PathSeg::Quad(q) => ok(q.p0) && ok(q.p1) && ok(q.p2),
        PathSeg::Cubic(c) => ok(c.p0) && ok(c.p1) && ok(c.p2) && ok(c.p3),
    }
}

/// Answers whether points are ink, within a budget of segment visits.
struct Ink<'a> {
    path: &'a BezPath,
    cost: usize,
    left: usize,
    /// A spend was refused: the budget ran out.
    short: bool,
}

impl Ink<'_> {
    /// Whether `p` is ink; `None` once the budget is spent.
    fn at(&mut self, p: Point) -> Option<bool> {
        self.spend(self.cost)?;
        Some(self.path.winding(p) != 0)
    }

    fn spend(&mut self, n: usize) -> Option<()> {
        let left = self.left.checked_sub(n);
        self.short |= left.is_none();
        self.left = left?;
        Some(())
    }
}

/// A straight part.
#[derive(Clone, Copy, Debug)]
struct Edge {
    part: usize,
    a: Point,
    b: Point,
    /// The unit direction from `a` to `b`.
    u: Vec2,
    len: f64,
}

/// A straight stem before edges split by junctions are merged.
struct Pair {
    direction: StemDirection,
    /// The stem's run direction (unit) and its normal, pointing from the first edge to the
    /// second.
    u: Vec2,
    sides: [(usize, Line); 2],
    width: f64,
    overlap: f64,
}

/// The stems of the outline made of `parts`, whose ink is the nonzero fill of `ink`.
pub(crate) fn find(parts: &[Part], ink: &BezPath, options: &StemOptions) -> Vec<Found> {
    let mut work = usize::MAX;
    find_within(parts, ink, options, &mut work)
}

/// [`find`] drawing its segment visits from `work` as well (at most [`MAX_WORK`] of them). When
/// `work` runs out it is left at 0 and the stems found so far are returned.
pub(crate) fn find_within(parts: &[Part], ink: &BezPath, options: &StemOptions, work: &mut usize) -> Vec<Found> {
    let start = (*work).min(MAX_WORK);
    let mut ink = Ink { path: ink, cost: ink.elements().len().max(1), left: start, short: false };
    let found = find_with(parts, &mut ink, options);
    *work = if ink.short && start == *work { 0 } else { work.saturating_sub(start.saturating_sub(ink.left)) };
    found
}

fn find_with(parts: &[Part], ink: &mut Ink, options: &StemOptions) -> Vec<Found> {
    let Some(o) = sane(options) else { return Vec::new() };
    let sin_tol = o.angle_tolerance.to_radians().sin();

    let mut edges: Vec<Edge> = parts
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let (a, b) = (p.seg.start(), p.seg.end());
            let len = a.distance(b);
            (len >= o.min_length.max(1e-6) && straight(&p.seg)).then(|| Edge { part: i, a, b, u: (b - a) / len, len })
        })
        .collect();
    if edges.len() > MAX_EDGES {
        edges.sort_by(|x, y| y.len.total_cmp(&x.len));
        edges.truncate(MAX_EDGES);
    }

    let mut pairs = Vec::new();
    'outer: for (i, a) in edges.iter().enumerate() {
        for b in edges.iter().skip(i + 1) {
            match pair(a, b, &o, sin_tol, ink) {
                Ok(Some(p)) => pairs.push(p),
                Ok(None) => {}
                Err(()) => break 'outer,
            }
            if pairs.len() >= MAX_STEMS {
                break 'outer;
            }
        }
    }
    let mut found = merge(pairs);
    found.sort_by(|x, y| y.overlap.total_cmp(&x.overlap));

    let mut round = round_stems(parts, &edges, &o, sin_tol, ink);
    round.sort_by(|x, y| y.stem.width.total_cmp(&x.stem.width));
    found.extend(dedupe_round(round));
    found.truncate(MAX_STEMS);
    found
}

/// The options with finite values in order; `None` when they admit no stem.
fn sane(o: &StemOptions) -> Option<StemOptions> {
    let ok = [o.min_width, o.max_width, o.min_length, o.angle_tolerance].iter().all(|v| v.is_finite());
    let out = StemOptions {
        min_width: o.min_width.max(0.0),
        max_width: o.max_width.min(crate::MAX_COORDINATE * 2.0),
        min_length: o.min_length.max(0.0),
        angle_tolerance: o.angle_tolerance.clamp(0.0, 45.0),
        direction: o.direction,
    };
    (ok && out.min_width <= out.max_width).then_some(out)
}

/// Whether `seg` is a line or a curve that keeps within [`STRAIGHT_TOLERANCE`] of its chord.
fn straight(seg: &PathSeg) -> bool {
    let near = |a: Point, b: Point, ctrl: &[Point]| {
        let len = a.distance(b);
        if len <= 0.0 {
            return false;
        }
        let u = (b - a) / len;
        ctrl.iter().all(|c| {
            let v = *c - a;
            let along = v.dot(u);
            v.cross(u).abs() <= STRAIGHT_TOLERANCE && (-STRAIGHT_TOLERANCE..=len + STRAIGHT_TOLERANCE).contains(&along)
        })
    };
    match seg {
        PathSeg::Line(_) => true,
        PathSeg::Quad(q) => near(q.p0, q.p2, &[q.p1]),
        PathSeg::Cubic(c) => near(c.p0, c.p3, &[c.p1, c.p2]),
    }
}

fn classify(u: Vec2, sin_tol: f64) -> StemDirection {
    if u.x.abs() <= sin_tol {
        StemDirection::Vertical
    } else if u.y.abs() <= sin_tol {
        StemDirection::Horizontal
    } else {
        StemDirection::Diagonal
    }
}

/// The stem between edges `a` and `b`, if they make one. `Err` when the work budget is spent.
fn pair(a: &Edge, b: &Edge, o: &StemOptions, sin_tol: f64, ink: &mut Ink) -> Result<Option<Pair>, ()> {
    if a.u.cross(b.u).abs() > sin_tol {
        return Ok(None);
    }
    let ub = if a.u.dot(b.u) >= 0.0 { b.u } else { -b.u };
    let sum = a.u + ub;
    let s = sum.hypot();
    if s <= 0.0 {
        return Ok(None);
    }
    let u = sum / s;
    let direction = classify(u, sin_tol);
    if o.direction.is_some_and(|d| d != direction) {
        return Ok(None);
    }
    let proj = |p: Point| p.to_vec2().dot(u);
    let span = |e: &Edge| {
        let (x, y) = (proj(e.a), proj(e.b));
        (x.min(y), x.max(y))
    };
    let ((a0, a1), (b0, b1)) = (span(a), span(b));
    let (lo, hi) = (a0.max(b0), a1.min(b1));
    let overlap = hi - lo;
    if overlap < o.min_length || overlap <= 0.0 {
        return Ok(None);
    }
    let at = |e: &Edge, s: f64| {
        let (p0, p1) = (proj(e.a), proj(e.b));
        let den = p1 - p0;
        if den.abs() < 1e-12 { e.a } else { e.a.lerp(e.b, (s - p0) / den) }
    };
    let n = Vec2::new(-u.y, u.x);
    let w0 = (at(b, lo) - at(a, lo)).dot(n);
    let w1 = (at(b, hi) - at(a, hi)).dot(n);
    if w0 * w1 <= 0.0 {
        return Ok(None);
    }
    let width = (w0.abs() + w1.abs()) / 2.0;
    if width < o.min_width || width > o.max_width || width > overlap {
        return Ok(None);
    }
    for f in [0.25, 0.5, 0.75] {
        let s = lo + overlap * f;
        let (pa, pb) = (at(a, s), at(b, s));
        for c in [0.25, 0.5, 0.75] {
            if !ink.at(pa.lerp(pb, c)).ok_or(())? {
                return Ok(None);
            }
        }
    }
    let mid = lo + overlap / 2.0;
    let (pa, pb) = (at(a, mid), at(b, mid));
    let across = (pb - pa) / width.max(1e-12);
    if ink.at(pa - across * PROBE).ok_or(())? || ink.at(pb + across * PROBE).ok_or(())? {
        return Ok(None);
    }
    let la = Line::new(at(a, lo), at(a, hi));
    let lb = Line::new(at(b, lo), at(b, hi));
    let (ma, mb) = (la.midpoint(), lb.midpoint());
    let a_first = match direction {
        StemDirection::Horizontal => ma.y < mb.y,
        StemDirection::Vertical | StemDirection::Diagonal => ma.x < mb.x || (ma.x == mb.x && ma.y < mb.y),
    };
    let sides = if a_first { [(a.part, la), (b.part, lb)] } else { [(b.part, lb), (a.part, la)] };
    Ok(Some(Pair { direction, u, sides, width, overlap }))
}

/// One stem for pairs of the same direction and width that share an edge on one side (an edge
/// split by a junction); the edges are joined along the stem.
fn merge(pairs: Vec<Pair>) -> Vec<Found> {
    // Each group: its first pair, the edges of both sides, and the lines along both sides.
    type Group = (Pair, Vec<Side>, Vec<Side>, Vec<Line>, Vec<Line>);
    let mut groups: Vec<Group> = Vec::new();
    for p in pairs {
        let [(fa, la), (fb, lb)] = p.sides;
        let joined = groups.iter_mut().find(|(g, s0, s1, ..)| {
            g.direction == p.direction && (g.width - p.width).abs() <= SAME_WIDTH && (s0.contains(&Side::Edge(fa)) || s1.contains(&Side::Edge(fb)))
        });
        match joined {
            Some((g, s0, s1, l0, l1)) => {
                g.overlap += p.overlap;
                for (list, part) in [(&mut *s0, fa), (&mut *s1, fb)] {
                    if !list.contains(&Side::Edge(part)) {
                        list.push(Side::Edge(part));
                    }
                }
                l0.push(la);
                l1.push(lb);
            }
            None => groups.push((p, vec![Side::Edge(fa)], vec![Side::Edge(fb)], vec![la], vec![lb])),
        }
    }
    groups
        .into_iter()
        .map(|(g, s0, s1, l0, l1)| Found {
            stem: Stem { direction: g.direction, first: span_of(&l0, g.u), second: span_of(&l1, g.u), width: g.width, round: false },
            sides: [s0, s1],
            overlap: g.overlap,
        })
        .collect()
}

/// The line from the earliest to the latest point of `lines` along `u`.
fn span_of(lines: &[Line], u: Vec2) -> Line {
    let points = || lines.iter().flat_map(|l| [l.p0, l.p1]);
    let key = |p: &Point| p.to_vec2().dot(u);
    let lo = points().min_by(|x, y| key(x).total_cmp(&key(y)));
    let hi = points().max_by(|x, y| key(x).total_cmp(&key(y)));
    match (lo, hi) {
        (Some(a), Some(b)) => Line::new(a, b),
        _ => Line::new(Point::ZERO, Point::ZERO),
    }
}

/// The unit tangent of `seg` at `t`, read a little way off where the derivative vanishes.
pub(crate) fn tangent(seg: &PathSeg, t: f64) -> Option<Vec2> {
    let c = seg.to_cubic();
    let d = c.deriv().eval(t).to_vec2();
    if d.hypot() > 1e-9 {
        return Some(d.normalize());
    }
    let (p, q) = if t < 0.5 { (c.eval(t), c.eval((t + 1e-3).min(1.0))) } else { (c.eval((t - 1e-3).max(0.0)), c.eval(t)) };
    let v = q - p;
    (v.hypot() > 1e-12).then(|| v.normalize())
}

/// A point rounded to hundredths, for finding points that coincide.
fn key(p: Point) -> (i64, i64) {
    ((p.x * 100.0).round() as i64, (p.y * 100.0).round() as i64)
}

fn round_stems(parts: &[Part], edges: &[Edge], o: &StemOptions, sin_tol: f64, ink: &mut Ink) -> Vec<Found> {
    let mut out = Vec::new();
    let sin_partner = ROUND_PARTNER_DEGREES.to_radians().sin();
    // The ends of straight edges, with their directions.
    let mut edge_ends: HashMap<(i64, i64), Vec<Vec2>> = HashMap::new();
    for e in edges {
        for p in [e.a, e.b] {
            edge_ends.entry(key(p)).or_default().push(e.u);
        }
    }
    let straight_parts: HashSet<usize> = edges.iter().map(|e| e.part).collect();
    let mut seen: HashSet<((i64, i64), bool)> = HashSet::new();
    let reach = o.max_width + 1.0;
    let directions: Vec<StemDirection> =
        [StemDirection::Vertical, StemDirection::Horizontal].into_iter().filter(|d| o.direction.is_none_or(|w| w == *d)).collect();
    for (i, part) in parts.iter().enumerate() {
        if matches!(part.seg, PathSeg::Line(_)) || straight_parts.contains(&i) || straight(&part.seg) {
            continue;
        }
        let mut ts = vec![0.0, 1.0];
        ts.extend(part.seg.extrema());
        for t in ts {
            let Some(tau) = tangent(&part.seg, t) else { continue };
            let p = part.seg.eval(t);
            for &direction in &directions {
                let vertical = direction == StemDirection::Vertical;
                let lean = if vertical { tau.x.abs() } else { tau.y.abs() };
                if lean > sin_tol || !seen.insert((key(p), vertical)) {
                    continue;
                }
                if edge_ends.get(&key(p)).is_some_and(|dirs| dirs.iter().any(|u| u.cross(tau).abs() <= sin_tol)) {
                    continue;
                }
                let axis = if vertical { Vec2::new(1.0, 0.0) } else { Vec2::new(0.0, 1.0) };
                let (Some(plus), Some(minus)) = (ink.at(p + axis * PROBE), ink.at(p - axis * PROBE)) else { return out };
                let ray = match (plus, minus) {
                    (true, false) => axis,
                    (false, true) => -axis,
                    _ => continue,
                };
                if ink.spend(parts.len()).is_none() {
                    return out;
                }
                let line = Line::new(p, p + ray * reach);
                let mut best: Option<(usize, f64, f64)> = None;
                for (k, other) in parts.iter().enumerate() {
                    for hit in other.seg.intersect_line(line) {
                        let d = hit.line_t * reach;
                        if d > PROBE && best.is_none_or(|(_, _, b)| d < b) {
                            best = Some((k, hit.segment_t.clamp(0.0, 1.0), d));
                        }
                    }
                }
                let Some((k, tk, width)) = best else { continue };
                if width < o.min_width || width > o.max_width {
                    continue;
                }
                let Some(far) = parts.get(k) else { continue };
                let q = far.seg.eval(tk);
                let Some(tau_q) = tangent(&far.seg, tk) else { continue };
                if tau.cross(tau_q).abs() > sin_partner {
                    continue;
                }
                let Some(beyond) = ink.at(q + ray * PROBE) else { return out };
                if beyond {
                    continue;
                }
                let near = (Line::new(p, p), Side::At(i, t));
                let far = (Line::new(q, q), Side::At(k, tk));
                let ((l0, s0), (l1, s1)) = if ray.x + ray.y > 0.0 { (near, far) } else { (far, near) };
                out.push(Found { stem: Stem { direction, first: l0, second: l1, width, round: true }, sides: [vec![s0], vec![s1]], overlap: 0.0 });
                if out.len() >= MAX_STEMS {
                    return out;
                }
            }
        }
    }
    out
}

/// Of round stems measured across the same part of a bowl, the thickest; `list` comes thickest
/// first. At most [`MAX_EDGES`] are compared.
fn dedupe_round(list: Vec<Found>) -> Vec<Found> {
    let mut kept: Vec<Found> = Vec::new();
    for f in list.into_iter().take(MAX_EDGES) {
        let vertical = f.stem.direction == StemDirection::Vertical;
        let across = |s: &Stem| if vertical { (s.first.p0.x, s.second.p0.x) } else { (s.first.p0.y, s.second.p0.y) };
        let along = |s: &Stem| if vertical { (s.first.p0.y + s.second.p0.y) / 2.0 } else { (s.first.p0.x + s.second.p0.x) / 2.0 };
        let same = kept.iter().any(|k| {
            let (a0, a1) = across(&k.stem);
            let (b0, b1) = across(&f.stem);
            k.stem.direction == f.stem.direction && a0 < b1 && b0 < a1 && (along(&k.stem) - along(&f.stem)).abs() <= k.stem.width.max(f.stem.width)
        });
        if !same {
            kept.push(f);
        }
    }
    kept
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn polygon(path: &mut BezPath, points: &[(f64, f64)]) {
        for (i, &(x, y)) in points.iter().enumerate() {
            if i == 0 {
                path.move_to((x, y));
            } else {
                path.line_to((x, y));
            }
        }
        path.close_path();
    }

    /// An oval as four cubics, counter-clockwise, or clockwise with `reverse`.
    fn oval(path: &mut BezPath, cx: f64, cy: f64, rx: f64, ry: f64, reverse: bool) {
        let k = 0.5523;
        let quarter = |sx: f64, sy: f64, ex: f64, ey: f64| (Point::new(cx + sx, cy + sy), Point::new(cx + ex, cy + ey));
        let mut corners = [(rx, 0.0), (0.0, ry), (-rx, 0.0), (0.0, -ry), (rx, 0.0)];
        if reverse {
            corners.reverse();
        }
        path.move_to((cx + corners[0].0, cy + corners[0].1));
        for w in corners.windows(2) {
            let ((sx, sy), (ex, ey)) = (w[0], w[1]);
            let (s, e) = quarter(sx, sy, ex, ey);
            // The handles run along the tangents: horizontal at the top and bottom, vertical at
            // the sides.
            let c1 = if sx == 0.0 { Point::new(s.x + ex * k, s.y) } else { Point::new(s.x, s.y + ey * k) };
            let c2 = if ex == 0.0 { Point::new(e.x + sx * k, e.y) } else { Point::new(e.x, e.y + sy * k) };
            path.curve_to(c1, c2, e);
        }
        path.close_path();
    }

    pub(crate) fn letter_h() -> BezPath {
        let mut p = BezPath::new();
        polygon(
            &mut p,
            &[
                (0.0, 0.0),
                (80.0, 0.0),
                (80.0, 300.0),
                (520.0, 300.0),
                (520.0, 0.0),
                (600.0, 0.0),
                (600.0, 700.0),
                (520.0, 700.0),
                (520.0, 360.0),
                (80.0, 360.0),
                (80.0, 700.0),
                (0.0, 700.0),
            ],
        );
        p
    }

    pub(crate) fn letter_n() -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((80.0, 0.0));
        p.line_to((80.0, 380.0));
        p.curve_to((100.0, 420.0), (160.0, 440.0), (240.0, 440.0));
        p.curve_to((320.0, 440.0), (400.0, 420.0), (400.0, 380.0));
        p.line_to((400.0, 0.0));
        p.line_to((480.0, 0.0));
        p.line_to((480.0, 380.0));
        p.curve_to((480.0, 470.0), (380.0, 520.0), (240.0, 520.0));
        p.curve_to((160.0, 520.0), (100.0, 490.0), (80.0, 450.0));
        p.line_to((80.0, 500.0));
        p.line_to((0.0, 500.0));
        p.close_path();
        p
    }

    pub(crate) fn letter_o() -> BezPath {
        let mut p = BezPath::new();
        oval(&mut p, 250.0, 260.0, 250.0, 260.0, false);
        oval(&mut p, 250.0, 260.0, 170.0, 190.0, true);
        p
    }

    fn widths(stems: &[Stem], direction: StemDirection, round: bool) -> Vec<f64> {
        stems.iter().filter(|s| s.direction == direction && s.round == round).map(|s| (s.width * 100.0).round() / 100.0).collect()
    }

    #[test]
    fn the_stems_of_h_n_and_o() {
        let o = StemOptions::default();
        let h = detect_stems(&letter_h(), &o);
        assert_eq!(widths(&h, StemDirection::Vertical, false), vec![80.0, 80.0], "{h:?}");
        assert_eq!(widths(&h, StemDirection::Horizontal, false), vec![60.0], "{h:?}");
        assert!(h.iter().all(|s| !s.round), "{h:?}");
        let left = h.iter().find(|s| s.direction == StemDirection::Vertical && s.first.p0.x == 0.0).unwrap();
        assert_eq!((left.first.p0.x, left.second.p0.x), (0.0, 80.0), "first is the left edge");
        assert_eq!((left.first.p0.y.min(left.first.p1.y), left.first.p0.y.max(left.first.p1.y)), (0.0, 700.0), "the split edge gives one stem");
        let bar = h.iter().find(|s| s.direction == StemDirection::Horizontal).unwrap();
        assert_eq!((bar.first.p0.y, bar.second.p0.y), (300.0, 360.0), "first is the lower edge");

        let n = detect_stems(&letter_n(), &o);
        assert_eq!(widths(&n, StemDirection::Vertical, false), vec![80.0, 80.0], "{n:?}");
        assert_eq!(widths(&n, StemDirection::Horizontal, true), vec![80.0], "the arch, {n:?}");
        assert!(widths(&n, StemDirection::Vertical, true).is_empty(), "the arch's ends belong to the straight stems, {n:?}");

        let round = detect_stems(&letter_o(), &o);
        assert_eq!(widths(&round, StemDirection::Vertical, true), vec![80.0, 80.0], "{round:?}");
        assert_eq!(widths(&round, StemDirection::Horizontal, true), vec![70.0, 70.0], "{round:?}");
        assert!(round.iter().all(|s| s.round));

        let only = detect_stems(&letter_h(), &StemOptions { direction: Some(StemDirection::Horizontal), ..o.clone() });
        assert_eq!(widths(&only, StemDirection::Horizontal, false), vec![60.0]);
        assert_eq!(only.len(), 1);
    }

    #[test]
    fn diagonals_are_measured_at_right_angles_and_bad_input_gives_nothing() {
        // A bar leaning 45 degrees, 100 units wide across.
        let mut p = BezPath::new();
        let d = 100.0 / 2f64.sqrt();
        polygon(&mut p, &[(0.0, 0.0), (2.0 * d, 0.0), (2.0 * d + 500.0, 500.0), (500.0, 500.0)]);
        let stems = detect_stems(&p, &StemOptions::default());
        let diagonal: Vec<&Stem> = stems.iter().filter(|s| s.direction == StemDirection::Diagonal).collect();
        assert_eq!(diagonal.len(), 1, "{stems:?}");
        assert!((diagonal[0].width - 100.0).abs() < 1e-6);

        let mut bad = BezPath::new();
        polygon(&mut bad, &[(0.0, 0.0), (f64::NAN, 0.0), (10.0, 10.0)]);
        assert!(detect_stems(&bad, &StemOptions::default()).is_empty());
        assert!(detect_stems(&letter_h(), &StemOptions { min_width: f64::INFINITY, ..StemOptions::default() }).is_empty());
        assert!(detect_stems(&BezPath::new(), &StemOptions::default()).is_empty());
    }
}
