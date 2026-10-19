//! The region solver of partial instancing (M12.21), ported from fontTools'
//! `varLib.instancer.solver` (MIT, Copyright (c) 2017 Just van Rossum and the fontTools
//! authors): how a tent-shaped region is rebased when an axis range is limited.
//!
//! A region's contribution along one axis is a tent: 0 up to `start`, rising to 1 at `peak`,
//! back to 0 at `end`. Limiting the axis to `min..=max` with a new default `default` keeps the
//! contribution the same at every location inside the new range, with regions expressed in the
//! new normalised space. That takes a part folded into the new default (the "gain", which has no
//! region) and up to four tents, each with the factor its deltas are multiplied by.

/// One unit of F2Dot14, the smallest step a region coordinate can take.
const EPSILON: f64 = 1.0 / 16384.0;

/// A region on one normalised axis: start, peak, end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tent {
    pub start: f64,
    pub peak: f64,
    pub end: f64,
}

impl Tent {
    fn reverse_negate(self) -> Tent {
        Tent { start: -self.end, peak: -self.peak, end: -self.start }
    }
}

/// A limited axis range in normalised coordinates: minimum, default, maximum, all within −1..=1
/// and in order. `distance_negative` and `distance_positive` are the user-space lengths of the
/// axis below and above its old default; they decide how a range that crosses the old default
/// is renormalised.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisRange {
    pub min: f64,
    pub default: f64,
    pub max: f64,
    pub distance_negative: f64,
    pub distance_positive: f64,
}

impl AxisRange {
    /// A range with both distances 1.
    pub fn new(min: f64, default: f64, max: f64) -> Self {
        Self { min, default, max, distance_negative: 1.0, distance_positive: 1.0 }
    }

    fn reverse_negate(self) -> AxisRange {
        AxisRange {
            min: -self.max,
            default: -self.default,
            max: -self.min,
            distance_negative: self.distance_positive,
            distance_positive: self.distance_negative,
        }
    }

    fn is_valid(&self) -> bool {
        [self.min, self.default, self.max, self.distance_negative, self.distance_positive].iter().all(|v| v.is_finite())
            && -1.0 <= self.min
            && self.min <= self.default
            && self.default <= self.max
            && self.max <= 1.0
    }

    /// A value of the old normalised space in the space of this range (fontTools'
    /// `renormalizeValue`). A range of zero width on the side of `v` gives 0.
    pub fn renormalize(&self, v: f64, extrapolate: bool) -> f64 {
        self.renormalize_at(v, extrapolate, 0)
    }

    fn renormalize_at(&self, v: f64, extrapolate: bool, depth: u8) -> f64 {
        let (lower, default, upper) = (self.min, self.default, self.max);
        let v = if extrapolate { v } else { v.max(lower).min(upper) };
        if v == default {
            return 0.0;
        }
        if default < 0.0 {
            // Mirroring makes the default positive, so this recurses at most once.
            if depth > 0 {
                return 0.0;
            }
            return -self.reverse_negate().renormalize_at(-v, extrapolate, depth + 1);
        }
        let ratio = |a: f64, b: f64| if b == 0.0 { 0.0 } else { a / b };
        if v > default {
            return ratio(v - default, upper - default);
        }
        if lower >= 0.0 {
            return ratio(v - default, default - lower);
        }
        let total = self.distance_negative * -lower + self.distance_positive * default;
        let distance = if v >= 0.0 { (default - v) * self.distance_positive } else { -v * self.distance_negative + self.distance_positive * default };
        -ratio(distance, total)
    }
}

/// fontTools' `supportScalar` of one axis at `v`, with the solver's rules: a peak of 0 or a
/// location at the peak gives 1; outside the tent 0.
fn support(v: f64, t: Tent) -> f64 {
    let Tent { start: lower, peak, end: upper } = t;
    if peak == 0.0 || v == peak {
        return 1.0;
    }
    if v <= lower || upper <= v {
        return 0.0;
    }
    if v < peak { (v - lower) / (peak - lower) } else { (v - upper) / (peak - upper) }
}

