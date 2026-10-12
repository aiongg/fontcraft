//! Bounds and curve maths of path elements.
//!
//! Ported from otfautohint's `glyphData.py` (`boundsState`, `pathBoundsState` and the curve
//! methods of `pathElement`; AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`), which
//! use fontTools' `bezierTools` (MIT, Copyright 2017 Just van Rossum and the fontTools authors):
//! the quadratic and cubic solvers, `calcCubicParameters`, `splitCubicAtT`, `segmentPointAtT`,
//! `approximateCubicArcLength`. The arithmetic keeps the original's order of operations so that
//! results agree to the last bit.

use std::f64::consts::PI;

use crate::glyph::{PathElement, PeRef, Pt, feq_by, fne, round_digits};

/// `[[min x, min y], [max x, max y]]`, as the original stores bounds.
pub type Rect = [[f64; 2]; 2];

/// fontTools' `epsilon` for the equation solvers.
const EPSILON: f64 = 1e-10;
/// fontTools' `epsilonDigits`: cubic roots are rounded to this many decimals.
const EPSILON_DIGITS: u32 = 6;

fn coord(p: Pt, j: usize) -> f64 {
    if j == 0 { p.x } else { p.y }
}

fn rect_get(r: &Rect, i: usize, j: usize) -> f64 {
    r.get(i).and_then(|row| row.get(j)).copied().unwrap_or(0.0)
}

fn rect_set(r: &mut Rect, i: usize, j: usize, v: f64) {
    if let Some(x) = r.get_mut(i).and_then(|row| row.get_mut(j)) {
        *x = v;
    }
}

fn set2<T>(a: &mut [[T; 2]; 2], i: usize, j: usize, v: T) {
    if let Some(x) = a.get_mut(i).and_then(|row| row.get_mut(j)) {
        *x = v;
    }
}

fn get2<T: Copy + Default>(a: &[[T; 2]; 2], i: usize, j: usize) -> T {
    a.get(i).and_then(|row| row.get(j)).copied().unwrap_or_default()
}

/// The points and curve parameters of a bounds record's extremes.
type Extremes<'a> = (&'a mut [[Pt; 2]; 2], &'a mut [[f64; 2]; 2]);

/// `mergePt` with the record of extremes passed separately, so `b` can be one of the state's
/// own rectangles.
fn merge_into(b: &mut Rect, mut ext: Option<Extremes<'_>>, p: Pt, t: f64) {
    for i in 0..2 {
        for j in 0..2 {
            let (v, cur) = (coord(p, j), rect_get(b, i, j));
            let beyond = if i == 0 { v < cur } else { v > cur };
            if beyond {
                rect_set(b, i, j, v);
                if let Some((extpts, tmap)) = ext.as_mut() {
                    set2(extpts, i, j, p);
                    set2(tmap, i, j, t);
                }
            }
        }
    }
}

/// The bounds of one path element and the points and curve parameters that define each side
/// (`boundsState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoundsState {
    /// Bounds of the chord from start to end point (`lb`).
    pub linear: Rect,
    /// The element's true bounds, curve extremes included.
    pub bounds: Rect,
    /// The point that sets each side: `extpts[min/max][x/y]`.
    pub extpts: [[Pt; 2]; 2],
    /// The curve parameter of each of those points.
    pub tmap: [[f64; 2]; 2],
    /// `bounds` and `linear` are one list in the original (lines, and curves whose control
    /// points stay inside the chord's bounds): a change to one is a change to both. See
    /// [`crate::glyph::GlyphData::get_bounds`].
    pub shared: bool,
}

impl BoundsState {
    /// Bounds of `pe` (`boundsState.__init__`).
    pub fn new(pe: &PathElement) -> Self {
        let s = [pe.s.x, pe.s.y];
        let mut st = BoundsState { linear: [s, s], bounds: [s, s], extpts: [[pe.s, pe.s], [pe.s, pe.s]], tmap: [[0.0; 2]; 2], shared: true };
        // linearBounds
        let mut lb = st.linear;
        merge_into(&mut lb, Some((&mut st.extpts, &mut st.tmap)), pe.e, 1.0);
        st.linear = lb;
        st.bounds = lb;
        if pe.is_line {
            return st;
        }
        // The t values of the control points are not used.
        let mut cpb = lb;
        merge_into(&mut cpb, None, pe.cs, 0.25);
        merge_into(&mut cpb, None, pe.ce, 0.75);
        if lb != cpb {
            st.calc_curve_bounds(pe);
            st.shared = false;
        }
        st
    }

