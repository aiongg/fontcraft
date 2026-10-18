//! Spiro curves: a smooth curve through on-curve control points, made of polynomial spirals and
//! converted to cubic Béziers for storage.
//!
//! The curve model follows Raph Levien's PhD thesis, *From Spiral to Spline: Optimal Techniques
//! in Interactive Curve Design* (UC Berkeley, 2009). It is written from the thesis's description
//! of the method; no implementation of it was read.
//!
//! Each segment between two control points is a spiral whose tangent angle is a polynomial of
//! degree 4 in arc length, so its curvature is a cubic in arc length. In a frame where the
//! segment has unit length and `s` runs from −½ to ½:
//! `θ(s) = k0·s + k1·s²/2 + k2·s³/6 + k3·s⁴/24`. The segment is then scaled and rotated so its
//! ends land on its two control points. Newton's method finds the four `k` of every segment so
//! that each control point's conditions hold:
//!
//! | Kind | Where two curved segments meet |
//! |---|---|
//! | [`SpiroKind::G4`] | tangent, curvature and the first two derivatives of curvature continuous |
//! | [`SpiroKind::G2`] | tangent and curvature continuous; the second derivative of curvature is zero on both sides |
//! | [`SpiroKind::Corner`] | nothing across the point; on each side the first two derivatives of curvature are zero |
//! | [`SpiroKind::Left`] | the segment after the point is straight; the curve before it meets the line tangentially |
//! | [`SpiroKind::Right`] | the segment before the point is straight; the curve after it leaves the line tangentially |
//!
//! A curved side that meets a straight segment takes the line's direction, and the second
//! derivative of its curvature is zero there. A segment between two corners is straight. The ends
//! of an open contour act as corners. Every curved segment so gets four conditions, two at each
//! end, and every straight one has its four `k` fixed at zero, so the system is square.
//!
//! With G2 points only, every segment has `k2 = k3 = 0`: the curve is the spline of Euler spirals
//! (curvature linear in arc length). Points on a circle give the circle.

use std::f64::consts::{FRAC_PI_2, PI};
use std::fmt;

use kurbo::{BezPath, CubicBez, ParamCurveNearest, Point, Vec2};

/// Most control points in one contour. The solver is dense: its work grows with the cube of
/// this.
pub const MAX_SPIRO_POINTS: usize = 128;
/// Largest coordinate a control point may have, in font units.
pub const MAX_SPIRO_COORDINATE: f64 = 1_000_000.0;
/// The conversion's default tolerance, in font units: the cubics stay this near the spiral.
pub const DEFAULT_TOLERANCE: f64 = 0.25;

/// Newton steps before the solver gives up.
const MAX_NEWTON_STEPS: usize = 40;
/// Halvings of one Newton step before the solver gives up.
const MAX_HALVINGS: usize = 12;
/// The solution is found when no condition is off by more than this (radians, or curvature in
/// the normalized frame).
const SOLVED: f64 = 1e-10;
/// Largest change of one `k` in one Newton step.
const MAX_STEP: f64 = 4.0;
/// Largest rate of turning in the unit frame (about 30 turns per segment): beyond it the
/// integration would need too many panels, and no useful curve turns that much.
const MAX_TURN_RATE: f64 = 200.0;
/// Most integration panels for one segment.
const MAX_PANELS: usize = 410;
/// Deepest halving of a segment when it is converted to cubics (at most 2⁸ cubics a segment).
const MAX_FIT_DEPTH: usize = 8;

/// Four-point Gauss–Legendre nodes and weights on [−1, 1].
const GL_X: [f64; 4] = [-0.861_136_311_594_052_6, -0.339_981_043_584_856_3, 0.339_981_043_584_856_3, 0.861_136_311_594_052_6];
const GL_W: [f64; 4] = [0.347_854_845_137_453_9, 0.652_145_154_862_546_1, 0.652_145_154_862_546_1, 0.347_854_845_137_453_9];

/// What a control point asks of the curve through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpiroKind {
    /// Smooth: tangent, curvature and the curvature's first two derivatives continuous.
    G4,
    /// Smooth: tangent and curvature continuous.
    G2,
    /// A corner: the curve may change direction.
    Corner,
    /// A curve before the point, a straight line after it, joined tangentially.
    Left,
    /// A straight line before the point, a curve after it, joined tangentially.
    Right,
}

