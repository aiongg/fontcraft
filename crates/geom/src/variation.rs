//! Interpolation between masters at a location in a normalized design space.
//!
//! Ported from fontTools `fontTools/varLib/models.py` (MIT; see `NOTICE`): `normalizeValue`,
//! `supportScalar` and `VariationModel` with its master ordering, supports, delta weights and
//! master scalars, so the numbers match the ones fontTools computes. Axes are numbered: a location
//! is one value per axis, 0 meaning the axis default, and the axis order of the reference's
//! `axisOrder` argument is the numbering.

use std::cmp::Ordering;
use std::fmt;

/// Most masters one model takes. Building a model costs the square of the master count.
pub const MAX_MASTERS: usize = 1024;
/// Most axes one model takes.
pub const MAX_AXES: usize = 256;

/// Why a model could not be built or used.
#[derive(Clone, Debug, PartialEq)]
pub enum VariationError {
    /// No master, more than [`MAX_MASTERS`] or more than [`MAX_AXES`] axes.
    Size,
    /// Locations of different lengths, or a value that is not finite.
    BadLocation,
    /// Two masters at the same location.
    DuplicateLocation,
    /// No master at the default location.
    NoDefault,
    /// Master values of different lengths, or a count that does not match the masters.
    BadValues,
}

impl fmt::Display for VariationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            VariationError::Size => "too many masters or axes, or no master",
            VariationError::BadLocation => "a master location is not a list of numbers of the same length as the others",
            VariationError::DuplicateLocation => "two masters are at the same location",
            VariationError::NoDefault => "no master is at the default location",
            VariationError::BadValues => "the masters do not have the same number of values",
        })
    }
}

impl std::error::Error for VariationError {}

/// `v` on the axis `lower`, `default`, `upper` as a normalized value: −1 at `lower`, 0 at
/// `default`, 1 at `upper`. Without `extrapolate`, `v` is first clamped to the range. `None`
/// when the three are not in order.
pub fn normalize_value(v: f64, lower: f64, default: f64, upper: f64, extrapolate: bool) -> Option<f64> {
    if !(lower <= default && default <= upper) {
        return None;
    }
    let v = if extrapolate { v } else { v.clamp(lower, upper) };
    if v == default || lower == upper {
        return Some(0.0);
    }
    if (v < default && lower != default) || (v > default && upper == default) {
        Some((v - default) / (default - lower))
    } else {
        Some((v - default) / (upper - default))
    }
}

/// One axis of a master's support: lower, peak and upper normalized value.
type Tent = (f64, f64, f64);
/// A support: the tents of the axes it varies on, by axis number.
type Support = Vec<(usize, Tent)>;

/// The scalar a delta with `support` gets at `location` (OpenType rules: an axis whose peak is 0
/// does not take part). With `ranges`, values beyond an axis's range extrapolate.
fn support_scalar(location: &[f64], support: &Support, ranges: Option<&[(f64, f64)]>) -> f64 {
    let mut scalar = 1.0;
    for &(axis, (lower, peak, upper)) in support {
        if peak == 0.0 || lower > peak || peak > upper || (lower < 0.0 && upper > 0.0) {
            continue;
        }
        let v = location.get(axis).copied().unwrap_or(0.0);
        if v == peak {
            continue;
        }
        if let Some(&(axis_min, axis_max)) = ranges.and_then(|r| r.get(axis)) {
            if v < axis_min && lower <= axis_min {
                if peak <= axis_min && peak < upper {
                    scalar *= (v - upper) / (peak - upper);
                    continue;
                } else if axis_min < peak {
                    scalar *= (v - lower) / (peak - lower);
                    continue;
                }
            } else if axis_max < v && axis_max <= upper {
                if axis_max <= peak && lower < peak {
                    scalar *= (v - lower) / (peak - lower);
                    continue;
                } else if peak < axis_max {
                    scalar *= (v - upper) / (peak - upper);
                    continue;
                }
            }
        }
        if v <= lower || upper <= v {
            return 0.0;
        }
        scalar *= if v < peak { (v - lower) / (peak - lower) } else { (v - upper) / (peak - upper) };
    }
    scalar
}

