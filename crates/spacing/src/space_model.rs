//! The shape-class model of auto-spacing: each side takes the spacing of the control side it
//! resembles most, corrected for the difference between the two shapes. Plan: `docs/plans/M13.md`
//! § "M13.20b" and decision D11. The caller (`spacing.autoSpace`) takes the profiles
//! ([`crate::profile`]) and knows glyphs, bands and masters; this module only sees shapes.
//!
//! # Method
//! A side's *shape* ([`SideShape`]) is its profile's depth inward from the side's extreme, row by
//! row, clipped; rows without ink count as the full clipping depth.
//!
//! - The *even white* of a shape is the white outside its extreme that would put it as far from a
//!   straight neighbour as two straight sides stand apart: the neighbour brings the straight white
//!   `S`, and the power mean of order −[`GAP_POWER`] of the row distances equals `2 S`. A side that
//!   is straight over the whole band has the even white `S`; a round or diagonal one has less, the
//!   less the fewer rows come close.
//! - A side takes the control side nearest to it by the mean depths of the bottom, middle and top
//!   thirds, as shares of the clipping depth (Euclidean distance; a control side of the same
//!   facing wins when both are about as near, [`FACING_BIAS`]). A control's own sides are their own
//!   control. Its white is that control's white plus its even white less the control's, so the
//!   controls keep their spacing exactly.
//! - Ink that reaches past the extreme (an ascender's arm, a descender's tail, ink above or below
//!   the band) is kept off the advance edge: the white at the glyph's bounds is never less than
//!   [`FLOOR_SHARE`] of `S`, nor less than the least of the controls has.
//!
//! The straight white `S` is the mean white of the two sides of the first control glyph: n or H
//! when the font has it, else the next.
//!
//! ```text
//! SideShape::new(profile: &SideProfile, depth: f64) -> Option<SideShape>
//! ClassModel::new(controls: Vec<Control>) -> Option<ClassModel>
//! ClassModel::straight(&self) -> f64
//! ClassModel::predict(&self, glyph: &str, side: Side, shape: &SideShape, overhang: f64) -> Option<Prediction>
//! ```

use crate::profile::{self, MIN_ROW_GAP, Side, SideProfile};

/// The order of the power mean of the row distances, negated. Fitted on one corpus font (Oswald);
/// orders 5 to 8 do about equally well.
pub const GAP_POWER: i32 = 6;
/// The least white at a glyph's bounds, as a share of the straight white.
pub const FLOOR_SHARE: f64 = 0.2;
/// What a control side of the opposite facing adds to its squared distance: a tie-break between
/// shapes that are about the same.
pub const FACING_BIAS: f64 = 0.001;
/// Bisection steps that solve for the even white, and the width in units they stop at.
const SOLVE_STEPS: usize = 64;
const SOLVE_WIDTH: f64 = 1e-4;

/// One side's shape over its band.
#[derive(Clone, Debug, PartialEq)]
pub struct SideShape {
    /// Per row, the depth inward from the extreme, clipped.
    rows: Vec<f64>,
    /// The mean depths of the bottom, middle and top thirds, as shares of the clipping depth.
    zones: [f64; 3],
}

impl SideShape {
    /// The shape of `profile`, clipped to `depth` inward from its extreme (a depth that is
    /// negative or not finite counts as 0); `None` without ink.
    pub fn new(profile: &SideProfile, depth: f64) -> Option<SideShape> {
        let depth = if depth.is_finite() { depth.max(0.0) } else { 0.0 };
        let extreme = profile.extreme()?;
        let clipped = profile::clip_depth(profile, depth);
        let share = |d: f64| if depth > 0.0 { d / depth } else { 0.0 };
        let zones = profile::zone_depths(&clipped)?.map(|d| share(d.unwrap_or(depth)));
        let rows = clipped.samples.iter().map(|x| x.map_or(depth, |x| (x - extreme).abs())).collect();
        Some(SideShape { rows, zones })
    }

