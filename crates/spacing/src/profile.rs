//! Side profiles of an outline and the measures auto-spacing and auto-kerning take from them.
//! Plan: `docs/plans/M13.md` § 3 (auto-spacing and auto-kerning, decision D11). Users:
//! `spacing.autoSpace` (M13.20), auto-kerning and Kern to Distance (M13.21), the pair tools and
//! Space Center (M13.22, M13.23).
//!
//! A profile samples, every `step` units over a horizontal band, the outline's horizontal
//! extreme on one side: the leftmost ink for the left side, the rightmost for the right. Rows of
//! the band with no ink have no sample. [`clip_depth`] limits each sample to `depth` inward from
//! the side's extreme, so deep counters (the inside of `c`, the gap of `T`) count only so far.
//! [`white_area`] is the area between a clipped profile and a sidebearing line; [`mean_white`]
//! divides it by the inked height, which gives the mean width of white per row. [`mean_depth`]
//! is the part of that mean that lies inside the profile, between the samples and the side's
//! extreme; the area model of auto-spacing weighs it against the white outside the extreme.
//! [`zone_depths`] takes that mean in each third of the band. [`optical_gap`] is the
//! distance between the right profile of one glyph and the left profile of the next, averaged
//! over the rows so that the closest rows count most.
//!
//! Profiles are in the coordinates of the outline they were taken from; the caller places glyphs
//! by passing offsets. For an italic master, pass the outline slanted back
//! ([`crate::measure::measured_outline`]) so profiles agree with the measured sidebearings.
//!
//! The rows lie at the centres of equal slices of the band, so a band from 0 to 500 with step 5
//! samples heights 2.5, 7.5 … 497.5: no row falls exactly on a baseline or a flat top, where a
//! horizontal segment meets the sampling line along its whole length. Two profiles taken over
//! the same band with the same step have the same rows, which [`optical_gap`] relies on.
//!
//! ```text
//! profiles(outline: &BezPath, band: Band, step: f64) -> Result<(SideProfile, SideProfile), ProfileError>
//! clip_depth(profile: &SideProfile, depth: f64) -> SideProfile
//! white_area(profile: &SideProfile, x: f64) -> f64
//! mean_white(profile: &SideProfile, x: f64) -> Option<f64>
//! mean_depth(profile: &SideProfile) -> Option<f64>
//! zone_depths(profile: &SideProfile) -> Option<[Option<f64>; 3]>
//! optical_gap(right: &SideProfile, left: &SideProfile, offset: f64) -> Option<f64>
//! SideProfile::extreme(&self) -> Option<f64>;  SideProfile::inked_rows(&self) -> usize
//! SideProfile::height(&self, i: usize) -> f64;  SideProfile::mean_x(&self) -> Option<f64>
//! ```

use fontcraft_geom::{BezPath, Line, ParamCurve, ParamCurveExtrema, Point};

/// Smallest sampling step, in font units.
pub const MIN_PROFILE_STEP: f64 = 1.0;
/// Most samples in one profile; a band too tall for its step is refused.
pub const MAX_PROFILE_SAMPLES: usize = 4_096;
/// The order of the power mean [`optical_gap`] takes of the row distances, negated: 2 makes a
/// row at half the distance count four times as much.
pub const GAP_POWER: f64 = 2.0;
/// The smallest row distance [`optical_gap`] uses; rows closer than this, or overlapping, count
/// as this close.
pub const MIN_ROW_GAP: f64 = 1.0;

/// Which side of the glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// The vertical band sampled: from `bottom` to `top`, in font units (for example the baseline
/// minus overshoot to the x-height plus overshoot).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub bottom: f64,
    pub top: f64,
}

/// One side's profile.
#[derive(Clone, Debug, PartialEq)]
pub struct SideProfile {
    pub side: Side,
    /// The height of the first sample.
    pub bottom: f64,
    /// The distance between samples.
    pub step: f64,
    /// The extreme x at `bottom + i × step`; `None` where that row has no ink.
    pub samples: Vec<Option<f64>>,
}