impl SpiroKind {
    /// The kind's name in commands and files: `g4`, `g2`, `corner`, `left`, `right`.
    pub fn as_str(self) -> &'static str {
        match self {
            SpiroKind::G4 => "g4",
            SpiroKind::G2 => "g2",
            SpiroKind::Corner => "corner",
            SpiroKind::Left => "left",
            SpiroKind::Right => "right",
        }
    }

    /// The kind named `name` (see [`SpiroKind::as_str`]).
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "g4" => SpiroKind::G4,
            "g2" => SpiroKind::G2,
            "corner" => SpiroKind::Corner,
            "left" => SpiroKind::Left,
            "right" => SpiroKind::Right,
            _ => return None,
        })
    }
}

/// One control point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpiroPoint {
    pub at: Point,
    pub kind: SpiroKind,
}

impl SpiroPoint {
    pub fn new(x: f64, y: f64, kind: SpiroKind) -> Self {
        Self { at: Point::new(x, y), kind }
    }
}

/// Why no curve could be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpiroError {
    /// Fewer than 2 points (3 for a closed contour).
    TooFew,
    /// More than [`MAX_SPIRO_POINTS`] points.
    TooMany,
    /// A coordinate is not a finite number within [`MAX_SPIRO_COORDINATE`].
    BadCoordinate,
    /// Two neighbouring points are at the same place.
    Coincident,
    /// The conditions have no solution the solver could find.
    NoSolution,
}

impl fmt::Display for SpiroError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SpiroError::TooFew => "a spiro contour needs at least 2 points, 3 when it is closed",
            SpiroError::TooMany => "a spiro contour has too many points",
            SpiroError::BadCoordinate => "a spiro point's coordinates must be finite numbers within range",
            SpiroError::Coincident => "two neighbouring spiro points are at the same place",
            SpiroError::NoSolution => "no smooth curve goes through these spiro points",
        })
    }
}

impl std::error::Error for SpiroError {}

/// One segment in the normalized frame (the contour moved and scaled so its mean chord is 1).
#[derive(Clone, Copy, Debug)]
struct Seg {
    start: Point,
    end: Point,
    /// Chord length and direction.
    len: f64,
    angle: f64,
    straight: bool,
    k: [f64; 4],
}

/// What one evaluation of a segment's spiral gives: the direction and length of its chord in
/// the unit frame, and their derivatives by each `k`.
#[derive(Clone, Copy, Debug, Default)]
struct Chord {
    alpha: f64,
    ln_c: f64,
    d_alpha: [f64; 4],
    d_ln_c: [f64; 4],
}

/// A solved spiro contour.
#[derive(Clone, Debug)]
pub struct Spline {
    closed: bool,
    origin: Vec2,
    scale: f64,
    segs: Vec<Seg>,
}

/// The cubic Béziers through `points` within `tolerance` font units (see [`Spline::to_bez`]).
pub fn to_bez(points: &[SpiroPoint], closed: bool, tolerance: f64) -> Result<BezPath, SpiroError> {
    Ok(Spline::solve(points, closed)?.to_bez(tolerance))
}

impl Spline {
    /// The curve through `points`. A closed contour's last segment goes from the last point back
    /// to the first.
    pub fn solve(points: &[SpiroPoint], closed: bool) -> Result<Spline, SpiroError> {
        let n = points.len();
        if n < if closed { 3 } else { 2 } {
            return Err(SpiroError::TooFew);
        }
        if n > MAX_SPIRO_POINTS {
            return Err(SpiroError::TooMany);
        }
        let usable = |v: f64| v.is_finite() && v.abs() <= MAX_SPIRO_COORDINATE;
        if points.iter().any(|p| !usable(p.at.x) || !usable(p.at.y)) {
            return Err(SpiroError::BadCoordinate);
        }
        let count = if closed { n } else { n - 1 };
        let at = |i: usize| points.get(i % n).map(|p| p.at).unwrap_or(Point::ZERO);
        let total: f64 = (0..count).map(|i| (at(i + 1) - at(i)).hypot()).sum();
        let scale = total / count as f64;
        if !scale.is_finite() || scale <= 0.0 {
            return Err(SpiroError::Coincident);
        }
        let origin = at(0).to_vec2();
        let local = |p: Point| ((p.to_vec2() - origin) / scale).to_point();
        let kinds: Vec<SpiroKind> = (0..n).map(|i| effective_kind(points, i, closed)).collect();
        let kind = |i: usize| kinds.get(i % n).copied().unwrap_or(SpiroKind::Corner);
        let mut segs = Vec::with_capacity(count);
        for i in 0..count {
            let (a, b) = (local(at(i)), local(at(i + 1)));
            let d = b - a;
            let len = d.hypot();
            if len.is_nan() || len <= 1e-9 {
                return Err(SpiroError::Coincident);
            }
            let (from, to) = (kind(i), kind(i + 1));
            let straight = from == SpiroKind::Left || to == SpiroKind::Right || (from == SpiroKind::Corner && to == SpiroKind::Corner);
            segs.push(Seg { start: a, end: b, len, angle: d.y.atan2(d.x), straight, k: [0.0; 4] });
        }
        let mut spline = Spline { closed, origin, scale, segs };
        spline.newton(&kinds)?;
        Ok(spline)
    }