    fn distance(&self, other: &SideShape) -> f64 {
        self.zones.iter().zip(&other.zones).map(|(a, b)| (a - b).powi(2)).sum()
    }

    /// The white outside the extreme that puts the side evenly beside a straight neighbour whose
    /// own white is `straight` (see the module comment).
    fn even_white(&self, straight: f64) -> f64 {
        let deepest = self.rows.iter().copied().fold(0.0, f64::max);
        let count = self.rows.len().max(1) as f64;
        let target = 2.0 * straight;
        // At `lo` every row is closer than the target, at `hi` none is.
        let (mut lo, mut hi) = (-straight - deepest, target + deepest);
        for _ in 0..SOLVE_STEPS {
            if hi - lo < SOLVE_WIDTH {
                break;
            }
            let mid = (lo + hi) / 2.0;
            let closeness = self.rows.iter().map(|d| (target / (mid + d + straight).max(MIN_ROW_GAP)).powi(GAP_POWER)).sum::<f64>() / count;
            if closeness < 1.0 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        (lo + hi) / 2.0
    }
}

/// A side whose spacing the model keeps and others take.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub glyph: String,
    pub side: Side,
    pub shape: SideShape,
    /// The white between the advance edge and the band's extreme.
    pub white: f64,
    /// How far the glyph's bounds reach past that extreme.
    pub overhang: f64,
}

/// What the model gives a side: its white outside the extreme, and the control it took it from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prediction<'a> {
    pub white: f64,
    pub control: &'a Control,
}

/// The controls of one group of glyphs, ready to space others.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassModel {
    /// Each control with its even white.
    controls: Vec<(Control, f64)>,
    /// The mean white of the first control glyph's sides.
    straight: f64,
    /// The least white at a glyph's bounds.
    floor: f64,
}

impl ClassModel {
    /// The model over `controls`, the first glyph's first (its white is the straight white);
    /// `None` without a control or with one that is not finite.
    pub fn new(controls: Vec<Control>) -> Option<ClassModel> {
        let first = controls.first()?.glyph.clone();
        let whites: Vec<f64> = controls.iter().filter(|c| c.glyph == first).map(|c| c.white).collect();
        let straight = whites.iter().sum::<f64>() / whites.len() as f64;
        let least = controls.iter().map(|c| c.white - c.overhang).fold(f64::INFINITY, f64::min);
        if ![straight, least].iter().all(|v| v.is_finite()) {
            return None;
        }
        // A straight side without white has no gap to even out; the corrections then count from a
        // unit of it, and the controls still keep their own.
        let even = straight.max(MIN_ROW_GAP);
        let controls = controls
            .into_iter()
            .map(|c| {
                let w = c.shape.even_white(even);
                (c, w)
            })
            .collect();
        Some(ClassModel { controls, straight, floor: least.min(FLOOR_SHARE * straight) })
    }

    /// The white beside a straight side.
    pub fn straight(&self) -> f64 {
        self.straight
    }