impl SideProfile {
    /// The side's extreme over all samples (smallest x for the left side, largest for the right);
    /// `None` without ink.
    pub fn extreme(&self) -> Option<f64> {
        let xs = self.samples.iter().flatten().copied();
        match self.side {
            Side::Left => xs.reduce(f64::min),
            Side::Right => xs.reduce(f64::max),
        }
    }

    /// How many rows have ink.
    pub fn inked_rows(&self) -> usize {
        self.samples.iter().flatten().count()
    }

    /// The height of row `i`.
    pub fn height(&self, i: usize) -> f64 {
        self.bottom + i as f64 * self.step
    }

    /// The mean x of the rows with ink; `None` without ink.
    pub fn mean_x(&self) -> Option<f64> {
        let n = self.inked_rows();
        (n > 0).then(|| self.samples.iter().flatten().sum::<f64>() / n as f64)
    }
}

/// Why profiles cannot be taken.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ProfileError {
    #[error("the sampling step must be a number of at least {MIN_PROFILE_STEP} unit")]
    Step,
    #[error("the band must run upward between finite heights")]
    Band,
    #[error("the band needs more than {MAX_PROFILE_SAMPLES} samples at this step")]
    TooManySamples,
    #[error("not supported yet: {0}")]
    Unsupported(String),
}

/// The heights the rows of `band` are sampled at, every `step` units: the first height and the
/// row count.
fn rows(band: Band, step: f64) -> Result<(f64, usize), ProfileError> {
    if !(step.is_finite() && step >= MIN_PROFILE_STEP) {
        return Err(ProfileError::Step);
    }
    if !(band.bottom.is_finite() && band.top.is_finite() && band.bottom <= band.top) {
        return Err(ProfileError::Band);
    }
    let span = band.top - band.bottom;
    let slices = (span / step).floor();
    if !slices.is_finite() || slices >= MAX_PROFILE_SAMPLES as f64 {
        return Err(ProfileError::TooManySamples);
    }
    // At least one row; the rows sit centred in the band.
    let count = (slices as usize).max(1);
    let first = band.bottom + (span - (count - 1) as f64 * step) / 2.0;
    Ok((first, count))
}