    /// The number of segments: one per point when closed, one fewer when open.
    pub fn segment_count(&self) -> usize {
        self.segs.len()
    }

    /// The contour is closed.
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// The point a share `t` (0 to 1) of the way along `segment`'s arc length, in font units.
    pub fn point(&self, segment: usize, t: f64) -> Option<Point> {
        let seg = self.segs.get(segment)?;
        let p = local_point(seg, t.clamp(0.0, 1.0) - 0.5)?;
        Some(self.world(p))
    }

    /// The tangent's direction in radians at a share `t` of the way along `segment`.
    pub fn tangent_angle(&self, segment: usize, t: f64) -> Option<f64> {
        let seg = self.segs.get(segment)?;
        let ch = chord(&seg.k, false)?;
        Some(seg.angle - ch.alpha + theta(&seg.k, t.clamp(0.0, 1.0) - 0.5))
    }

    /// The signed curvature (1 / font units, positive turning left) at a share `t` of the way
    /// along `segment`.
    pub fn curvature(&self, segment: usize, t: f64) -> Option<f64> {
        let seg = self.segs.get(segment)?;
        let ch = chord(&seg.k, false)?;
        let arc = seg.len / ch.ln_c.exp();
        let k = theta_d(&seg.k, t.clamp(0.0, 1.0) - 0.5, 1) / arc / self.scale;
        k.is_finite().then_some(k)
    }

    /// The contour as cubic Béziers (and lines for straight segments), each cubic within
    /// `tolerance` font units of the spiral. A tolerance that is not a positive number uses
    /// [`DEFAULT_TOLERANCE`].
    pub fn to_bez(&self, tolerance: f64) -> BezPath {
        let tolerance = if tolerance.is_finite() && tolerance > 0.0 { tolerance } else { DEFAULT_TOLERANCE };
        let tol = tolerance / self.scale;
        let mut out = BezPath::new();
        let Some(first) = self.segs.first() else { return out };
        out.move_to(self.world(first.start));
        for seg in &self.segs {
            if seg.straight {
                out.line_to(self.world(seg.end));
                continue;
            }
            let mut pieces = Vec::new();
            fit(seg, -0.5, 0.5, tol, 0, &mut pieces);
            let last = pieces.len().saturating_sub(1);
            for (i, c) in pieces.into_iter().enumerate() {
                // The last piece ends exactly on the control point.
                let end = if i == last { seg.end } else { c.p3 };
                out.curve_to(self.world(c.p1), self.world(c.p2), self.world(end));
            }
        }
        if self.closed {
            out.close_path();
        }
        out
    }

    fn world(&self, p: Point) -> Point {
        (p.to_vec2() * self.scale + self.origin).to_point()
    }

    /// Finds every segment's `k` by damped Newton steps.
    fn newton(&mut self, kinds: &[SpiroKind]) -> Result<(), SpiroError> {
        let mut rows = self.equations(kinds).ok_or(SpiroError::NoSolution)?;
        let unknowns = 4 * self.segs.len();
        if rows.len() != unknowns {
            return Err(SpiroError::NoSolution);
        }
        for _ in 0..MAX_NEWTON_STEPS {
            let size = norm(&rows);
            if size.max <= SOLVED {
                return Ok(());
            }
            let matrix: Vec<Vec<f64>> = rows
                .iter()
                .map(|r| {
                    let mut dense = vec![0.0; unknowns];
                    for &(col, v) in &r.grad {
                        if let Some(x) = dense.get_mut(col) {
                            *x += v;
                        }
                    }
                    dense
                })
                .collect();
            let rhs: Vec<f64> = rows.iter().map(|r| -r.value).collect();
            let mut step = solve_linear(matrix, rhs).ok_or(SpiroError::NoSolution)?;
            let biggest = step.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
            if biggest > MAX_STEP {
                step.iter_mut().for_each(|v| *v *= MAX_STEP / biggest);
            }
            let start: Vec<[f64; 4]> = self.segs.iter().map(|s| s.k).collect();
            let mut lambda = 1.0;
            let mut accepted = None;
            for _ in 0..MAX_HALVINGS {
                for (i, seg) in self.segs.iter_mut().enumerate() {
                    let base = start.get(i).copied().unwrap_or_default();
                    for (m, k) in seg.k.iter_mut().enumerate() {
                        *k = base.get(m).copied().unwrap_or(0.0) + lambda * step.get(4 * i + m).copied().unwrap_or(0.0);
                    }
                }
                if let Some(trial) = self.equations(kinds)
                    && norm(&trial).sum < size.sum
                {
                    accepted = Some(trial);
                    break;
                }
                lambda *= 0.5;
            }
            match accepted {
                Some(trial) => rows = trial,
                None => {
                    for (seg, k) in self.segs.iter_mut().zip(start) {
                        seg.k = k;
                    }
                    return if size.max <= 1e3 * SOLVED { Ok(()) } else { Err(SpiroError::NoSolution) };
                }
            }
        }
        if norm(&rows).max <= SOLVED * 1e3 { Ok(()) } else { Err(SpiroError::NoSolution) }
    }

