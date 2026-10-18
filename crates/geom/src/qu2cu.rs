//! Quadratic splines to cubic curves (`outlines.convertCurves`, M12.29).
//!
//! Ported from fontTools' `qu2cu` (fontTools/qu2cu/qu2cu.py, fontTools 4.66.1; Apache License
//! 2.0, Copyright 2023 Google Inc. and Behdad Esfahbod), with `splitCubicAtTC` and its helpers
//! from fontTools/misc/bezierTools.py (MIT). The arithmetic keeps the reference's order of
//! operations so the results are the same to the last bit.
//!
//! The quadratic segments of a run of splines are raised to cubics; a dynamic programme then picks,
//! for every prefix of the run, the cheapest way to write it: each segment kept quadratic (its
//! cost the points it needs), or a stretch of segments merged into one cubic when the cubic,
//! split back at the reconstructed parameters, stays within the tolerance at the knots and
//! everywhere between. Sharp corners always stay on-curve points.
//!
//! Differences from the reference, all on inputs it would mishandle:
//! - a merge whose parameters come out infinite or not a number is treated as impossible, like
//!   a division by zero;
//! - the interior error test stops splitting at depth [`MAX_FIT_DEPTH`] and reports a miss;
//! - the work is bounded by [`MAX_WORK`]; a run that needs more fails with [`Qu2CuError::TooMuchWork`].

/// Most merge attempts, counted in quadratic segments examined, for one call. The search is
/// cubic in the run length, so a run of a few thousand segments without corners reaches it.
pub const MAX_WORK: usize = 50_000_000;
/// Deepest subdivision of the interior error test.
pub const MAX_FIT_DEPTH: usize = 64;

/// Why a conversion failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Qu2CuError {
    /// The tolerance is not a positive finite number.
    BadTolerance,
    /// A spline has fewer than 3 points, or a coordinate is not finite.
    BadSpline,
    /// A spline does not start where the one before it ends.
    NotConnected,
    /// The search needs more than [`MAX_WORK`] steps.
    TooMuchWork,
}

impl std::fmt::Display for Qu2CuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BadTolerance => "the conversion error must be a positive number",
            Self::BadSpline => "a quadratic spline needs at least 3 finite points",
            Self::NotConnected => "the quadratic splines do not connect end to start",
            Self::TooMuchWork => "the run of quadratic curves is too long to convert",
        })
    }
}

impl std::error::Error for Qu2CuError {}

/// One output curve. Each starts where the one before it ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    /// Start, off-curve point, end.
    Quadratic([(f64, f64); 3]),
    /// Start, two off-curve points, end.
    Cubic([(f64, f64); 4]),
}

impl Curve {
    /// The points after the start: the arguments of a pen's `qCurveTo` or `curveTo`.
    pub fn tail(&self) -> &[(f64, f64)] {
        match self {
            Self::Quadratic(p) => p.get(1..).unwrap_or(&[]),
            Self::Cubic(p) => p.get(1..).unwrap_or(&[]),
        }
    }
}

/// A complex number as fontTools uses it for points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct C {
    re: f64,
    im: f64,
}

impl C {
    const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    fn finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
    fn pair(self) -> (f64, f64) {
        (self.re, self.im)
    }
}

impl std::ops::Add for C {
    type Output = C;
    fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
}
impl std::ops::Sub for C {
    type Output = C;
    fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
}
impl std::ops::Mul<f64> for C {
    type Output = C;
    fn mul(self, k: f64) -> C {
        C::new(self.re * k, self.im * k)
    }
}
impl std::ops::Div<f64> for C {
    type Output = C;
    fn div(self, k: f64) -> C {
        C::new(self.re / k, self.im / k)
    }
}

type Cubic = [C; 4];

