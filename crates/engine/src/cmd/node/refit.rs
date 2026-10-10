//! One cubic segment fitted to the curve of several, for deleting nodes (`edit.delete`).
//!
//! The ends and the directions the curve leaves and enters them in are kept, so the neighbouring
//! segments meet the new one as they met the old ones. The two handle lengths are fitted by
//! least squares over samples of the old curve, alternating with a Newton step that moves each
//! sample's parameter to its nearest point (the method of P. J. Schneider, "An Algorithm for
//! Automatically Fitting Digitized Curves", Graphics Gems, 1990). When the old curve is one
//! cubic split in two, the unsplit cubic is tried as well. The work is bounded: a fixed number
//! of samples and iterations, whatever the input.

use fontcraft_geom::{CubicBez, ParamCurve, ParamCurveNearest, Piece, Point, Vec2};

use super::ops::{EPS, end_tangent, start_tangent};

/// Samples taken along each piece of the old curve.
const SAMPLES_PER_PIECE: usize = 24;
/// Most samples in all.
const MAX_SAMPLES: usize = 2_000;
/// Rounds of fitting and reparameterising.
const ITERATIONS: usize = 12;

/// The cubic that best follows `pieces` from the start of the first to the end of the last.
/// `None` when the curve has no length or no direction at an end.
pub(super) fn fit(pieces: &[Piece]) -> Option<CubicBez> {
    let (first, last) = (pieces.first()?, pieces.last()?);
    let (p0, p3) = (first.eval(0.0), last.eval(1.0));
    let (t1, t2) = (start_tangent(first)?, end_tangent(last)?);
    let samples = sample(pieces);
    let fitted = least_squares(&samples, p0, p3, t1, t2);
    let candidates = fitted.into_iter().chain(unsplit(pieces));
    let mut best: Option<(f64, CubicBez)> = None;
    for c in candidates {
        let e = deviation(&c, &samples);
        if e.is_finite() && best.as_ref().is_none_or(|(b, _)| e < *b) {
            best = Some((e, c));
        }
    }
    best.map(|(_, c)| c)
}

/// The largest distance from a sample to the curve `c`.
pub(super) fn deviation(c: &CubicBez, samples: &[Point]) -> f64 {
    samples.iter().map(|&p| c.nearest(p, 1e-6).distance_sq.sqrt()).fold(0.0, f64::max)
}

fn sample(pieces: &[Piece]) -> Vec<Point> {
    let per = (MAX_SAMPLES / pieces.len().max(1)).clamp(2, SAMPLES_PER_PIECE);
    let mut out = Vec::with_capacity(per * pieces.len() + 1);
    for (i, piece) in pieces.iter().enumerate() {
        let from = if i == 0 { 0 } else { 1 };
        for k in from..=per {
            out.push(piece.eval(k as f64 / per as f64));
        }
    }
    out
}

/// Parameters for the samples in proportion to the distance along them.
fn chord_params(samples: &[Point]) -> Option<Vec<f64>> {
    let mut acc = Vec::with_capacity(samples.len());
    let mut total = 0.0;
    let mut prev: Option<Point> = None;
    for &p in samples {
        if let Some(q) = prev {
            total += p.distance(q);
        }
        acc.push(total);
        prev = Some(p);
    }
    if !(total > EPS && total.is_finite()) {
        return None;
    }
    Some(acc.into_iter().map(|d| d / total).collect())
}

fn least_squares(samples: &[Point], p0: Point, p3: Point, t1: Vec2, t2: Vec2) -> Option<CubicBez> {
    let mut u = chord_params(samples)?;
    let mut best: Option<(f64, CubicBez)> = None;
    for _ in 0..ITERATIONS {
        let c = generate(samples, &u, p0, p3, t1, t2);
        let err = samples.iter().zip(&u).map(|(&p, &t)| c.eval(t).distance(p)).fold(0.0, f64::max);
        if err.is_finite() && best.as_ref().is_none_or(|(b, _)| err < *b) {
            best = Some((err, c));
        }
        for (t, &p) in u.iter_mut().zip(samples) {
            *t = newton(&c, p, *t);
        }
    }
    best.map(|(_, c)| c)
}