    /// Every condition with its value now and its gradient by the unknowns (`4·segment + m` is
    /// `k_m` of a segment). `None` when a segment turns too fast to evaluate.
    fn equations(&self, kinds: &[SpiroKind]) -> Option<Vec<Row>> {
        let chords: Vec<Chord> =
            self.segs.iter().map(|s| if s.straight { Some(Chord::default()) } else { chord(&s.k, true) }).collect::<Option<_>>()?;
        let count = self.segs.len();
        let n = kinds.len();
        let mut rows = Vec::with_capacity(4 * count);
        let side = |i: usize| -> Option<Side<'_>> {
            let seg = self.segs.get(i)?;
            Some(Side { index: i, seg, chord: chords.get(i).copied().unwrap_or_default() })
        };
        for (i, seg) in self.segs.iter().enumerate() {
            if seg.straight {
                for m in 0..4 {
                    rows.push(Row { value: seg.k.get(m).copied().unwrap_or(0.0), grad: vec![(4 * i + m, 1.0)] });
                }
            }
        }
        for j in 0..n {
            let kind = kinds.get(j).copied().unwrap_or(SpiroKind::Corner);
            let incoming = if self.closed { Some((j + n - 1) % n) } else { j.checked_sub(1) };
            let outgoing = if self.closed || j + 1 < n { Some(j) } else { None };
            let (a, b) = (incoming.and_then(side), outgoing.and_then(side));
            let (curve_a, curve_b) = (a.filter(|s| !s.seg.straight), b.filter(|s| !s.seg.straight));
            let turn = |a: &Side<'_>, b: &Side<'_>| wrap(b.seg.angle - a.seg.angle);
            match (curve_a, curve_b) {
                (Some(a), Some(b)) => match kind {
                    SpiroKind::G4 => {
                        rows.push(a.at(END, 0).minus(&b.at(START, 0), turn(&a, &b)));
                        for d in 1..4 {
                            rows.push(a.at(END, d).minus(&b.at(START, d), 0.0));
                        }
                    }
                    SpiroKind::G2 => {
                        rows.push(a.at(END, 0).minus(&b.at(START, 0), turn(&a, &b)));
                        rows.push(a.at(END, 1).minus(&b.at(START, 1), 0.0));
                        rows.push(a.raw(END, 3));
                        rows.push(b.raw(START, 3));
                    }
                    _ => rows.extend([a.raw(END, 2), a.raw(END, 3), b.raw(START, 2), b.raw(START, 3)]),
                },
                // A curve into a straight segment: tangent to it, unless the point is a corner.
                (Some(a), None) => match b {
                    Some(line) if kind != SpiroKind::Corner => {
                        let mut r = a.at(END, 0);
                        r.value -= turn(&a, &line);
                        rows.extend([r, a.raw(END, 3)]);
                    }
                    _ => rows.extend([a.raw(END, 2), a.raw(END, 3)]),
                },
                // A straight segment into a curve.
                (None, Some(b)) => match a {
                    Some(line) if kind != SpiroKind::Corner => {
                        let mut r = b.at(START, 0);
                        r.value += turn(&line, &b);
                        rows.extend([r, b.raw(START, 3)]);
                    }
                    _ => rows.extend([b.raw(START, 2), b.raw(START, 3)]),
                },
                (None, None) => {}
            }
        }
        Some(rows)
    }
}

/// The kind point `i` acts as: the ends of an open contour are corners unless they start or end
/// a straight line.
fn effective_kind(points: &[SpiroPoint], i: usize, closed: bool) -> SpiroKind {
    let kind = points.get(i).map_or(SpiroKind::Corner, |p| p.kind);
    let end = !closed && (i == 0 || i + 1 == points.len());
    match kind {
        SpiroKind::G4 | SpiroKind::G2 if end => SpiroKind::Corner,
        k => k,
    }
}