/// Converts a run of connected quadratic splines (each: an on-curve point, one or more off-curve
/// points with implied on-curve points between them, an on-curve point) into quadratic and
/// cubic curves within `max_err` font units; `all_cubic` makes every curve cubic. Empty input
/// gives no curves.
pub fn quadratic_to_curves(quads: &[Vec<(f64, f64)>], max_err: f64, all_cubic: bool) -> Result<Vec<Curve>, Qu2CuError> {
    if quads.is_empty() {
        return Ok(Vec::new());
    }
    if !(max_err.is_finite() && max_err > 0.0) {
        return Err(Qu2CuError::BadTolerance);
    }
    if quads.iter().any(|s| s.len() < 3 || s.iter().any(|p| !(p.0.is_finite() && p.1.is_finite()))) {
        return Err(Qu2CuError::BadSpline);
    }
    let first = quads.first().and_then(|s| s.first()).ok_or(Qu2CuError::BadSpline)?;
    let mut q = vec![C::new(first.0, first.1)];
    let mut costs: Vec<usize> = vec![1];
    let mut cost = 1usize;
    for spline in quads {
        let p: Vec<C> = spline.iter().map(|&(x, y)| C::new(x, y)).collect();
        if q.last() != p.first() {
            return Err(Qu2CuError::NotConnected);
        }
        for _ in 0..p.len().saturating_sub(2) {
            cost += 1;
            costs.push(cost);
            costs.push(cost);
        }
        let with_implied = add_implicit_on_curves(&p);
        q.extend(with_implied.iter().skip(1));
        costs.pop();
        cost += 1;
        costs.push(cost);
    }
    spline_to_curves(&q, &costs, max_err, all_cubic)
}

/// `p` with the implied on-curve point inserted between every two off-curve points.
fn add_implicit_on_curves(p: &[C]) -> Vec<C> {
    let mut q = Vec::with_capacity(p.len().saturating_mul(2));
    let offs = p.len().saturating_sub(2);
    for (i, &pt) in p.iter().enumerate() {
        q.push(pt);
        if (1..offs).contains(&i)
            && let Some(&next) = p.get(i + 1)
        {
            q.push(pt + (next - pt) * 0.5);
        }
    }
    q
}

fn elevate_quadratic(p0: C, p1: C, p2: C) -> Cubic {
    let p1_2_3 = p1 * (2.0 / 3.0);
    [p0, p0 * (1.0 / 3.0) + p1_2_3, p2 * (1.0 / 3.0) + p1_2_3, p2]
}

/// Does the cubic stay within `tolerance` of the origin (its ends are not checked)?
fn cubic_farthest_fit_inside(p0: C, p1: C, p2: C, p3: C, tolerance: f64, depth: usize) -> bool {
    if p2.abs() <= tolerance && p1.abs() <= tolerance {
        return true;
    }
    if depth >= MAX_FIT_DEPTH {
        return false;
    }
    let mid = (p0 + (p1 + p2) * 3.0 + p3) * 0.125;
    if mid.abs() > tolerance {
        return false;
    }
    let deriv3 = (p3 + p2 - p1 - p0) * 0.125;
    cubic_farthest_fit_inside(p0, (p0 + p1) * 0.5, mid - deriv3, mid, tolerance, depth + 1)
        && cubic_farthest_fit_inside(mid, mid + deriv3, (p2 + p3) * 0.5, p3, tolerance, depth + 1)
}

/// One cubic with the end points and end tangents of `curves[start..start + n]`, and the
/// parameters at which it splits back into them; `None` where the reference divides by zero.
fn merge_curves(curves: &[Cubic], start: usize, n: usize) -> Option<(Cubic, Vec<f64>)> {
    let mut prod_ratio = 1.0f64;
    let mut sum_ratio = 1.0f64;
    let mut ts = Vec::with_capacity(n);
    ts.push(1.0f64);
    for k in 1..n {
        let ck = curves.get(start + k)?;
        let before = curves.get(start + k - 1)?;
        let ([_, _, b2, b3], [k0, k1, _, _]) = (*before, *ck);
        let denominator = (b3 - b2).abs();
        if denominator == 0.0 {
            return None;
        }
        let ratio = (k1 - k0).abs() / denominator;
        prod_ratio *= ratio;
        sum_ratio += prod_ratio;
        ts.push(sum_ratio);
    }
    ts.pop();
    let ts: Vec<f64> = ts.iter().map(|t| t / sum_ratio).collect();
    let first = curves.get(start)?;
    let last = curves.get((start + n).checked_sub(1)?)?;
    let ([p0, f1, _, _], [_, _, l2, p3]) = (*first, *last);
    let d1 = ts.first().copied().unwrap_or(1.0);
    let d2 = ts.last().map_or(1.0, |t| 1.0 - t);
    if d1 == 0.0 || d2 == 0.0 {
        return None;
    }
    let p1 = p0 + (f1 - p0) / d1;
    let p2 = p3 + (l2 - p3) / d2;
    let curve = [p0, p1, p2, p3];
    (curve.iter().all(|c| c.finite()) && ts.iter().all(|t| t.is_finite())).then_some((curve, ts))
}