/// The regions, each with the scalar its deltas are multiplied by, that replace `tent` once the
/// axis is limited to `range`, in the coordinates of the new range. A `None` region is the
/// default location (deltas folded into the default). Solutions with a scalar of 0 are left
/// out, so an empty list means the region vanishes. A tent with a peak of 0, out of order or
/// outside −2..=2, and a range out of order or outside −1..=1, give an empty list: the caller
/// drops such regions before solving.
pub fn rebase_tent(tent: Tent, range: AxisRange) -> Vec<(f64, Option<Tent>)> {
    let in_domain = [tent.start, tent.peak, tent.end].iter().all(|v| v.is_finite())
        && -2.0 <= tent.start
        && tent.start <= tent.peak
        && tent.peak <= tent.end
        && tent.end <= 2.0
        && tent.peak != 0.0;
    if !in_domain || !range.is_valid() {
        return Vec::new();
    }
    let n = |v: f64| range.renormalize(v, true);
    solve(tent, range, 0)
        .into_iter()
        .filter(|(scalar, _)| *scalar != 0.0)
        .map(|(scalar, t)| (scalar, t.map(|t| Tent { start: n(t.start), peak: n(t.peak), end: n(t.end) })))
        .collect()
}

/// `_solve`: the solutions in the old normalised space. Each call mirrors at most once and
/// clips the peak at most once, so `depth` stays below 4.
fn solve(tent: Tent, limit: AxisRange, depth: u8) -> Vec<(f64, Option<Tent>)> {
    if depth > 4 {
        return Vec::new();
    }
    let AxisRange { min: axis_min, default: axis_def, max: axis_max, .. } = limit;
    let Tent { start: lower, peak, end: upper } = tent;

    // Mirror the problem so that the default is at or below the peak.
    if axis_def > peak {
        return solve(tent.reverse_negate(), limit.reverse_negate(), depth + 1).into_iter().map(|(s, t)| (s, t.map(Tent::reverse_negate))).collect();
    }

    // Case 1: the whole region lies beyond the new maximum.
    if axis_max <= lower && axis_max < peak {
        return Vec::new();
    }

    // Case 2: the peak lies beyond the new maximum: clip the tent there and scale.
    if axis_max < peak {
        let mult = support(axis_max, tent);
        let clipped = Tent { start: lower, peak: axis_max, end: axis_max };
        return solve(clipped, limit, depth + 1).into_iter().map(|(s, t)| (s * mult, t)).collect();
    }

    // lower <= axis_def <= peak <= axis_max
    let gain = support(axis_def, tent);
    let mut out: Vec<(f64, Option<Tent>)> = vec![(gain, None)];

    // The positive side. `out_gain` is the tent's value at the new maximum.
    let out_gain = support(axis_max, tent);
    if gain >= out_gain {
        // Case 3a: the down-slope falls below the gain before the maximum; also taken when both
        // gains are 0.
        let crossing = peak + (1.0 - gain) * (upper - peak);
        out.push((1.0 - gain, Some(Tent { start: lower.max(axis_def), peak, end: crossing })));
        if upper >= axis_max {
            // Case 3a1: one tent after the crossing.
            out.push((out_gain - gain, Some(Tent { start: crossing, peak: axis_max, end: axis_max })));
        } else {
            // Case 3a2: two tents, to keep the value down to the maximum.
            // A tent's peak cannot fall on the default: nudge it.
            let upper = if upper == axis_def { upper + EPSILON } else { upper };
            out.push((-gain, Some(Tent { start: crossing, peak: upper, end: axis_max })));
            out.push((-gain, Some(Tent { start: upper, peak: axis_max, end: axis_max })));
        }
    } else {
        // Case 3 (keeping the tent and moving its end) is disabled in fontTools because
        // sanitisers reject the result, so case 4 always applies: two tents.
        out.push((1.0 - gain, Some(Tent { start: axis_def.max(lower), peak, end: axis_max })));
        // No tent of zero width.
        if peak < axis_max {
            out.push((out_gain - gain, Some(Tent { start: peak, peak: axis_max, end: axis_max })));
        }
    }

    // The negative side.
    if lower <= axis_min {
        // Case 1neg: the start lies beyond the new minimum: chop.
        let scalar = support(axis_min, tent);
        out.push((scalar - gain, Some(Tent { start: axis_min, peak: axis_min, end: axis_def })));
    } else {
        // Case 2neg: two tents to keep the value down to the minimum.
        // A tent's peak cannot fall on the default: nudge it.
        let lower = if lower == axis_def { lower - EPSILON } else { lower };
        out.push((-gain, Some(Tent { start: axis_min, peak: lower, end: axis_def })));
        out.push((-gain, Some(Tent { start: axis_min, peak: axis_min, end: lower })));
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
    use super::*;

    fn t(start: f64, peak: f64, end: f64) -> Tent {
        Tent { start, peak, end }
    }

    type Want = (f64, Option<(f64, f64, f64)>);

    fn close(got: &[(f64, Option<Tent>)], want: &[Want]) {
        assert_eq!(got.len(), want.len(), "{got:?}");
        for ((gs, gt), (ws, wt)) in got.iter().zip(want) {
            assert!((gs - ws).abs() < 1e-9, "{got:?}");
            match (gt, wt) {
                (None, None) => {}
                (Some(g), Some((a, b, c))) => {
                    assert!((g.start - a).abs() < 1e-9 && (g.peak - b).abs() < 1e-9 && (g.end - c).abs() < 1e-9, "{got:?}");
                }
                _ => panic!("{got:?}"),
            }
        }
    }

    /// Expected values from fontTools 4.66.1 `varLib.instancer.solver.rebaseTent`.
    #[test]
    fn rebased_tents_match_the_reference_solver() {
        // Pinning folds the region into the default.
        close(&rebase_tent(t(0.0, 1.0, 1.0), AxisRange::new(0.5, 0.5, 0.5)), &[(0.5, None)]);
        // A region beyond the new maximum vanishes.
        assert!(rebase_tent(t(0.0, 1.0, 1.0), AxisRange::new(-1.0, 0.0, 0.0)).is_empty());
        // Limiting the maximum to 0.5 clips the peak: the deltas scale by 0.5.
        close(&rebase_tent(t(0.0, 1.0, 1.0), AxisRange::new(0.0, 0.0, 0.5)), &[(0.5, Some((0.0, 1.0, 1.0)))]);
        // Limiting the minimum leaves a positive region alone.
        close(&rebase_tent(t(0.0, 1.0, 1.0), AxisRange::new(-0.5, 0.0, 1.0)), &[(1.0, Some((0.0, 1.0, 1.0)))]);
        // A new default under an intermediate region: the gain and the split tents.
        close(
            &rebase_tent(t(0.0, 0.5, 1.0), AxisRange::new(0.0, 0.25, 1.0)),
            &[(0.5, None), (0.5, Some((0.0, 1.0 / 3.0, 2.0 / 3.0))), (-0.5, Some((2.0 / 3.0, 1.0, 1.0))), (-0.5, Some((-1.0, -1.0, 0.0)))],
        );
        // The same with an L-shaped range on the negative side.
        close(&rebase_tent(t(-1.0, -1.0, 0.0), AxisRange::new(-0.5, 0.0, 1.0)), &[(0.5, Some((-1.0, -1.0, 0.0)))]);
    }

    #[test]
    fn renormalising_uses_the_user_space_distances() {
        // fontTools: NormalizedAxisTripleAndDistances(-1, 0.5, 1, 100, 400).renormalizeValue(-0.5)
        let r = AxisRange { min: -1.0, default: 0.5, max: 1.0, distance_negative: 100.0, distance_positive: 400.0 };
        let total = 100.0 + 400.0 * 0.5;
        assert!((r.renormalize(-0.5, true) - -(0.5 * 100.0 + 400.0 * 0.5) / total).abs() < 1e-12);
        assert!((r.renormalize(0.75, true) - 0.5).abs() < 1e-12);
        assert_eq!(r.renormalize(0.5, true), 0.0);
    }
}