const START: f64 = -0.5;
const END: f64 = 0.5;

/// One condition: its value now and its gradient as (unknown, derivative) pairs.
#[derive(Clone, Debug)]
struct Row {
    value: f64,
    grad: Vec<(usize, f64)>,
}

impl Row {
    /// `self − other − constant`.
    fn minus(mut self, other: &Row, constant: f64) -> Row {
        self.value -= other.value + constant;
        self.grad.extend(other.grad.iter().map(|&(c, v)| (c, -v)));
        self
    }
}

/// A segment as one side of a control point.
#[derive(Clone, Copy)]
struct Side<'a> {
    index: usize,
    seg: &'a Seg,
    chord: Chord,
}

impl Side<'_> {
    /// At `s`: for `d = 0` the tangent angle relative to the chord; for `d` from 1 to 3 the
    /// `d − 1`-th derivative of curvature by arc length, in the normalized frame.
    fn at(&self, s: f64, d: usize) -> Row {
        let k = &self.seg.k;
        let c = &self.chord;
        let mut grad = Vec::with_capacity(4);
        let value = if d == 0 {
            for m in 0..4 {
                let da = c.d_alpha.get(m).copied().unwrap_or(0.0);
                grad.push((4 * self.index + m, d_theta(s, 0, m) - da));
            }
            theta(k, s) - c.alpha
        } else {
            // Arc length is the chord over the unit-frame chord, so each derivative by arc length
            // scales by (c / len).
            let f = ((c.ln_c).exp() / self.seg.len).powi(d as i32);
            let v = theta_d(k, s, d);
            for m in 0..4 {
                let dl = c.d_ln_c.get(m).copied().unwrap_or(0.0);
                grad.push((4 * self.index + m, d_theta(s, d, m) * f + v * f * d as f64 * dl));
            }
            v * f
        };
        Row { value, grad }
    }

    /// The `d`-th derivative of `θ` at `s` in the unit frame: zero conditions on it need no
    /// scaling.
    fn raw(&self, s: f64, d: usize) -> Row {
        let grad = (0..4).map(|m| (4 * self.index + m, d_theta(s, d, m))).collect();
        Row { value: theta_d(&self.seg.k, s, d), grad }
    }
}

#[derive(Clone, Copy)]
struct Norm {
    max: f64,
    sum: f64,
}

fn norm(rows: &[Row]) -> Norm {
    rows.iter().fold(Norm { max: 0.0, sum: 0.0 }, |n, r| {
        let v = if r.value.is_finite() { r.value.abs() } else { f64::INFINITY };
        Norm { max: n.max.max(v), sum: n.sum + v * v }
    })
}

/// `a` brought into (−π, π].
fn wrap(a: f64) -> f64 {
    let mut a = a % (2.0 * PI);
    if a > PI {
        a -= 2.0 * PI;
    } else if a <= -PI {
        a += 2.0 * PI;
    }
    a
}

fn theta(k: &[f64; 4], s: f64) -> f64 {
    let [a, b, c, d] = *k;
    s * (a + s * (b / 2.0 + s * (c / 6.0 + s * d / 24.0)))
}

/// The `d`-th derivative of `θ` at `s` (`d` from 0 to 3).
fn theta_d(k: &[f64; 4], s: f64, d: usize) -> f64 {
    let [a, b, c, e] = *k;
    match d {
        0 => theta(k, s),
        1 => a + s * (b + s * (c / 2.0 + s * e / 6.0)),
        2 => b + s * (c + s * e / 2.0),
        _ => c + s * e,
    }
}

/// The derivative of `θ^(d)(s)` by `k_m`: `s^p / p!` with `p = m + 1 − d`.
fn d_theta(s: f64, d: usize, m: usize) -> f64 {
    let Some(p) = (m + 1).checked_sub(d) else { return 0.0 };
    let fact = match p {
        0 | 1 => 1.0,
        2 => 2.0,
        3 => 6.0,
        _ => 24.0,
    };
    s.powi(p as i32) / fact
}

/// The most `θ'` can be over the segment, for choosing the number of integration panels.
fn turn_rate(k: &[f64; 4]) -> f64 {
    let [a, b, c, d] = *k;
    a.abs() + b.abs() / 2.0 + c.abs() / 8.0 + d.abs() / 48.0
}