/// The left and right profiles of `outline` (a glyph's resolved outline, components decomposed)
/// over `band`, sampled every `step` units.
pub fn profiles(outline: &BezPath, band: Band, step: f64) -> Result<(SideProfile, SideProfile), ProfileError> {
    let (first, count) = rows(band, step)?;
    // Each segment with its vertical extent, so a row tests only the segments that reach it.
    let segs: Vec<_> = outline
        .segments()
        .filter_map(|seg| {
            let b = seg.bounding_box();
            [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()).then_some((seg, b))
        })
        .collect();
    let mut left = Vec::with_capacity(count);
    let mut right = Vec::with_capacity(count);
    for i in 0..count {
        let y = first + i as f64 * step;
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for (seg, b) in &segs {
            if y < b.y0 || y > b.y1 {
                continue;
            }
            let line = Line::new(Point::new(b.x0 - 1.0, y), Point::new(b.x1 + 1.0, y));
            for hit in seg.intersect_line(line) {
                let x = seg.eval(hit.segment_t).x;
                if x.is_finite() {
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
        }
        if lo <= hi {
            left.push(Some(lo));
            right.push(Some(hi));
        } else {
            left.push(None);
            right.push(None);
        }
    }
    Ok((SideProfile { side: Side::Left, bottom: first, step, samples: left }, SideProfile { side: Side::Right, bottom: first, step, samples: right }))
}

/// `profile` with every sample limited to `depth` units inward from the side's extreme. A
/// negative depth counts as zero; a depth that is not finite leaves the profile as it is.
pub fn clip_depth(profile: &SideProfile, depth: f64) -> SideProfile {
    let (Some(extreme), true) = (profile.extreme(), depth.is_finite()) else {
        return profile.clone();
    };
    let depth = depth.max(0.0);
    let clip = |x: f64| match profile.side {
        Side::Left => x.min(extreme + depth),
        Side::Right => x.max(extreme - depth),
    };
    SideProfile { samples: profile.samples.iter().map(|s| s.map(clip)).collect(), ..profile.clone() }
}

/// The white area between `profile` and the vertical line at `x` (a sidebearing line), over the
/// rows that have ink, in square font units. Rows where the ink crosses the line count
/// negatively.
pub fn white_area(profile: &SideProfile, x: f64) -> f64 {
    let sum: f64 = profile
        .samples
        .iter()
        .flatten()
        .map(|s| match profile.side {
            Side::Left => s - x,
            Side::Right => x - s,
        })
        .sum();
    sum * profile.step
}

/// The mean width of white per inked row between `profile` and the line at `x`: the white area
/// divided by the inked height. `None` without ink.
pub fn mean_white(profile: &SideProfile, x: f64) -> Option<f64> {
    let n = profile.inked_rows();
    (n > 0).then(|| white_area(profile, x) / (n as f64 * profile.step))
}

/// The mean depth of `profile`: the mean over the rows with ink of each sample's distance
/// inward from the side's extreme. `None` without ink.
pub fn mean_depth(profile: &SideProfile) -> Option<f64> {
    let extreme = profile.extreme()?;
    let n = profile.inked_rows();
    let sum: f64 = profile
        .samples
        .iter()
        .flatten()
        .map(|s| match profile.side {
            Side::Left => s - extreme,
            Side::Right => extreme - s,
        })
        .sum();
    (n > 0).then(|| sum / n as f64)
}

/// The mean depth of `profile` in the bottom, middle and top third of its rows (a row belongs to the
/// third its centre lies in, so a symmetric profile has symmetric thirds): the mean over the third's
/// rows with ink of each sample's distance inward from the side's extreme. A third without
/// ink is `None`; so is the whole without ink anywhere.
pub fn zone_depths(profile: &SideProfile) -> Option<[Option<f64>; 3]> {
    let extreme = profile.extreme()?;
    let count = profile.samples.len().max(1);
    let mut sums = [(0.0, 0usize); 3];
    for (i, x) in profile.samples.iter().enumerate() {
        let (Some(x), Some(zone)) = (x, sums.get_mut((2 * i + 1) * 3 / (2 * count))) else { continue };
        zone.0 += (x - extreme).abs();
        zone.1 += 1;
    }
    Some(sums.map(|(sum, n)| (n > 0).then(|| sum / n as f64)))
}

/// The optical distance between `right` (the right profile of the first glyph) and `left` (the
/// left profile of the second glyph, shifted right by `offset`: the first glyph's advance plus
/// any kerning), over the rows where both have ink: the power mean of order −[`GAP_POWER`] of
/// the row distances, each at least [`MIN_ROW_GAP`]. Equal distances give that distance; a few
/// close rows pull the result toward them more than a plain mean would, as the eye judges a pair
/// by where it comes closest. `None` when no row has both. Both profiles must share bottom and
/// step.
pub fn optical_gap(right: &SideProfile, left: &SideProfile, offset: f64) -> Option<f64> {
    if !offset.is_finite() {
        return None;
    }
    let same = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0);
    if !same(right.bottom, left.bottom) || !same(right.step, left.step) {
        return None;
    }
    let rows: Vec<f64> =
        right.samples.iter().zip(&left.samples).filter_map(|(r, l)| Some((l.as_ref()? + offset - r.as_ref()?).max(MIN_ROW_GAP))).collect();
    // Relative to the closest row, so equal distances give that distance exactly.
    let closest = rows.iter().copied().reduce(f64::min)?;
    let mean = rows.iter().map(|d| (d / closest).powf(-GAP_POWER)).sum::<f64>() / rows.len() as f64;
    Some(closest * mean.powf(-1.0 / GAP_POWER)).filter(|g| g.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_geom::Shape as _;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        fontcraft_geom::Rect::new(x0, y0, x1, y1).to_path(0.1)
    }

    fn rect_left() -> SideProfile {
        profiles(&rect(50.0, 0.0, 450.0, 500.0), Band { bottom: 0.0, top: 500.0 }, 5.0).unwrap().0
    }

    /// An ellipse of four cubic curves.
    fn oval(cx: f64, cy: f64, rx: f64, ry: f64) -> BezPath {
        let k = 0.552_284_75;
        let mut p = BezPath::new();
        p.move_to((cx + rx, cy));
        p.curve_to((cx + rx, cy + ry * k), (cx + rx * k, cy + ry), (cx, cy + ry));
        p.curve_to((cx - rx * k, cy + ry), (cx - rx, cy + ry * k), (cx - rx, cy));
        p.curve_to((cx - rx, cy - ry * k), (cx - rx * k, cy - ry), (cx, cy - ry));
        p.curve_to((cx + rx * k, cy - ry), (cx + rx, cy - ry * k), (cx + rx, cy));
        p.close_path();
        p
    }

    #[test]
    fn a_rectangle_has_straight_profiles_and_no_ink_outside_it() {
        let (l, r) = profiles(&rect(50.0, 0.0, 450.0, 500.0), Band { bottom: -20.0, top: 520.0 }, 10.0).unwrap();
        assert_eq!(l.samples.len(), 54);
        assert_eq!((l.bottom, l.step), (-15.0, 10.0));
        assert_eq!((l.bottom, l.step), (r.bottom, r.step));
        // Rows at -15 and -5 and at 505 and 515 miss the rectangle.
        assert_eq!(&l.samples[..2], &[None, None]);
        assert_eq!(&l.samples[52..], &[None, None]);
        assert!(l.samples[2..52].iter().all(|s| *s == Some(50.0)), "{l:?}");
        assert!(r.samples[2..52].iter().all(|s| *s == Some(450.0)), "{r:?}");
        assert_eq!((l.extreme(), r.extreme()), (Some(50.0), Some(450.0)));
        assert_eq!(l.inked_rows(), 50);
        // 50 rows of 10 units, 50 units of white each to the line at 0.
        assert_eq!(white_area(&l, 0.0), 25_000.0);
        assert_eq!(mean_white(&l, 0.0), Some(50.0));
        assert_eq!(mean_white(&r, 500.0), Some(50.0));
        // Clipping changes nothing on a straight side.
        assert_eq!(clip_depth(&l, 15.0), l);
    }

    #[test]
    fn an_o_has_round_profiles_and_clipping_limits_its_white() {
        let (l, r) = profiles(&oval(250.0, 250.0, 200.0, 260.0), Band { bottom: -10.0, top: 510.0 }, 5.0).unwrap();
        let (el, er) = (l.extreme().unwrap(), r.extreme().unwrap());
        // The sampled extremes lie within a fraction of a unit of the true ones (50 and 450).
        assert!((el - 50.0).abs() < 0.1 && (er - 450.0).abs() < 0.1, "{el} {er}");
        // Symmetric: the right profile mirrors the left.
        for (a, b) in l.samples.iter().zip(&r.samples) {
            let (a, b) = (a.unwrap(), b.unwrap());
            assert!((a + b - 500.0).abs() < 1e-6, "{a} {b}");
        }
        // Rows near the top and bottom curve in further than near the middle.
        assert!(l.samples[0].unwrap() > l.samples[l.samples.len() / 2].unwrap() + 100.0);
        let clipped = clip_depth(&l, 30.0);
        assert!(clipped.samples.iter().flatten().all(|x| *x <= el + 30.0 + 1e-9));
        assert!(white_area(&clipped, 0.0) < white_area(&l, 0.0));
        // The mean depth is the white between the clipped samples and the extreme.
        assert!((mean_depth(&clipped).unwrap() - mean_white(&clipped, el).unwrap()).abs() < 1e-9);
        assert_eq!(mean_depth(&rect_left()), Some(0.0));
        // Deep at the top and bottom, shallow in the middle; a third without ink has no mean.
        let [bottom, middle, top] = zone_depths(&clipped).unwrap().map(Option::unwrap);
        assert!(bottom > 5.0 * middle && (top - bottom).abs() < 1e-6, "{bottom} {middle} {top}");
        let (half, _) = profiles(&rect(0.0, 0.0, 10.0, 300.0), Band { bottom: 0.0, top: 500.0 }, 5.0).unwrap();
        assert_eq!(zone_depths(&half), Some([Some(0.0), Some(0.0), None]));
        let mean = mean_white(&clipped, 0.0).unwrap();
        assert!(mean > 50.0 && mean < 80.0, "{mean}");
        // The other side clips toward the right extreme.
        let cr = clip_depth(&r, 30.0);
        assert!(cr.samples.iter().flatten().all(|x| *x >= er - 30.0 - 1e-9));
        assert!((mean_white(&cr, 500.0).unwrap() - mean).abs() < 1e-6);
    }

    #[test]
    fn the_optical_gap_leans_toward_the_closest_rows_where_both_have_ink() {
        let band = Band { bottom: 0.0, top: 500.0 };
        let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-9);
        let (_, r) = profiles(&rect(50.0, 0.0, 450.0, 500.0), band, 5.0).unwrap();
        let (l, _) = profiles(&rect(30.0, 0.0, 400.0, 250.0), band, 5.0).unwrap();
        // The second glyph starts at 500 + 30; only its lower half has ink.
        assert!(near(optical_gap(&r, &l, 500.0), 80.0));
        assert!(near(optical_gap(&r, &l, 460.0), 40.0));
        // Overlapping rows count as one unit apart.
        assert!(near(optical_gap(&r, &l, 400.0), MIN_ROW_GAP));
        // Half the rows 80 apart, half 180: the plain mean would be 130.
        let mut step = rect(30.0, 0.0, 400.0, 250.0);
        step.extend(rect(130.0, 250.0, 400.0, 500.0).elements().iter().copied());
        let (l2, _) = profiles(&step, band, 5.0).unwrap();
        let expected = ((80f64.powi(-2) + 180f64.powi(-2)) / 2.0).powf(-0.5);
        assert!(near(optical_gap(&r, &l2, 500.0), expected), "{:?} {expected}", optical_gap(&r, &l2, 500.0));
        assert!(expected > 100.0 && expected < 105.0);
        let (other, _) = profiles(&rect(30.0, 0.0, 400.0, 250.0), band, 10.0).unwrap();
        assert_eq!(optical_gap(&r, &other, 500.0), None, "different steps");
        let (empty, _) = profiles(&BezPath::new(), band, 5.0).unwrap();
        assert_eq!(optical_gap(&r, &empty, 500.0), None);
        assert_eq!(empty.extreme(), None);
        assert_eq!(mean_white(&empty, 0.0), None);
    }

    #[test]
    fn hostile_bands_and_steps_are_refused() {
        let p = rect(0.0, 0.0, 10.0, 10.0);
        assert_eq!(profiles(&p, Band { bottom: 0.0, top: 10.0 }, 0.5), Err(ProfileError::Step));
        assert_eq!(profiles(&p, Band { bottom: 0.0, top: 10.0 }, f64::NAN), Err(ProfileError::Step));
        assert_eq!(profiles(&p, Band { bottom: 10.0, top: 0.0 }, 1.0), Err(ProfileError::Band));
        assert_eq!(profiles(&p, Band { bottom: 0.0, top: f64::INFINITY }, 1.0), Err(ProfileError::Band));
        assert_eq!(profiles(&p, Band { bottom: -1e6, top: 1e6 }, 1.0), Err(ProfileError::TooManySamples));
        // A band of zero height has one row.
        let (l, _) = profiles(&p, Band { bottom: 5.0, top: 5.0 }, 1.0).unwrap();
        assert_eq!(l.samples, vec![Some(0.0)]);
    }
}