/// Masters at locations, ready to interpolate between.
#[derive(Clone, Debug)]
pub struct VariationModel {
    axis_count: usize,
    /// For each master in the caller's order, its position in the model's order.
    mapping: Vec<usize>,
    /// In the model's order.
    supports: Vec<Support>,
    /// In the model's order: earlier masters and the weight of their delta.
    delta_weights: Vec<Vec<(usize, f64)>>,
    /// Each axis's smallest and largest master value, when extrapolating.
    ranges: Option<Vec<(f64, f64)>>,
}

impl VariationModel {
    /// A model of masters at `locations` (normalized; one value per axis, all the same length,
    /// one of them all zeros). With `extrapolate`, locations beyond the masters extrapolate.
    pub fn new(locations: &[Vec<f64>], extrapolate: bool) -> Result<Self, VariationError> {
        let axis_count = locations.first().map_or(0, Vec::len);
        if locations.is_empty() || locations.len() > MAX_MASTERS || axis_count > MAX_AXES {
            return Err(VariationError::Size);
        }
        if locations.iter().any(|l| l.len() != axis_count || l.iter().any(|v| !v.is_finite())) {
            return Err(VariationError::BadLocation);
        }
        // `-0.0` and `0.0` are the same location.
        let locations: Vec<Vec<f64>> = locations.iter().map(|l| l.iter().map(|&v| if v == 0.0 { 0.0 } else { v }).collect()).collect();
        for (i, a) in locations.iter().enumerate() {
            if locations.iter().skip(i + 1).any(|b| b == a) {
                return Err(VariationError::DuplicateLocation);
            }
        }
        if !locations.iter().any(|l| l.iter().all(|&v| v == 0.0)) {
            return Err(VariationError::NoDefault);
        }
        let mut ranges: Vec<(f64, f64)> = vec![(0.0, 0.0); axis_count];
        for l in &locations {
            for (r, &v) in ranges.iter_mut().zip(l) {
                *r = (r.0.min(v), r.1.max(v));
            }
        }
        // Without extrapolation the regions reach to −1 and 1 on every axis a master uses.
        let region_ranges: Vec<(f64, f64)> = if extrapolate {
            ranges.clone()
        } else {
            ranges.iter().map(|&(lo, hi)| if lo == 0.0 && hi == 0.0 { (0.0, 0.0) } else { (-1.0, 1.0) }).collect()
        };

        // The model's order: by number of axes, then by how many of them hold a value that a
        // master on that axis alone has, then by axes, signs and distances.
        let mut on_axis: Vec<Vec<f64>> = vec![vec![0.0]; axis_count];
        for l in &locations {
            let mut used = l.iter().enumerate().filter(|(_, v)| **v != 0.0);
            if let (Some((axis, &v)), None) = (used.next(), used.next())
                && let Some(points) = on_axis.get_mut(axis)
            {
                points.push(v);
            }
        }
        let key = |l: &Vec<f64>| {
            let axes: Vec<usize> = (0..axis_count).filter(|&a| l.get(a).is_some_and(|v| *v != 0.0)).collect();
            let on_point = axes.iter().filter(|&&a| on_axis.get(a).is_some_and(|p| p.len() > 1 && l.get(a).is_some_and(|v| p.contains(v)))).count();
            let value = |a: &usize| l.get(*a).copied().unwrap_or(0.0);
            let signs: Vec<i8> = axes.iter().map(|a| if value(a) < 0.0 { -1 } else { 1 }).collect();
            let sizes: Vec<f64> = axes.iter().map(|a| value(a).abs()).collect();
            (axes.len(), on_point, axes, signs, sizes)
        };
        let mut order: Vec<usize> = (0..locations.len()).collect();
        let keys: Vec<_> = locations.iter().map(key).collect();
        order.sort_by(|&a, &b| {
            let (Some(ka), Some(kb)) = (keys.get(a), keys.get(b)) else { return Ordering::Equal };
            ka.0.cmp(&kb.0)
                .then(kb.1.cmp(&ka.1))
                .then_with(|| ka.2.cmp(&kb.2))
                .then_with(|| ka.3.cmp(&kb.3))
                .then_with(|| compare_floats(&ka.4, &kb.4))
        });
        let mut mapping = vec![0; locations.len()];
        for (pos, &i) in order.iter().enumerate() {
            if let Some(m) = mapping.get_mut(i) {
                *m = pos;
            }
        }
        let sorted: Vec<&Vec<f64>> = order.iter().filter_map(|&i| locations.get(i)).collect();

        // Supports: each master's region, cut back where an earlier master lies inside it.
        let mut supports: Vec<Support> = Vec::with_capacity(sorted.len());
        for l in &sorted {
            let mut region: Support = l
                .iter()
                .enumerate()
                .filter(|(_, v)| **v != 0.0)
                .map(|(axis, &v)| {
                    let (lo, hi) = region_ranges.get(axis).copied().unwrap_or((-1.0, 1.0));
                    (axis, if v > 0.0 { (0.0, v, hi) } else { (lo, v, 0.0) })
                })
                .collect();
            for prev in &supports {
                if prev.len() != region.len() || prev.iter().zip(&region).any(|(p, r)| p.0 != r.0) {
                    continue;
                }
                let relevant = region.iter().zip(prev).all(|(&(_, (lower, peak, upper)), &(_, (_, p, _)))| p == peak || (lower < p && p < upper));
                if !relevant {
                    continue;
                }
                let mut best: Vec<(usize, Tent)> = Vec::new();
                let mut best_ratio = -1.0;
                for (&(axis, (lower, at, upper)), &(_, (_, val, _))) in region.iter().zip(prev) {
                    let (tent, ratio) = match val.partial_cmp(&at) {
                        Some(Ordering::Less) => ((val, at, upper), (val - at) / (lower - at)),
                        Some(Ordering::Greater) => ((lower, at, val), (val - at) / (upper - at)),
                        _ => continue,
                    };
                    if ratio > best_ratio {
                        best.clear();
                        best_ratio = ratio;
                    }
                    if ratio == best_ratio {
                        best.push((axis, tent));
                    }
                }
                for (axis, tent) in best {
                    if let Some(slot) = region.iter_mut().find(|(a, _)| *a == axis) {
                        slot.1 = tent;
                    }
                }
            }
            supports.push(region);
        }
        let delta_weights = sorted
            .iter()
            .enumerate()
            .map(|(i, l)| {
                supports.iter().take(i).enumerate().filter_map(|(j, s)| Some((j, support_scalar(l, s, None))).filter(|(_, w)| *w != 0.0)).collect()
            })
            .collect();
        Ok(Self { axis_count, mapping, supports, delta_weights, ranges: extrapolate.then_some(ranges) })
    }