    /// `calcCurveBounds`: the extremes of the curve at the roots of its derivative.
    fn calc_curve_bounds(&mut self, pe: &PathElement) {
        let mut curveb = self.linear;
        let (a, b, c, d) = pe.cubic_parameters();
        let mut roots = Vec::new();
        for dim in 0..2 {
            let sols = solve_quadratic(coord(a, dim) * 3.0, coord(b, dim) * 2.0, coord(c, dim));
            roots.extend(sols.into_iter().filter(|t| (0.0..1.0).contains(t)));
        }
        for t in roots {
            let p = Pt::new(a.x * t * t * t + b.x * t * t + c.x * t + d.x, a.y * t * t * t + b.y * t * t + c.y * t + d.y);
            merge_into(&mut curveb, Some((&mut self.extpts, &mut self.tmap)), p, t);
        }
        self.bounds = curveb;
    }

    /// Adds `p` at parameter `t` as a possible extreme of `b`; with `record`, a new extreme is
    /// written to [`BoundsState::extpts`] and [`BoundsState::tmap`] (`mergePt`).
    pub fn merge_pt(&mut self, b: &mut Rect, p: Pt, t: f64, record: bool) {
        let ext = if record { Some((&mut self.extpts, &mut self.tmap)) } else { None };
        merge_into(b, ext, p, t);
    }

    /// The side farthest from the chord in x, or in y with `do_y`: (distance, point, t, the side
    /// is the maximum) (`farthestExtreme`). A distance of 0 means the curve does not leave its
    /// chord's bounds in that dimension; the other values are then meaningless.
    pub fn farthest_extreme(&self, do_y: bool) -> (f64, Pt, f64, bool) {
        let idx = usize::from(do_y);
        let d0 = (rect_get(&self.linear, 0, idx) - rect_get(&self.bounds, 0, idx)).abs();
        let d1 = (rect_get(&self.linear, 1, idx) - rect_get(&self.bounds, 1, idx)).abs();
        if d0 > d1 {
            (d0, get2(&self.extpts, 0, idx), get2(&self.tmap, 0, idx), false)
        } else if fne(d1, 0.0) {
            (d1, get2(&self.extpts, 1, idx), get2(&self.tmap, 1, idx), true)
        } else {
            (0.0, Pt::default(), 0.0, false)
        }
    }

    /// The bounds overlap `other`'s, grown by `margin` (`intersects`).
    pub fn intersects(&self, other: &BoundsState, margin: f64) -> bool {
        let (a, b) = (&self.bounds, &other.bounds);
        rect_get(a, 0, 0) <= rect_get(b, 1, 0) + margin
            && rect_get(a, 1, 0) + margin >= rect_get(b, 0, 0)
            && rect_get(a, 0, 1) <= rect_get(b, 1, 1) + margin
            && rect_get(a, 1, 1) + margin >= rect_get(b, 0, 1)
    }
}

/// The bounds of a glyph or subpath and the elements that set each side (`pathBoundsState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathBoundsState {
    pub bounds: Rect,
    /// `extpes[min/max][x/y]`.
    pub extpes: [[PeRef; 2]; 2],
}

impl PathBoundsState {
    /// The bounds of one element (`pathBoundsState.__init__`).
    pub fn new(pe: PeRef, bounds: &BoundsState) -> Self {
        Self { bounds: bounds.bounds, extpes: [[pe, pe], [pe, pe]] }
    }

    /// Grows to include `other` (`merge`).
    pub fn merge(&mut self, other: &PathBoundsState) {
        for i in 0..2 {
            for j in 0..2 {
                let (v, cur) = (rect_get(&other.bounds, i, j), rect_get(&self.bounds, i, j));
                let beyond = if i == 0 { v < cur } else { v > cur };
                if beyond {
                    rect_set(&mut self.bounds, i, j, v);
                    set2(&mut self.extpes, i, j, get2(&other.extpes, i, j));
                }
            }
        }
    }