/// `∫ e^{iθ(s)} ds` from `from` to `to`, and with `grad` its derivatives by each `k`. `None`
/// when the segment turns too fast.
fn integrate(k: &[f64; 4], from: f64, to: f64, grad: bool) -> Option<(Vec2, [Vec2; 4])> {
    let rate = turn_rate(k);
    if !rate.is_finite() || rate > MAX_TURN_RATE {
        return None;
    }
    let width = to - from;
    let panels = (((8.0 + 2.0 * rate) * width.abs()).ceil() as usize).clamp(1, MAX_PANELS);
    let h = width / panels as f64;
    let mut z = Vec2::ZERO;
    let mut dz = [Vec2::ZERO; 4];
    for p in 0..panels {
        let mid = from + h * (p as f64 + 0.5);
        for (x, w) in GL_X.iter().zip(GL_W.iter()) {
            let s = mid + x * h / 2.0;
            let w = w * h / 2.0;
            let (sin, cos) = theta(k, s).sin_cos();
            z += Vec2::new(cos, sin) * w;
            if grad {
                // d/dk_m e^{iθ} = i·e^{iθ}·s^{m+1}/(m+1)!
                for (m, slot) in dz.iter_mut().enumerate() {
                    *slot += Vec2::new(-sin, cos) * (w * d_theta(s, 0, m));
                }
            }
        }
    }
    Some((z, dz))
}

/// The chord of the unit-length spiral `k`: its angle and log length, with derivatives when
/// `grad`.
fn chord(k: &[f64; 4], grad: bool) -> Option<Chord> {
    let (z, dz) = integrate(k, START, END, grad)?;
    let c2 = z.hypot2();
    if c2.is_nan() || c2 <= 1e-24 {
        return None;
    }
    let mut out = Chord { alpha: z.y.atan2(z.x), ln_c: 0.5 * c2.ln(), ..Chord::default() };
    // dz / z = d ln c + i dα
    for m in 0..4 {
        let d = dz.get(m).copied().unwrap_or(Vec2::ZERO);
        let q = Vec2::new(d.x * z.x + d.y * z.y, d.y * z.x - d.x * z.y) / c2;
        if let (Some(l), Some(a)) = (out.d_ln_c.get_mut(m), out.d_alpha.get_mut(m)) {
            *l = q.x;
            *a = q.y;
        }
    }
    Some(out)
}

/// The point at `s` of a solved segment, in the normalized frame.
fn local_point(seg: &Seg, s: f64) -> Option<Point> {
    if seg.straight {
        return Some(seg.start.lerp(seg.end, s + 0.5));
    }
    let ch = chord(&seg.k, false)?;
    let arc = seg.len / ch.ln_c.exp();
    let (part, _) = integrate(&seg.k, START, s, false)?;
    let rot = seg.angle - ch.alpha;
    let (sin, cos) = rot.sin_cos();
    let v = Vec2::new(part.x * cos - part.y * sin, part.x * sin + part.y * cos) * arc;
    let p = seg.start + v;
    (p.x.is_finite() && p.y.is_finite()).then_some(p)
}

/// Cubics for the part of `seg` from `s0` to `s1`, each within `tol` of the spiral, appended to
/// `out` (in the normalized frame).
fn fit(seg: &Seg, s0: f64, s1: f64, tol: f64, depth: usize, out: &mut Vec<CubicBez>) {
    let Some(cubic) = piece(seg, s0, s1) else { return };
    let turn = (theta(&seg.k, s1) - theta(&seg.k, s0)).abs();
    let split = depth < MAX_FIT_DEPTH
        && (turn > FRAC_PI_2
            || [0.25, 0.5, 0.75].iter().any(|f| local_point(seg, s0 + (s1 - s0) * f).is_none_or(|p| cubic.nearest(p, 1e-9).distance_sq > tol * tol)));
    if split {
        let mid = 0.5 * (s0 + s1);
        fit(seg, s0, mid, tol, depth + 1, out);
        fit(seg, mid, s1, tol, depth + 1, out);
    } else {
        out.push(cubic);
    }
}

/// One cubic for the part of `seg` from `s0` to `s1`: the end points and tangents of the spiral,
/// with the handle length a circular arc of the same turn would have.
fn piece(seg: &Seg, s0: f64, s1: f64) -> Option<CubicBez> {
    let ch = chord(&seg.k, false)?;
    let arc = seg.len / ch.ln_c.exp();
    let (p0, p3) = (local_point(seg, s0)?, local_point(seg, s1)?);
    let rot = seg.angle - ch.alpha;
    let (t0, t1) = (rot + theta(&seg.k, s0), rot + theta(&seg.k, s1));
    let length = arc * (s1 - s0);
    let turn = t1 - t0;
    let h = if turn.abs() < 1e-9 { length / 3.0 } else { length * 4.0 / 3.0 * (turn / 4.0).tan() / turn };
    let (u0, u1) = (Vec2::new(t0.cos(), t0.sin()), Vec2::new(t1.cos(), t1.sin()));
    let c = CubicBez::new(p0, p0 + u0 * h, p3 - u1 * h, p3);
    [c.p1, c.p2].iter().all(|p| p.x.is_finite() && p.y.is_finite()).then_some(c)
}