    /// The number of axes the model's locations have.
    pub fn axis_count(&self) -> usize {
        self.axis_count
    }

    /// The weight of each master (in the caller's order) at `location`: the value there is the
    /// sum of each master's value times its weight. Missing axes count as 0.
    pub fn master_scalars(&self, location: &[f64]) -> Vec<f64> {
        let mut out: Vec<f64> = self.supports.iter().map(|s| support_scalar(location, s, self.ranges.as_deref())).collect();
        for i in (0..out.len()).rev() {
            let Some(weights) = self.delta_weights.get(i) else { continue };
            let here = out.get(i).copied().unwrap_or(0.0);
            for &(j, w) in weights {
                if let Some(o) = out.get_mut(j) {
                    *o -= here * w;
                }
            }
        }
        self.mapping.iter().map(|&m| out.get(m).copied().unwrap_or(0.0)).collect()
    }

    /// Interpolates `masters` (one list of numbers per master, in the caller's order, all the
    /// same length) at `location`, adding the weighted masters in order as fontTools does.
    pub fn interpolate(&self, location: &[f64], masters: &[Vec<f64>]) -> Result<Vec<f64>, VariationError> {
        let len = masters.first().map_or(0, Vec::len);
        if masters.len() != self.mapping.len() || masters.iter().any(|m| m.len() != len) {
            return Err(VariationError::BadValues);
        }
        let mut out: Option<Vec<f64>> = None;
        for (values, scalar) in masters.iter().zip(self.master_scalars(location)) {
            if scalar == 0.0 {
                continue;
            }
            match &mut out {
                None => out = Some(values.iter().map(|v| v * scalar).collect()),
                Some(sum) => sum.iter_mut().zip(values).for_each(|(s, v)| *s += v * scalar),
            }
        }
        Ok(out.unwrap_or_else(|| vec![0.0; len]))
    }
}