    /// These bounds lie inside `other`'s (`within`).
    pub fn within(&self, other: &PathBoundsState) -> bool {
        let (a, b) = (&self.bounds, &other.bounds);
        rect_get(a, 0, 0) >= rect_get(b, 0, 0)
            && rect_get(a, 0, 1) >= rect_get(b, 0, 1)
            && rect_get(a, 1, 0) <= rect_get(b, 1, 0)
            && rect_get(a, 1, 1) <= rect_get(b, 1, 1)
    }
}

impl PathElement {
    /// The polynomial coefficients `(a, b, c, d)` of the cubic (`cubicParameters`). *Shared.*
    pub fn cubic_parameters(&self) -> (Pt, Pt, Pt, Pt) {
        calc_cubic_parameters(self.s, self.cs, self.ce, self.e)
    }

    /// Whether `p` lies on the element within `factor`; with `want_t`, also the parameter
    /// (`containsPoint`). Without `want_t` the parameter is `None`; with it, a point that is not
    /// on the element gives `Some(0.0)` as in the original. *Shared.*
    pub fn contains_point(&self, p: Pt, factor: f64, want_t: bool) -> (bool, Option<f64>) {
        if self.is_line {
            let ds = self.s.distsq(self.e).sqrt();
            let d1 = self.s.distsq(p).sqrt();
            let d2 = self.e.distsq(p).sqrt();
            let iseq = feq_by(ds, d1 + d2, factor);
            if !want_t {
                return (iseq, None);
            }
            // The original divides by zero on a zero-length line; 0 is the start.
            let t = if ds == 0.0 { 0.0 } else { d1 / ds };
            return (iseq, Some(t));
        }
        let (a, b, c, d) = self.cubic_parameters();
        let (i, j) = if (self.s.x - self.e.x).abs() > (self.s.y - self.e.y).abs() { (0, 1) } else { (1, 0) };
        let tl = solve_cubic(coord(a, i), coord(b, i), coord(c, i), coord(d, i) - coord(p, i));
        let first = tl
            .into_iter()
            .filter(|t| (-0.1..=1.1).contains(t))
            .map(|t| (coord(a, j) * t * t * t + coord(b, j) * t * t + coord(c, j) * t + coord(d, j), t))
            .find(|(v, _)| feq_by(*v, coord(p, j), factor))
            .map(|(_, t)| t);
        match (first, want_t) {
            (Some(t), true) => (true, Some(t)),
            (None, true) => (false, Some(0.0)),
            (found, false) => (found.is_some(), None),
        }
    }

    /// A point that gives the direction of the element at `t` (1, else 0) (`slopePoint`).
    /// *Shared.*
    pub fn slope_point(&self, t: f64) -> Pt {
        if t == 1.0 {
            if self.is_line {
                return self.s;
            }
            if self.e.near(self.ce) {
                return self.cs;
            }
            self.ce
        } else {
            if self.is_line {
                return self.e;
            }
            if self.s.near(self.cs) {
                return self.ce;
            }
            self.cs
        }
    }

    /// Splits the element at `t`, keeping the first part and returning the second (`splitAt`).
    /// The second part is a new element: no masks, no flex, [`PathElement::flex_unset`].
    pub fn split_at(&mut self, t: f64) -> PathElement {
        let mut ret = if self.is_line {
            let pb = self.s + (self.e - self.s) * t;
            let ret = PathElement::line(pb, self.e);
            self.e = pb;
            self.ce = pb;
            ret
        } else {
            let parts = split_cubic_at_t(self.s, self.cs, self.ce, self.e, &[t]);
            let (Some(first), Some(second)) = (parts.first().copied(), parts.get(1).copied()) else {
                return PathElement::line(self.e, self.e);
            };
            self.cs = first[1];
            self.ce = first[2];
            self.e = first[3];
            PathElement::curve(second[0], second[1], second[2], second[3])
        };
        ret.position = self.position;
        ret.is_close = self.is_close;
        self.is_close = false;
        self.bounds = None;
        ret
    }

    /// The point at parameter `t` (`atT`). *Shared.*
    pub fn at_t(&self, t: f64) -> Pt {
        if self.is_line { line_point_at_t(self.s, self.e, t) } else { cubic_point_at_t(self.s, self.cs, self.ce, self.e, t) }
    }