/// The cubic split at each of `ts` (fontTools' `splitCubicAtTC`).
fn split_cubic_at_tc(c: &Cubic, ts: &[f64]) -> Vec<Cubic> {
    let [pt1, pt2, pt3, pt4] = *c;
    let cc = (pt2 - pt1) * 3.0;
    let b = (pt3 - pt2) * 3.0 - cc;
    let a = pt4 - pt1 - cc - b;
    let d = pt1;
    let mut bounds = Vec::with_capacity(ts.len() + 2);
    bounds.push(0.0);
    bounds.extend_from_slice(ts);
    bounds.push(1.0);
    bounds
        .windows(2)
        .filter_map(|w| match *w {
            [t1, t2] => {
                let delta = t2 - t1;
                let delta_2 = delta * delta;
                let delta_3 = delta * delta_2;
                let t1_2 = t1 * t1;
                let t1_3 = t1 * t1_2;
                let a1 = a * delta_3;
                let b1 = (a * 3.0 * t1 + b) * delta_2;
                let c1 = (b * 2.0 * t1 + cc + a * 3.0 * t1_2) * delta;
                let d1 = a * t1_3 + b * t1_2 + cc * t1 + d;
                let p2 = c1 * (1.0 / 3.0) + d1;
                let p3 = (b1 + c1) * (1.0 / 3.0) + p2;
                let p4 = a1 + b1 + c1 + d1;
                Some([d1, p2, p3, if t2 == 1.0 { pt4 } else { p4 }])
            }
            _ => None,
        })
        .collect()
}

/// A best way to write a prefix, compared as the reference compares its tuples.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Solution {
    num_points: usize,
    error: f64,
    start_index: usize,
    is_cubic: bool,
}

impl Solution {
    fn less(&self, o: &Solution) -> bool {
        (self.num_points, self.error, self.start_index, self.is_cubic).partial_cmp(&(o.num_points, o.error, o.start_index, o.is_cubic))
            == Some(std::cmp::Ordering::Less)
    }
}