    /// The white of `side` of `glyph`, whose bounds reach `overhang` past the band's extreme.
    pub fn predict(&self, glyph: &str, side: Side, shape: &SideShape, overhang: f64) -> Option<Prediction<'_>> {
        let own = self.controls.iter().find(|(c, _)| c.glyph == glyph && c.side == side);
        let (control, control_even) = match own {
            Some(found) => found,
            None => {
                let rank = |c: &Control| c.shape.distance(shape) + if c.side == side { 0.0 } else { FACING_BIAS };
                // The first of equals, as `min_by` keeps it.
                self.controls.iter().min_by(|(a, _), (b, _)| rank(a).total_cmp(&rank(b)))?
            }
        };
        let white = control.white + shape.even_white(self.straight.max(MIN_ROW_GAP)) - control_even;
        let white = white.max(overhang + self.floor);
        white.is_finite().then_some(Prediction { white, control })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Band, profiles};
    use fontcraft_geom::{BezPath, Shape as _};

    const BAND: Band = Band { bottom: 0.0, top: 500.0 };
    const DEPTH: f64 = 75.0;

    fn shapes(path: &BezPath) -> (SideShape, SideShape) {
        let (l, r) = profiles(path, BAND, 5.0).unwrap();
        (SideShape::new(&l, DEPTH).unwrap(), SideShape::new(&r, DEPTH).unwrap())
    }

    fn stem() -> BezPath {
        fontcraft_geom::Rect::new(0.0, 0.0, 80.0, 500.0).to_path(0.1)
    }

    fn round(rx: f64) -> BezPath {
        let (k, r) = (0.552_284_75, 250.0);
        let mut p = BezPath::new();
        p.move_to((r + rx, r));
        p.curve_to((r + rx, r + r * k), (r + rx * k, 2.0 * r), (r, 2.0 * r));
        p.curve_to((r - rx * k, 2.0 * r), (r - rx, r + r * k), (r - rx, r));
        p.curve_to((r - rx, r - r * k), (r - rx * k, 0.0), (r, 0.0));
        p.curve_to((r + rx * k, 0.0), (r + rx, r - r * k), (r + rx, r));
        p.close_path();
        p
    }

    fn model() -> ClassModel {
        let controls = [("n", stem(), 60.0), ("o", round(200.0), 40.0)]
            .into_iter()
            .flat_map(|(name, path, white)| {
                let (l, r) = shapes(&path);
                [(Side::Left, l), (Side::Right, r)].map(|(side, shape)| Control { glyph: name.into(), side, shape, white, overhang: 0.0 })
            })
            .collect();
        ClassModel::new(controls).unwrap()
    }

    #[test]
    fn sides_take_the_spacing_of_the_control_they_resemble() {
        let m = model();
        assert_eq!(m.straight(), 60.0);
        // The controls keep their white exactly, whatever the solved corrections.
        let (l, _) = shapes(&round(200.0));
        assert_eq!(m.predict("o", Side::Left, &l, 0.0).unwrap().white, 40.0);
        // Another straight side is the n's; a round one that curves in slower wants a little more
        // room than the o.
        let (s, _) = shapes(&stem());
        let p = m.predict("l", Side::Right, &s, 0.0).unwrap();
        assert_eq!((p.control.glyph.as_str(), p.control.side, p.white), ("n", Side::Right, 60.0));
        let (c, _) = shapes(&round(150.0));
        let p = m.predict("c", Side::Left, &c, 0.0).unwrap();
        assert_eq!((p.control.glyph.as_str(), p.control.side), ("o", Side::Left));
        assert!(p.white > 40.0 && p.white < 50.0, "{}", p.white);
        // Ink past the extreme stays inside the advance: a fifth of the straight white at least.
        let p = m.predict("f", Side::Right, &s, 100.0).unwrap();
        assert!((p.white - 112.0).abs() < 1e-9, "{}", p.white);
    }

    #[test]
    fn hostile_shapes_and_controls_do_not_panic() {
        let empty = profiles(&BezPath::new(), BAND, 5.0).unwrap().0;
        assert_eq!(SideShape::new(&empty, DEPTH), None);
        assert_eq!(ClassModel::new(Vec::new()), None);
        let (l, _) = profiles(&round(200.0), BAND, 5.0).unwrap();
        for depth in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            let shape = SideShape::new(&l, depth).unwrap();
            assert!(shape.even_white(60.0).is_finite());
        }
        let shape = SideShape::new(&l, DEPTH).unwrap();
        let control = |white: f64| Control { glyph: "n".into(), side: Side::Left, shape: shape.clone(), white, overhang: 0.0 };
        assert_eq!(ClassModel::new(vec![control(f64::NAN)]), None);
        // A font spaced at or below zero still keeps its controls.
        for white in [0.0, -40.0] {
            let m = ClassModel::new(vec![control(white)]).unwrap();
            assert_eq!(m.predict("n", Side::Left, &shape, 0.0).unwrap().white, white);
        }
    }
}