/// The cubic from `p0` to `p3` with handles along `t1` and `t2` whose lengths fit the samples
/// at parameters `u` best, by least squares.
fn generate(samples: &[Point], u: &[f64], p0: Point, p3: Point, t1: Vec2, t2: Vec2) -> CubicBez {
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (&d, &t) in samples.iter().zip(u) {
        let s = 1.0 - t;
        let (b0, b1, b2, b3) = (s * s * s, 3.0 * t * s * s, 3.0 * t * t * s, t * t * t);
        let (a1, a2) = (t1 * b1, t2 * b2);
        c00 += a1.dot(a1);
        c01 += a1.dot(a2);
        c11 += a2.dot(a2);
        let rest = d.to_vec2() - (p0.to_vec2() * (b0 + b1) + p3.to_vec2() * (b2 + b3));
        x0 += a1.dot(rest);
        x1 += a2.dot(rest);
    }
    let det = c00 * c11 - c01 * c01;
    let chord = p0.distance(p3);
    let (mut l1, mut l2) = if det.abs() > EPS { ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det) } else { (0.0, 0.0) };
    // A negative or vanishing length means the fit failed: fall back to a third of the chord.
    let floor = chord * 1e-6;
    if !(l1.is_finite() && l2.is_finite()) || l1 < floor || l2 < floor {
        (l1, l2) = (chord / 3.0, chord / 3.0);
    }
    CubicBez::new(p0, p0 + t1 * l1, p3 + t2 * l2, p3)
}

/// One Newton step moving `t` towards the parameter of the point of `c` nearest to `p`.
fn newton(c: &CubicBez, p: Point, t: f64) -> f64 {
    let s = 1.0 - t;
    let (p0, p1, p2, p3) = (c.p0.to_vec2(), c.p1.to_vec2(), c.p2.to_vec2(), c.p3.to_vec2());
    let d1 = ((p1 - p0) * (s * s) + (p2 - p1) * (2.0 * s * t) + (p3 - p2) * (t * t)) * 3.0;
    let d2 = ((p2 - p1 * 2.0 + p0) * s + (p3 - p2 * 2.0 + p1) * t) * 6.0;
    let diff = c.eval(t) - p;
    let den = d1.dot(d1) + diff.dot(d2);
    if den.abs() < EPS {
        return t;
    }
    let next = t - diff.dot(d1) / den;
    if next.is_finite() { next.clamp(0.0, 1.0) } else { t }
}

/// When `pieces` are two cubics that are one cubic split at some `t`, that cubic. The split
/// parameter follows from the handle lengths at the join (de Casteljau: they stand in the ratio
/// `t : 1 - t`).
fn unsplit(pieces: &[Piece]) -> Option<CubicBez> {
    let [Piece::Cubic(a), Piece::Cubic(b)] = pieces else { return None };
    let (la, lb) = (a.p3.distance(a.p2), b.p1.distance(b.p0));
    if la + lb <= EPS {
        return None;
    }
    let t = la / (la + lb);
    if !(t > 1e-6 && t < 1.0 - 1e-6) {
        return None;
    }
    let c1 = a.p0 + (a.p1 - a.p0) / t;
    let c2 = b.p3 + (b.p2 - b.p3) / (1.0 - t);
    Some(CubicBez::new(a.p0, c1, c2, b.p3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_cubic_is_put_back_together() {
        let c = CubicBez::new((0.0, 0.0), (0.0, 120.0), (180.0, 200.0), (300.0, 200.0));
        let (a, b) = (c.subsegment(0.0..0.37), c.subsegment(0.37..1.0));
        let fitted = fit(&[Piece::Cubic(a), Piece::Cubic(b)]).unwrap();
        let samples: Vec<Point> = (0..=100).map(|k| c.eval(k as f64 / 100.0)).collect();
        assert!(deviation(&fitted, &samples) < 1e-6);
    }

    #[test]
    fn a_quarter_circle_pair_is_refitted_closely() {
        // Two quarters of a circle of radius 100, from the right extreme over the top to the left.
        let k = 0.552_284_75 * 100.0;
        let q1 = CubicBez::new((100.0, 0.0), (100.0, k), (k, 100.0), (0.0, 100.0));
        let q2 = CubicBez::new((0.0, 100.0), (-k, 100.0), (-100.0, k), (-100.0, 0.0));
        let fitted = fit(&[Piece::Cubic(q1), Piece::Cubic(q2)]).unwrap();
        // The handles keep the vertical directions at both ends.
        assert!((fitted.p1.x - 100.0).abs() < 1e-9 && (fitted.p2.x + 100.0).abs() < 1e-9);
        let samples = sample(&[Piece::Cubic(q1), Piece::Cubic(q2)]);
        assert!(deviation(&fitted, &samples) < 10.0);
    }
}