/// `q`: on-curve and off-curve points alternating; `costs`: the points needed to write `q` up
/// to each index, implied on-curve points costing nothing.
fn spline_to_curves(q: &[C], costs: &[usize], tolerance: f64, all_cubic: bool) -> Result<Vec<Curve>, Qu2CuError> {
    if q.len() < 3 {
        return Err(Qu2CuError::BadSpline);
    }
    let elevated: Vec<Cubic> =
        (0..q.len() - 2).step_by(2).filter_map(|i| Some(elevate_quadratic(*q.get(i)?, *q.get(i + 1)?, *q.get(i + 2)?))).collect();
    let n = elevated.len();
    let mut forced = vec![false; n + 1];
    for i in 1..n {
        let (Some(prev), Some(cur)) = (elevated.get(i - 1), elevated.get(i)) else { continue };
        let ([_, _, p0, _], [p1, p2, _, _]) = (*prev, *cur);
        if (p1 - p0).abs() + (p2 - p1).abs() > tolerance + (p2 - p0).abs()
            && let Some(f) = forced.get_mut(i)
        {
            *f = true;
        }
    }
    let cost_at = |i: usize| costs.get(i).copied().unwrap_or(0);
    let mut sols = vec![Solution { num_points: 0, error: 0.0, start_index: 0, is_cubic: false }];
    let impossible = Solution { num_points: n * 3 + 1, error: 0.0, start_index: 1, is_cubic: false };
    let mut start = 0usize;
    let mut work = 0usize;
    for i in 1..=n {
        let mut best = impossible;
        for j in start..i {
            let Some(&Solution { num_points: j_count, error: j_error, .. }) = sols.get(j) else { break };
            if !all_cubic {
                let this_count = (cost_at(2 * i - 1) + 1).saturating_sub(cost_at(2 * j));
                let sol = Solution { num_points: j_count + this_count, error: j_error, start_index: i - j, is_cubic: false };
                if sol.less(&best) {
                    best = sol;
                }
                if this_count <= 3 {
                    continue;
                }
            }
            work = work.saturating_add(i - j);
            if work > MAX_WORK {
                return Err(Qu2CuError::TooMuchWork);
            }
            let Some((curve, ts)) = merge_curves(&elevated, j, i - j) else { continue };
            let mut error = 0.0f64;
            let mut reconstructed = Vec::with_capacity(i - j);
            for (k, piece) in split_cubic_at_tc(&curve, &ts).into_iter().enumerate() {
                let Some(orig) = elevated.get(j + k) else { break };
                let ([.., end], [.., orig_end]) = (piece, *orig);
                error = error.max((end - orig_end).abs());
                if error > tolerance {
                    break;
                }
                reconstructed.push(piece);
            }
            if error > tolerance {
                continue;
            }
            for (k, piece) in reconstructed.iter().enumerate() {
                let Some(orig) = elevated.get(j + k) else { break };
                let ([a0, a1, a2, a3], [b0, b1, b2, b3]) = (*piece, *orig);
                let [p0, p1, p2, p3] = [a0 - b0, a1 - b1, a2 - b2, a3 - b3];
                if !cubic_farthest_fit_inside(p0, p1, p2, p3, tolerance, 0) {
                    error = tolerance + 1.0;
                    break;
                }
            }
            if error > tolerance {
                continue;
            }
            let count = j_count + 3;
            let sol = Solution { num_points: count, error: j_error.max(error), start_index: i - j, is_cubic: true };
            if sol.less(&best) {
                best = sol;
            }
            if count == 3 {
                break;
            }
        }
        sols.push(best);
        if forced.get(i).copied().unwrap_or(false) {
            start = i;
        }
    }
    // Walk back from the end; `start_index` is at least 1, so this ends.
    let mut pieces = Vec::new();
    let mut i = sols.len() - 1;
    while i > 0 {
        let s = sols.get(i).copied().unwrap_or(impossible);
        let step = s.start_index.clamp(1, i);
        pieces.push((i, s.is_cubic));
        i -= step;
    }
    let mut curves = Vec::with_capacity(pieces.len());
    let mut j = 0usize;
    for (i, is_cubic) in pieces.into_iter().rev() {
        let merged = if is_cubic { merge_curves(&elevated, j, i - j) } else { None };
        match merged {
            Some(([c0, c1, c2, c3], _)) => curves.push(Curve::Cubic([c0.pair(), c1.pair(), c2.pair(), c3.pair()])),
            None => {
                for k in j..i {
                    let (Some(a), Some(b), Some(c)) = (q.get(2 * k), q.get(2 * k + 1), q.get(2 * k + 2)) else { continue };
                    curves.push(Curve::Quadratic([a.pair(), b.pair(), c.pair()]));
                }
            }
        }
        j = i;
    }
    Ok(curves)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(list: &[(u64, u64)]) -> Vec<(f64, f64)> {
        list.iter().map(|&(x, y)| (f64::from_bits(x), f64::from_bits(y))).collect()
    }

    // Expected values: fontTools 4.66.1 `quadratic_to_curves` (compiled) on the same input.
    #[test]
    fn a_converted_cubic_comes_back_as_the_reference_gives_it() {
        // cu2qu's spline for (0,0) (0,550) (450,1000) (1000,1000) at 1 unit.
        let spline = bits(&[
            (0x0, 0x0),
            (0x0, 0x4064a00000000000),
            (0x4059566666666666, 0x407d05999999999a),
            (0x4071d80000000001, 0x4086540000000000),
            (0x4080bd3333333335, 0x408c153333333333),
            (0x408a180000000001, 0x408f400000000000),
            (0x408f400000000000, 0x408f400000000000),
        ]);
        let out = quadratic_to_curves(&[spline], 1.0, false).unwrap();
        let expected =
            bits(&[(0x0, 0x0), (0x0, 0x40812ffffffffff9), (0x407c1fffffffffee, 0x408f400000000000), (0x408f400000000000, 0x408f400000000000)]);
        let Curve::Cubic(c) = out[0] else { panic!("expected one cubic, got {out:?}") };
        assert_eq!(out.len(), 1);
        assert_eq!(c.to_vec(), expected);
    }

    #[test]
    fn corners_short_splines_and_all_cubic() {
        let a = vec![(0.0, 0.0), (100.0, 200.0), (300.0, 300.0), (500.0, 250.0), (600.0, 0.0)];
        let b = vec![(600.0, 0.0), (500.0, -100.0), (300.0, -150.0), (100.0, -100.0), (0.0, -50.0)];
        let out = quadratic_to_curves(&[a.clone(), b.clone()], 0.5, false).unwrap();
        assert_eq!(out.len(), 6);
        assert_eq!(out[1], Curve::Quadratic([(200.0, 250.0), (300.0, 300.0), (400.0, 275.0)]));
        let out = quadratic_to_curves(&[a, b], 0.5, true).unwrap();
        assert_eq!(out.len(), 6);
        assert_eq!(
            out[2],
            Curve::Cubic([(400.0, 275.0), (466.66666666666663, 258.3333333333333), (533.3333333333333, 166.66666666666666), (600.0, 0.0)])
        );

        let one = vec![(0.0, 0.0), (50.0, 100.0), (100.0, 0.0)];
        assert_eq!(quadratic_to_curves(&[one.clone()], 1.0, false).unwrap(), vec![Curve::Quadratic([(0.0, 0.0), (50.0, 100.0), (100.0, 0.0)])]);
        assert_eq!(
            quadratic_to_curves(&[one], 1.0, true).unwrap(),
            vec![Curve::Cubic([(0.0, 0.0), (33.33333333333333, 66.66666666666666), (66.66666666666666, 66.66666666666666), (100.0, 0.0)])]
        );
        // Two off-curve points cost 3 points either way: they stay quadratic.
        let two = vec![(0.0, 0.0), (0.0, 300.0), (300.0, 600.0), (600.0, 600.0)];
        assert_eq!(
            quadratic_to_curves(&[two], 1.0, false).unwrap(),
            vec![Curve::Quadratic([(0.0, 0.0), (0.0, 300.0), (150.0, 450.0)]), Curve::Quadratic([(150.0, 450.0), (300.0, 600.0), (600.0, 600.0)])]
        );
    }

    #[test]
    fn bad_input_is_refused() {
        let s = vec![(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)];
        assert_eq!(quadratic_to_curves(&[s.clone()], 0.0, false), Err(Qu2CuError::BadTolerance));
        assert_eq!(quadratic_to_curves(&[vec![(0.0, 0.0), (1.0, 1.0)]], 1.0, false), Err(Qu2CuError::BadSpline));
        assert_eq!(quadratic_to_curves(&[vec![(0.0, 0.0), (f64::NAN, 1.0), (2.0, 0.0)]], 1.0, false), Err(Qu2CuError::BadSpline));
        assert_eq!(quadratic_to_curves(&[s.clone(), s], 1.0, false), Err(Qu2CuError::NotConnected));
        assert_eq!(quadratic_to_curves(&[], 1.0, false), Ok(Vec::new()));
        // Two equal points make the merge divide by zero: it is skipped, not a crash.
        let flat = vec![(0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)];
        assert!(quadratic_to_curves(&[flat], 1.0, false).is_ok());
    }
}