    /// The element as a fontTools segment: its points after the start (`fonttoolsSegment`).
    pub fn fonttools_segment(&self) -> Vec<Pt> {
        if self.is_line { vec![self.s, self.e] } else { vec![self.s, self.cs, self.ce, self.e] }
    }
}

/// fontTools' `solveQuadratic`: the real roots of `a·x² + b·x + c`, neither sorted nor unique.
pub fn solve_quadratic(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a.abs() < EPSILON {
        if b.abs() < EPSILON { Vec::new() } else { vec![-c / b] }
    } else {
        let dd = b * b - 4.0 * a * c;
        if dd >= 0.0 {
            let rdd = dd.sqrt();
            vec![(-b + rdd) / 2.0 / a, (-b - rdd) / 2.0 / a]
        } else {
            Vec::new()
        }
    }
}

/// fontTools' `solveCubic`: the real roots of `a·x³ + b·x² + c·x + d`, rounded to 6 decimals.
pub fn solve_cubic(a: f64, b: f64, c: f64, d: f64) -> Vec<f64> {
    if a.abs() < EPSILON {
        return solve_quadratic(b, c, d);
    }
    let a1 = b / a;
    let a2 = c / a;
    let a3 = d / a;

    let q = (a1 * a1 - 3.0 * a2) / 9.0;
    let r = (2.0 * a1 * a1 * a1 - 9.0 * a1 * a2 + 27.0 * a3) / 54.0;

    let r2 = r * r;
    let q3 = q * q * q;
    let r2 = if r2 < EPSILON { 0.0 } else { r2 };
    let q3 = if q3.abs() < EPSILON { 0.0 } else { q3 };

    let r2_q3 = r2 - q3;

    if r2 == 0.0 && q3 == 0.0 {
        let x = round_digits(-a1 / 3.0, EPSILON_DIGITS);
        vec![x, x, x]
    } else if r2_q3 <= EPSILON * 0.5 {
        // Python's min and max: the first argument wins unless the second is strictly beyond.
        let v = r / q3.sqrt();
        let v = if 1.0 < v { 1.0 } else { v };
        let v = if -1.0 > v { -1.0 } else { v };
        let theta = v.acos();
        let rq2 = -2.0 * q.sqrt();
        let a1_3 = a1 / 3.0;
        let mut xs = [rq2 * (theta / 3.0).cos() - a1_3, rq2 * ((theta + 2.0 * PI) / 3.0).cos() - a1_3, rq2 * ((theta + 4.0 * PI) / 3.0).cos() - a1_3];
        // Stable, with -0 and 0 equal, as Python's sorted.
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let [x0, x1, x2] = xs;
        let rd = |v: f64| round_digits(v, EPSILON_DIGITS);
        if x1 - x0 < EPSILON && x2 - x1 < EPSILON {
            let x = rd((x0 + x1 + x2) / 3.0);
            vec![x, x, x]
        } else if x1 - x0 < EPSILON {
            let x = rd((x0 + x1) / 2.0);
            vec![x, x, rd(x2)]
        } else if x2 - x1 < EPSILON {
            let x = rd((x1 + x2) / 2.0);
            vec![rd(x0), x, x]
        } else {
            vec![rd(x0), rd(x1), rd(x2)]
        }
    } else {
        let mut x = (r2_q3.sqrt() + r.abs()).powf(1.0 / 3.0);
        x += q / x;
        if r >= 0.0 {
            x = -x;
        }
        vec![round_digits(x - a1 / 3.0, EPSILON_DIGITS)]
    }
}

/// fontTools' `calcCubicParameters`: `(a, b, c, d)` with `p(t) = a·t³ + b·t² + c·t + d`.
pub fn calc_cubic_parameters(p1: Pt, p2: Pt, p3: Pt, p4: Pt) -> (Pt, Pt, Pt, Pt) {
    let cx = (p2.x - p1.x) * 3.0;
    let cy = (p2.y - p1.y) * 3.0;
    let bx = (p3.x - p2.x) * 3.0 - cx;
    let by = (p3.y - p2.y) * 3.0 - cy;
    let ax = p4.x - p1.x - cx - bx;
    let ay = p4.y - p1.y - cy - by;
    (Pt::new(ax, ay), Pt::new(bx, by), Pt::new(cx, cy), p1)
}