/// Solves `a · x = b` by Gaussian elimination with partial pivoting. `None` when `a` is
/// singular.
fn solve_linear(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    let scale = a.iter().flatten().fold(0.0_f64, |m, v| m.max(v.abs()));
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    for col in 0..n {
        let (pivot, size) =
            a.iter().enumerate().skip(col).map(|(r, row)| (r, row.get(col).copied().unwrap_or(0.0).abs())).max_by(|x, y| x.1.total_cmp(&y.1))?;
        if size.is_nan() || size <= 1e-13 * scale {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        let (top, rest) = a.split_at_mut(col + 1);
        let row = top.last()?;
        let p = *row.get(col)?;
        let bp = *b.get(col)?;
        for (offset, other) in rest.iter_mut().enumerate() {
            let f = other.get(col).copied().unwrap_or(0.0) / p;
            if f.abs() < 1e-300 {
                continue;
            }
            for (x, y) in other.iter_mut().zip(row.iter()).skip(col) {
                *x -= f * y;
            }
            if let Some(v) = b.get_mut(col + 1 + offset) {
                *v -= f * bp;
            }
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let row = a.get(r)?;
        let done: f64 = row.iter().zip(x.iter()).skip(r + 1).map(|(a, x)| a * x).sum();
        let v = (b.get(r)? - done) / row.get(r)?;
        if !v.is_finite() {
            return None;
        }
        *x.get_mut(r)? = v;
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use kurbo::{ParamCurve, PathEl, PathSeg};

    use super::*;

    fn pts(kind: SpiroKind, xy: &[(f64, f64)]) -> Vec<SpiroPoint> {
        xy.iter().map(|&(x, y)| SpiroPoint::new(x, y, kind)).collect()
    }

    fn samples(s: &Spline) -> Vec<Point> {
        (0..s.segment_count()).flat_map(|i| (0..=16).map(move |t| s.point(i, t as f64 / 16.0).unwrap())).collect()
    }

    /// The largest distance from any of `points` to the nearest segment of `path`.
    fn worst_distance(path: &BezPath, points: &[Point]) -> f64 {
        let segs: Vec<PathSeg> = path.segments().collect();
        points.iter().map(|p| segs.iter().map(|s| s.nearest(*p, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min)).fold(0.0, f64::max)
    }

    fn angle_gap(a: f64, b: f64) -> f64 {
        wrap(a - b).abs()
    }

    #[test]
    fn four_g2_points_on_a_circle_give_the_circle() {
        let points = pts(SpiroKind::G2, &[(100.0, 0.0), (0.0, 100.0), (-100.0, 0.0), (0.0, -100.0)]);
        let spline = Spline::solve(&points, true).unwrap();
        for p in samples(&spline) {
            assert!((p.to_vec2().hypot() - 100.0).abs() < 1e-6, "{p:?}");
        }
        for i in 0..4 {
            assert!((spline.curvature(i, 0.3).unwrap() - 0.01).abs() < 1e-9);
        }
        let bez = spline.to_bez(0.1);
        assert_eq!(bez.elements().last(), Some(&PathEl::ClosePath));
        for seg in bez.segments() {
            for t in 0..=8 {
                let p = seg.eval(t as f64 / 8.0);
                assert!((p.to_vec2().hypot() - 100.0).abs() < 0.1, "{p:?}");
            }
        }
    }

    #[test]
    fn a_closed_g2_curve_through_four_uneven_points_is_smooth_and_converts_to_cubics() {
        let xy = [(0.0, 0.0), (300.0, 40.0), (260.0, 260.0), (-20.0, 220.0)];
        let points = pts(SpiroKind::G2, &xy);
        let spline = Spline::solve(&points, true).unwrap();
        for i in 0..4 {
            let next = (i + 1) % 4;
            // Through the points…
            let (x, y) = xy[i];
            assert!((spline.point(i, 0.0).unwrap() - Point::new(x, y)).hypot() < 1e-6);
            assert!((spline.point(i, 1.0).unwrap() - Point::new(xy[next].0, xy[next].1)).hypot() < 1e-6);
            // …with tangent and curvature continuous at each of them.
            assert!(angle_gap(spline.tangent_angle(i, 1.0).unwrap(), spline.tangent_angle(next, 0.0).unwrap()) < 1e-8);
            let (k_in, k_out) = (spline.curvature(i, 1.0).unwrap(), spline.curvature(next, 0.0).unwrap());
            assert!((k_in - k_out).abs() < 1e-8 * k_in.abs().max(1e-3), "{k_in} {k_out}");
        }
        let bez = spline.to_bez(0.25);
        assert!(bez.elements().iter().all(|e| !matches!(e, PathEl::LineTo(_))));
        assert!(worst_distance(&bez, &samples(&spline)) < 0.25 + 1e-6);
    }

    #[test]
    fn three_g4_points_on_an_arc_give_the_arc() {
        let points = pts(SpiroKind::G4, &[(100.0, 0.0), (0.0, 100.0), (-100.0, 0.0)]);
        let spline = Spline::solve(&points, false).unwrap();
        assert_eq!(spline.segment_count(), 2);
        for p in samples(&spline) {
            assert!((p.to_vec2().hypot() - 100.0).abs() < 1e-6, "{p:?}");
        }
    }

    #[test]
    fn line_transitions_give_straight_sides_and_tangent_curves() {
        // A stadium: two straight sides between `left` and `right` points, round ends.
        let points = vec![
            SpiroPoint::new(0.0, 0.0, SpiroKind::Left),
            SpiroPoint::new(200.0, 0.0, SpiroKind::Right),
            SpiroPoint::new(300.0, 100.0, SpiroKind::G2),
            SpiroPoint::new(200.0, 200.0, SpiroKind::Left),
            SpiroPoint::new(0.0, 200.0, SpiroKind::Right),
            SpiroPoint::new(-100.0, 100.0, SpiroKind::G4),
        ];
        let spline = Spline::solve(&points, true).unwrap();
        for t in 0..=8 {
            let t = t as f64 / 8.0;
            for (seg, centre) in [(1, (200.0, 100.0)), (2, (200.0, 100.0)), (4, (0.0, 100.0)), (5, (0.0, 100.0))] {
                let p = spline.point(seg, t).unwrap();
                assert!(((p - Point::new(centre.0, centre.1)).hypot() - 100.0).abs() < 1e-6, "{seg} {p:?}");
            }
        }
        let bez = spline.to_bez(0.25);
        let lines = bez.elements().iter().filter(|e| matches!(e, PathEl::LineTo(_))).count();
        assert_eq!(lines, 2);
    }

    #[test]
    fn corners_only_give_a_polygon() {
        let points = pts(SpiroKind::Corner, &[(0.0, 0.0), (100.0, 0.0), (50.0, 80.0)]);
        let bez = to_bez(&points, true, 0.25).unwrap();
        let els = bez.elements();
        assert_eq!(els.len(), 5);
        assert!(els.iter().skip(1).take(3).all(|e| matches!(e, PathEl::LineTo(_))));
    }

    #[test]
    fn bad_input_is_refused() {
        let g = SpiroKind::G2;
        assert_eq!(Spline::solve(&pts(g, &[(0.0, 0.0), (1.0, 1.0)]), true).unwrap_err(), SpiroError::TooFew);
        assert_eq!(Spline::solve(&pts(g, &[(0.0, 0.0)]), false).unwrap_err(), SpiroError::TooFew);
        assert_eq!(Spline::solve(&pts(g, &[(0.0, 0.0), (f64::NAN, 1.0), (3.0, 3.0)]), false).unwrap_err(), SpiroError::BadCoordinate);
        assert_eq!(Spline::solve(&pts(g, &[(0.0, 0.0), (1e300, 1.0), (3.0, 3.0)]), false).unwrap_err(), SpiroError::BadCoordinate);
        assert_eq!(Spline::solve(&pts(g, &[(0.0, 0.0), (10.0, 0.0), (10.0, 0.0)]), false).unwrap_err(), SpiroError::Coincident);
        let many: Vec<(f64, f64)> = (0..=MAX_SPIRO_POINTS).map(|i| (i as f64, (i % 2) as f64)).collect();
        assert_eq!(Spline::solve(&pts(g, &many), false).unwrap_err(), SpiroError::TooMany);
        // A zigzag no smooth curve can follow sensibly still ends in a result or an error.
        let zig: Vec<(f64, f64)> = (0..40).map(|i| (i as f64 * 0.001, if i % 2 == 0 { 0.0 } else { 500.0 })).collect();
        let _ = Spline::solve(&pts(SpiroKind::G4, &zig), true);
    }
}