/// Lexicographic order of two lists of finite numbers.
fn compare_floats(a: &[f64], b: &[f64]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        match x.total_cmp(y) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn normalize_value_matches_the_reference_examples() {
        assert_eq!(normalize_value(400.0, 100.0, 400.0, 900.0, false), Some(0.0));
        assert_eq!(normalize_value(100.0, 100.0, 400.0, 900.0, false), Some(-1.0));
        assert_eq!(normalize_value(650.0, 100.0, 400.0, 900.0, false), Some(0.5));
        assert_eq!(normalize_value(1400.0, 100.0, 400.0, 900.0, false), Some(1.0));
        assert_eq!(normalize_value(1400.0, 100.0, 400.0, 900.0, true), Some(2.0));
        assert_eq!(normalize_value(0.0, 10.0, 5.0, 20.0, true), None);
    }

    /// The reference's doctest model (weights scaled to −1…1), with the numbers fontTools 4
    /// prints for it.
    #[test]
    fn the_reference_model_gives_the_reference_weights() {
        // Axes: 0 = wght, 1 = wdth.
        let locs = vec![
            vec![100.0 / 180.0, 0.0],
            vec![-100.0 / 180.0, 0.0],
            vec![-1.0, 0.0],
            vec![0.0, 0.3],
            vec![120.0 / 180.0, 0.3],
            vec![120.0 / 180.0, 0.2],
            vec![0.0, 0.0],
            vec![1.0, 0.3],
            vec![1.0, 0.0],
        ];
        let model = VariationModel::new(&locs, false).unwrap();
        let mut sorted: Vec<(usize, usize)> = model.mapping.iter().enumerate().map(|(i, &m)| (m, i)).collect();
        sorted.sort();
        assert_eq!(sorted.iter().map(|x| x.1).collect::<Vec<_>>(), [6, 1, 2, 0, 8, 3, 7, 4, 5]);
        assert_eq!(model.delta_weights[6], [(0, 1.0), (4, 1.0), (5, 1.0)]);
        assert_eq!(
            model.delta_weights[8],
            [(0, 1.0), (3, 0.7500000000000001), (4, 0.24999999999999986), (5, 0.6666666666666667), (6, 0.4444444444444445), (7, 0.6666666666666667)]
        );
        let want = [
            0.20249999999999985,
            0.0,
            0.0,
            0.45833333333333337,
            0.22499999999999998,
            0.225,
            0.0016666666666667884,
            2.7755575615628914e-17,
            -0.11249999999999996,
        ];
        assert_eq!(model.master_scalars(&[0.3, 0.25]), want);
        // Interpolating a master's own location gives that master.
        let values: Vec<Vec<f64>> = (0..locs.len()).map(|i| vec![i as f64 * 10.0, (i * i) as f64]).collect();
        for (i, l) in locs.iter().enumerate() {
            let got = model.interpolate(l, &values).unwrap();
            assert!((got[0] - values[i][0]).abs() < 1e-9 && (got[1] - values[i][1]).abs() < 1e-9, "{i}: {got:?}");
        }
    }

    #[test]
    fn two_axes_at_the_corners_extrapolate() {
        // A default, one master at each axis's top and one at both: bilinear inside, linear
        // beyond.
        let locs = vec![vec![0.0, 0.0], vec![1.0, 0.0], vec![0.0, 1.0], vec![1.0, 1.0]];
        let values = vec![vec![0.0], vec![10.0], vec![100.0], vec![200.0]];
        let model = VariationModel::new(&locs, true).unwrap();
        let at = |x: f64, y: f64| model.interpolate(&[x, y], &values).unwrap()[0];
        assert!((at(0.5, 0.5) - 77.5).abs() < 1e-12);
        assert!((at(2.0, 0.0) - 20.0).abs() < 1e-12);
        assert!((at(-1.0, 0.0) + 10.0).abs() < 1e-12);
    }

    #[test]
    fn bad_models_are_errors() {
        assert_eq!(VariationModel::new(&[], false).unwrap_err(), VariationError::Size);
        assert_eq!(VariationModel::new(&[vec![1.0]], false).unwrap_err(), VariationError::NoDefault);
        assert_eq!(VariationModel::new(&[vec![0.0], vec![-0.0]], false).unwrap_err(), VariationError::DuplicateLocation);
        assert_eq!(VariationModel::new(&[vec![0.0], vec![f64::NAN]], false).unwrap_err(), VariationError::BadLocation);
        let m = VariationModel::new(&[vec![0.0], vec![1.0]], false).unwrap();
        assert_eq!(m.interpolate(&[0.5], &[vec![1.0], vec![1.0, 2.0]]).unwrap_err(), VariationError::BadValues);
    }
}