/// fontTools' `calcCubicPoints`, the inverse of [`calc_cubic_parameters`].
fn calc_cubic_points(a: Pt, b: Pt, c: Pt, d: Pt) -> [Pt; 4] {
    let x2 = (c.x / 3.0) + d.x;
    let y2 = (c.y / 3.0) + d.y;
    let x3 = (b.x + c.x) / 3.0 + x2;
    let y3 = (b.y + c.y) / 3.0 + y2;
    let x4 = a.x + d.x + c.x + b.x;
    let y4 = a.y + d.y + c.y + b.y;
    [d, Pt::new(x2, y2), Pt::new(x3, y3), Pt::new(x4, y4)]
}

/// fontTools' `splitCubicAtT`: the curve cut at each of `ts`, taken in the order given (the
/// original does not sort them). The last part ends exactly at `p4`.
pub fn split_cubic_at_t(p1: Pt, p2: Pt, p3: Pt, p4: Pt, ts: &[f64]) -> Vec<[Pt; 4]> {
    let (a, b, c, d) = calc_cubic_parameters(p1, p2, p3, p4);
    let mut all = Vec::with_capacity(ts.len() + 2);
    all.push(0.0);
    all.extend_from_slice(ts);
    all.push(1.0);
    let mut segments: Vec<[Pt; 4]> = all
        .windows(2)
        .filter_map(|w| match w {
            [t1, t2] => Some((*t1, *t2)),
            _ => None,
        })
        .map(|(t1, t2)| {
            let delta = t2 - t1;
            let delta_2 = delta * delta;
            let delta_3 = delta * delta_2;
            let t1_2 = t1 * t1;
            let t1_3 = t1 * t1_2;
            let a1 = Pt::new(a.x * delta_3, a.y * delta_3);
            let b1 = Pt::new((3.0 * a.x * t1 + b.x) * delta_2, (3.0 * a.y * t1 + b.y) * delta_2);
            let c1 = Pt::new((2.0 * b.x * t1 + c.x + 3.0 * a.x * t1_2) * delta, (2.0 * b.y * t1 + c.y + 3.0 * a.y * t1_2) * delta);
            let d1 = Pt::new(a.x * t1_3 + b.x * t1_2 + c.x * t1 + d.x, a.y * t1_3 + b.y * t1_2 + c.y * t1 + d.y);
            calc_cubic_points(a1, b1, c1, d1)
        })
        .collect();
    if let Some(last) = segments.last_mut() {
        last[3] = p4;
    }
    segments
}

/// fontTools' `linePointAtT`.
pub fn line_point_at_t(p1: Pt, p2: Pt, t: f64) -> Pt {
    Pt::new(p1.x * (1.0 - t) + p2.x * t, p1.y * (1.0 - t) + p2.y * t)
}

/// fontTools' `cubicPointAtT`.
pub fn cubic_point_at_t(p1: Pt, p2: Pt, p3: Pt, p4: Pt, t: f64) -> Pt {
    let t2 = t * t;
    let one_t = 1.0 - t;
    let one_t_2 = one_t * one_t;
    let f = |a: f64, b: f64, c: f64, d: f64| one_t_2 * one_t * a + 3.0 * (one_t_2 * t * b + one_t * t2 * c) + t2 * t * d;
    Pt::new(f(p1.x, p2.x, p3.x, p4.x), f(p1.y, p2.y, p3.y, p4.y))
}

/// fontTools' `approximateCubicArcLength` (Gauss–Lobatto quadrature with five points). The
/// original works on complex numbers; `abs` is `hypot`.
pub fn approximate_cubic_arc_length(p1: Pt, p2: Pt, p3: Pt, p4: Pt) -> f64 {
    let abs = |p: Pt| p.x.hypot(p.y);
    let lin =
        |k1: f64, k2: f64, k3: f64, k4: f64| Pt::new(k1 * p1.x + k2 * p2.x + k3 * p3.x + k4 * p4.x, k1 * p1.y + k2 * p2.y + k3 * p3.y + k4 * p4.y);
    let v0 = abs(p2 - p1) * 0.15;
    let v1 = abs(lin(-0.558983582205757, 0.325650248872424, 0.208983582205757, 0.024349751127576));
    let v2 = abs(p4 - p1 + p3 - p2) * 0.26666666666666666;
    let v3 = abs(lin(-0.024349751127576, -0.208983582205757, -0.325650248872424, 0.558983582205757));
    let v4 = abs(p4 - p3) * 0.15;
    v0 + v1 + v2 + v3 + v4
}

#[cfg(test)]
mod tests {
    //! Expected values from `tests/oracle/glyph_data.py` (otfautohint and fontTools).
    use super::*;

    fn pt(x: f64, y: f64) -> Pt {
        Pt::new(x, y)
    }

    #[test]
    fn solvers_and_arc_length_match_fonttools() {
        assert_eq!(solve_cubic(1.0, 1.0, -6.0, 0.0), vec![-3.0, -0.0, 2.0]);
        assert_eq!(solve_cubic(-10.0, -9.0, 48.0, -29.0), vec![-2.9, 1.0, 1.0]);
        assert_eq!(solve_cubic(1.0, -4.5, 6.75, -3.375), vec![1.5, 1.5, 1.5]);
        assert_eq!(solve_cubic(2.0, -3.0, 0.5, 0.1), vec![-0.114829, 0.342131, 1.272698]);
        assert_eq!(solve_cubic(1.0, 0.0, 1.0, -1.0), vec![0.682328]);
        // A tiny leading coefficient falls back to the quadratic, roots unsorted.
        assert_eq!(solve_cubic(1e-12, 1.0, -3.0, 2.0), vec![2.0, 1.0]);
        assert_eq!(approximate_cubic_arc_length(pt(0.0, 0.0), pt(25.0, 100.0), pt(75.0, 100.0), pt(100.0, 0.0)), 190.04332968932817);
        assert_eq!(approximate_cubic_arc_length(pt(0.0, 0.0), pt(50.0, 0.0), pt(100.0, 50.0), pt(100.0, 100.0)), 154.8852074945903);
    }

    #[test]
    fn curve_bounds_extremes_and_split_match_otfautohint() {
        // The top of a cupped serif: a curve that rises above its chord.
        let mut pe = PathElement::curve(pt(0.0, 0.0), pt(30.0, 100.0), pt(70.0, 100.0), pt(100.0, 0.0));
        let bs = pe.get_bounds().clone();
        assert_eq!(bs.linear, [[0.0, 0.0], [100.0, 0.0]]);
        assert_eq!(bs.bounds, [[0.0, 0.0], [100.0, 75.0]]);
        assert_eq!(bs.extpts, [[pt(0.0, 0.0), pt(0.0, 0.0)], [pt(100.0, 0.0), pt(50.0, 75.0)]]);
        assert_eq!(bs.tmap, [[0.0, 0.0], [1.0, 0.5]]);
        assert!(!bs.shared);
        assert_eq!(bs.farthest_extreme(true), (75.0, pt(50.0, 75.0), 0.5, true));
        assert_eq!(bs.farthest_extreme(false).0, 0.0);

        assert_eq!(pe.contains_point(pt(50.0, 75.0), 0.01, true), (true, Some(0.5)));
        assert_eq!(pe.contains_point(pt(50.0, 70.0), 0.01, true), (false, Some(0.0)));
        assert_eq!(pe.contains_point(pt(50.0, 75.0), 0.01, false), (true, None));
        let line = PathElement::line(pt(100.0, 0.0), pt(100.0, -200.0));
        assert_eq!(line.contains_point(pt(100.0, -50.0), 0.01, true), (true, Some(0.25)));

        assert_eq!(pe.at_t(0.25), pt(24.0625, 56.25));
        assert_eq!(pe.assoc_factor(false), 2.0246565712990474);
        assert_eq!(pe.assoc_factor(true), 4.049313142598095);
        assert_eq!((pe.slope_point(0.0), pe.slope_point(1.0)), (pt(30.0, 100.0), pt(70.0, 100.0)));

        let second = pe.split_at(0.25);
        assert_eq!((pe.s, pe.cs, pe.ce, pe.e), (pt(0.0, 0.0), pt(7.5, 25.0), pt(15.625, 43.75), pt(24.0625, 56.25)));
        assert_eq!((second.s, second.cs, second.ce, second.e), (pt(24.0625, 56.25), pt(49.375, 93.75), pt(77.5, 75.0), pt(100.0, 0.0)));
        assert!(pe.bounds.is_none() && second.flex_unset);
    }
}
