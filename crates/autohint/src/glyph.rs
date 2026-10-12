//! The glyph as the autohinter sees it: points, stems, path elements and the glyph's subpaths.
//!
//! Ported from otfautohint's `glyphData.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`), including the parts of fontTools' `BasePen` (MIT) the original relies on.
//!
//! Differences from the original, by design:
//! - `pt.a` / `pt.o` take the dimension as an argument instead of reading a thread-local.
//! - A path element is named by [`PeRef`] instead of by object identity. `glyphData` used itself
//!   as the "before the first element" cursor; here [`GlyphData::first`] and
//!   [`GlyphData::first_for_hints`] start a walk and [`GlyphData::next`] continues it. The walks
//!   follow the indexes of the [`PeRef`], so they need no `syncPositions` first.
//! - The pen methods (`moveTo`, `lineTo`, `curveToOne`, `closePath`) become
//!   [`GlyphData::from_contours`], which draws every contour with the same rules.
//! - Existing hints (the `hStem`/`vStem`/`hintmask` quasi-pen methods) are read by
//!   [`GlyphData::set_hints`]; FontCraft passes hints that way when a glyph keeps manual ones.
//! - A mask list `[h, v]` whose half is `None` is a [`HintMask`] whose vector is empty.
//! - Two quirks of the original are kept because its results depend on them:
//!   [`PathElement::flex_unset`] (`flex` is `False` rather than `None`) and the shared bounds
//!   lists of [`GlyphData::get_bounds`].

use std::collections::{BTreeMap, HashMap};
use std::ops::{Add, Mul, Sub};

use fontcraft_geom::{BezPath, PathEl, Point};

use crate::bezier::{BoundsState, PathBoundsState, approximate_cubic_arc_length};
use crate::{AutohintError, GlyphHints, HintMask, MAX_ELEMENTS};

/// The largest coordinate accepted, as the model's limit.
pub const MAX_COORDINATE: f64 = 1_000_000.0;

/// The two hinting dimensions. `H` is otfautohint's `hhinter` (horizontal stems: `hstem`, y
/// edges) and `V` its `vhinter` (vertical stems: `vstem`, x edges).
///
/// The aligned coordinate [`Pt::a`] is x for `H` and y for `V`; the opposite coordinate
/// [`Pt::o`] is the other one. This is the original's convention (`pt.setAlign`), kept so the
/// ported code reads the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dim {
    H,
    V,
}

impl Dim {
    pub fn is_v(self) -> bool {
        self == Dim::V
    }
}

/// otfautohint's float tolerance (`feq`, `fne`).
pub const FEQ_FACTOR: f64 = 1.52e-5;

/// `a` and `b` are equal within `factor` (`feq`); [`FEQ_FACTOR`] is the original's default.
pub fn feq(a: f64, b: f64) -> bool {
    (a - b).abs() < FEQ_FACTOR
}

/// `a` and `b` differ by at least [`FEQ_FACTOR`] (`fne`).
pub fn fne(a: f64, b: f64) -> bool {
    (a - b).abs() >= FEQ_FACTOR
}

/// `feq` with an explicit tolerance.
pub fn feq_by(a: f64, b: f64, factor: f64) -> bool {
    (a - b).abs() < factor
}

/// Python's `round(v, digits)`: the exact binary value rounded half to even at `digits`
/// decimals, then read back as the nearest float. Exact for every value whose rounded form has at
/// most 15 significant digits, which covers every coordinate the model accepts.
pub fn round_digits(v: f64, digits: u32) -> f64 {
    if !v.is_finite() || v == 0.0 {
        return v;
    }
    let bits = v.to_bits();
    let negative = bits >> 63 == 1;
    let exp_bits = ((bits >> 52) & 0x7ff) as i64;
    let frac = bits & ((1u64 << 52) - 1);
    let (mant, exp) = if exp_bits == 0 { (frac, -1074) } else { (frac | (1u64 << 52), exp_bits - 1075) };
    if exp >= 0 {
        // Already an integer.
        return v;
    }
    let Some(scale) = 10u128.checked_pow(digits) else { return v };
    let Some(prod) = u128::from(mant).checked_mul(scale) else { return v };
    let shift = exp.unsigned_abs();
    let q = if shift >= 128 {
        0
    } else {
        let shift = shift as u32;
        let q = prod >> shift;
        let rem = prod & ((1u128 << shift) - 1);
        let half = 1u128 << (shift - 1);
        if rem > half || (rem == half && q & 1 == 1) { q + 1 } else { q }
    };
    let r = q as f64 / scale as f64;
    if negative { -r } else { r }
}

/// A value rounded to 4 decimals, as `norm_float` does before printing.
pub fn norm_float(v: f64) -> f64 {
    round_digits(v, 4)
}

/// A point (`pt`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// The point, rounded half to even when `round` is set (`pt(…, roundCoords)`).
    pub fn rounded_if(x: f64, y: f64, round: bool) -> Self {
        if round { Self::new(x.round_ties_even(), y.round_ties_even()) } else { Self::new(x, y) }
    }

    /// The aligned coordinate: x for [`Dim::H`], y for [`Dim::V`].
    pub fn a(self, dim: Dim) -> f64 {
        match dim {
            Dim::H => self.x,
            Dim::V => self.y,
        }
    }

    /// The opposite coordinate: y for [`Dim::H`], x for [`Dim::V`].
    pub fn o(self, dim: Dim) -> f64 {
        match dim {
            Dim::H => self.y,
            Dim::V => self.x,
        }
    }

    /// The point with its aligned and opposite coordinates given (`pt.ao`).
    pub fn from_ao(dim: Dim, a: f64, o: f64) -> Self {
        match dim {
            Dim::H => Self::new(a, o),
            Dim::V => Self::new(o, a),
        }
    }

    pub fn avg(self, other: Pt) -> Pt {
        Pt::new((self.x + other.x) * 0.5, (self.y + other.y) * 0.5)
    }

    pub fn dot(self, other: Pt) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// The scalar cross product `other.y·self.x − other.x·self.y` (`scross`).
    pub fn scross(self, other: Pt) -> f64 {
        other.y * self.x - other.x * self.y
    }

    pub fn distsq(self, other: Pt) -> f64 {
        let (dx, dy) = (self.x - other.x, self.y - other.y);
        dx * dx + dy * dy
    }

    pub fn a_dist(self, other: Pt, dim: Dim) -> f64 {
        (self - other).a(dim).abs()
    }

    pub fn o_dist(self, other: Pt, dim: Dim) -> f64 {
        (self - other).o(dim).abs()
    }

    pub fn normsq(self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    pub fn abs(self) -> Pt {
        Pt::new(self.x.abs(), self.y.abs())
    }

    /// Rounded half to even, as Python's `round` (`pt.round(0)`).
    pub fn round(self) -> Pt {
        Pt::new(self.x.round_ties_even(), self.y.round_ties_even())
    }

    /// Rounded to `digits` decimals as Python's `round` (`pt.round(dec)`).
    pub fn round_to(self, digits: u32) -> Pt {
        Pt::new(round_digits(self.x, digits), round_digits(self.y, digits))
    }

    /// Equal within [`FEQ_FACTOR`] on both coordinates (`pt.__eq__`).
    pub fn near(self, other: Pt) -> bool {
        feq(self.x, other.x) && feq(self.y, other.y)
    }

    /// Equal within `factor` on both coordinates (`pt.__eq__(other, factor)`).
    pub fn near_by(self, other: Pt, factor: f64) -> bool {
        feq_by(self.x, other.x, factor) && feq_by(self.y, other.y, factor)
    }

    /// Bit-for-bit equal coordinates (`eq_exact`).
    pub fn eq_exact(self, other: Pt) -> bool {
        self.x == other.x && self.y == other.y
    }
}

impl Add for Pt {
    type Output = Pt;
    fn add(self, o: Pt) -> Pt {
        Pt::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Pt {
    type Output = Pt;
    fn sub(self, o: Pt) -> Pt {
        Pt::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f64> for Pt {
    type Output = Pt;
    fn mul(self, k: f64) -> Pt {
        Pt::new(self.x * k, self.y * k)
    }
}

/// A stem hint (`stem`): `lb` is the left or bottom edge, `rt` the right or top edge. `rt < lb`
/// marks a ghost hint, with a difference of 20 (top) or 21 (bottom).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stem {
    pub lb: f64,
    pub rt: f64,
}

/// `stem.BandMargin`.
pub const STEM_BAND_MARGIN: f64 = 30.0;

impl Stem {
    pub const fn new(lb: f64, rt: f64) -> Self {
        Self { lb, rt }
    }

    /// `Some("top")` / `Some("bottom")` for a ghost hint (`isGhost`: "high" and "low"). The
    /// widths compare exactly, as in the original.
    pub fn ghost(&self) -> Option<&'static str> {
        let width = self.rt - self.lb;
        if width == -20.0 {
            Some("top")
        } else if width == -21.0 {
            Some("bottom")
        } else {
            None
        }
    }

    /// Two stems that may stand for each other as ghosts (`ghostCompat`): one is a ghost and
    /// the other is not, and they share the ghost's edge.
    pub fn ghost_compat(&self, other: &Stem) -> bool {
        let (mine, theirs) = (self.ghost(), other.ghost());
        if mine.is_some() == theirs.is_some() {
            return false;
        }
        if mine == Some("top") || theirs == Some("top") { self.lb == other.rt } else { self.lb == other.lb }
    }

    /// A stem whose edges are reversed without being a ghost (`isBad`).
    pub fn is_bad(&self) -> bool {
        self.ghost().is_none() && (self.rt - self.lb) < 0.0
    }

    /// The charstring operands of this stem after `last` (`relVals`): the start relative to
    /// `last`'s top edge (or 0), then the width, each rounded to 4 decimals.
    pub fn rel_vals(&self, last: Option<&Stem>) -> (f64, f64) {
        let l = last.map_or(0.0, |s| s.rt);
        (norm_float(self.lb - l), norm_float(self.rt - self.lb))
    }

    /// The stem as UFO hint data stores it: position and width (`UFOVals`).
    pub fn ufo_vals(&self) -> (f64, f64) {
        (self.lb, self.rt - self.lb)
    }

    /// The edges a ghost hint stands for: both are its one edge (Type 2 charstring format,
    /// page 22).
    fn effective(&self) -> (f64, f64) {
        match self.ghost() {
            Some("top") => (self.lb, self.lb),
            Some(_) => (self.rt, self.rt),
            None => (self.lb, self.rt),
        }
    }

    /// The two stems overlap, or come within [`STEM_BAND_MARGIN`] of it (`overlaps`).
    pub fn overlaps(&self, other: &Stem) -> bool {
        let (lloc, uloc) = self.effective();
        let (olloc, ouloc) = other.effective();
        let uloc = uloc + STEM_BAND_MARGIN;
        let lloc = lloc - STEM_BAND_MARGIN;
        olloc <= uloc && ouloc >= lloc
    }

    /// Distance from `loc` to the stem, 0 inside it (`distance`).
    pub fn distance(&self, loc: f64) -> f64 {
        let (lloc, uloc) = self.effective();
        if loc >= lloc && loc <= uloc {
            return 0.0;
        }
        (lloc - loc).abs().min((uloc - loc).abs())
    }
}

/// Names a path element: its subpath, its offset in the subpath and, for an element that stands
/// in for part of a split curve during segment generation, its index in the parent's
/// substitution list ([`SegmentSub::Parts`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeRef {
    pub subpath: usize,
    pub offset: usize,
    pub sub: Option<usize>,
}

impl PeRef {
    pub const fn new(subpath: usize, offset: usize) -> Self {
        Self { subpath, offset, sub: None }
    }

    /// The element this one belongs to (itself for an element that is no substitute).
    pub fn parent(self) -> PeRef {
        PeRef { sub: None, ..self }
    }
}

/// Which half of a flex pair an element is (`pathElement.flex`: 1 or 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexPart {
    First,
    Second,
}

/// A curve split at its inflections for segment generation (`segment_sub`): the parent holds
/// the parts, each part knows its index in the parent's list.
#[derive(Clone, Debug, PartialEq)]
pub enum SegmentSub {
    Parts(Vec<PathElement>),
    Part(usize),
}

/// One line or curve of a glyph (`pathElement`).
#[derive(Clone, Debug, PartialEq)]
pub struct PathElement {
    pub is_line: bool,
    /// The implicit closing line of a subpath.
    pub is_close: bool,
    /// Start point.
    pub s: Pt,
    /// First control point (curves; equal to `s` on lines).
    pub cs: Pt,
    /// Second control point (curves; equal to `e` on lines).
    pub ce: Pt,
    /// End point.
    pub e: Pt,
    /// The hint mask written before this element. On a closing line it is written before the
    /// subpath's `moveto` instead, so it is in force from the start of the subpath.
    pub masks: Option<HintMask>,
    pub flex: Option<FlexPart>,
    /// The original's `flex` is `False` rather than `None`: true for elements made by
    /// [`PathElement::line`] and [`PathElement::curve`] (split parts, substitutes, null closes),
    /// false for elements drawn by [`GlyphData::from_contours`] and after
    /// [`GlyphData::clear_flex`]. The original tests `flex is None` in two places that therefore
    /// skip such elements: the curve-extreme segments of segment generation and
    /// [`PathElement::convert_to_line`].
    pub flex_unset: bool,
    /// Cached bounds (`getBounds`); cleared by [`PathElement::clear_temp_state`].
    pub bounds: Option<BoundsState>,
    /// (subpath, offset) in the glyph; `(-1, -1)` style "unknown" is `None`.
    pub position: Option<(usize, usize)>,
    pub segment_sub: Option<SegmentSub>,
    /// For an element of an overlap-free outline, the element of the original outline it was
    /// matched to by [`GlyphData::associate_path`] (`association`): `None` before matching (the
    /// original has no attribute then), `Some(None)` when no match was found (the original
    /// warns), `Some(Some(r))` for a match.
    pub association: Option<Option<PeRef>>,
}

/// `pathElement.assocMatchFactor`.
pub const ASSOC_MATCH_FACTOR: f64 = 93.0;
/// `pathElement.tSlop`.
pub const T_SLOP: f64 = 0.005;
/// `pathElement.middleMult`.
pub const MIDDLE_MULT: f64 = 2.0;

impl PathElement {
    fn new(is_line: bool, s: Pt, cs: Pt, ce: Pt, e: Pt) -> Self {
        Self {
            is_line,
            is_close: false,
            s,
            cs,
            ce,
            e,
            masks: None,
            flex: None,
            flex_unset: true,
            bounds: None,
            position: None,
            segment_sub: None,
            association: None,
        }
    }

    /// A line, as the original's `pathElement(s, e)`.
    pub fn line(s: Pt, e: Pt) -> Self {
        Self::new(true, s, s, e, e)
    }

    /// A curve, as the original's `pathElement(s, cs, ce, e)`.
    pub fn curve(s: Pt, cs: Pt, ce: Pt, e: Pt) -> Self {
        Self::new(false, s, cs, ce, e)
    }

    /// Bounds, computed and cached on first use (`getBounds`).
    pub fn get_bounds(&mut self) -> &BoundsState {
        let b = match self.bounds.take() {
            Some(b) => b,
            None => BoundsState::new(self),
        };
        self.bounds.insert(b)
    }

    pub fn clear_temp_state(&mut self) {
        self.bounds = None;
        self.segment_sub = None;
    }

    /// The first element of its subpath (`isStart`).
    pub fn is_start(&self) -> bool {
        matches!(self.position, Some((_, 0)))
    }

    /// Start and end points within 2 units of each other in both dimensions (`isTiny`).
    pub fn is_tiny(&self) -> bool {
        let d = (self.e - self.s).abs();
        d.x < 2.0 && d.y < 2.0
    }

    /// Start and end points within about 6 units (`isShort`, from the C hinter's `IsShort`).
    pub fn is_short(&self) -> bool {
        let d = (self.e - self.s).abs();
        // The original unpacks the sorted pair as `mx, mn`: the smaller difference comes first.
        let (first, second) = if d.y < d.x { (d.y, d.x) } else { (d.x, d.y) };
        first + second * 0.336 < 6.0
    }

    /// Turns a curve into a line with the same end points (`convertToLine`). Refused, as in the
    /// original, for an element with flex or with [`PathElement::flex_unset`].
    pub fn convert_to_line(&mut self) {
        if self.is_line || self.flex.is_some() || self.flex_unset {
            return;
        }
        self.is_line = true;
        self.cs = self.s;
        self.ce = self.e;
        self.bounds = None;
    }

    /// Turns a line into a curve with control points on it: `cs` at `s_ratio` (default .333333)
    /// of the way from the start, `ce` at `e_ratio` (default `s_ratio`) from the end
    /// (`convertToCurve`).
    pub fn convert_to_curve(&mut self, s_ratio: Option<f64>, e_ratio: Option<f64>) {
        if !self.is_line {
            return;
        }
        let s_ratio = s_ratio.unwrap_or(0.333333);
        let e_ratio = e_ratio.unwrap_or(s_ratio);
        self.is_line = false;
        self.cs = self.s * (1.0 - s_ratio) + self.e * s_ratio;
        self.ce = self.s * e_ratio + self.e * (1.0 - e_ratio);
    }

    /// Drops the vertical masks of this element, or the horizontal ones, and the whole mask
    /// when the other half is empty (`clearHints`).
    pub fn clear_hints(&mut self, vert: bool) {
        let Some(m) = self.masks.take() else { return };
        self.masks = match vert {
            true if !m.h.is_empty() => Some(HintMask { h: m.h, v: Vec::new() }),
            false if !m.v.is_empty() => Some(HintMask { h: Vec::new(), v: m.v }),
            _ => None,
        };
    }

    /// The tolerance used when matching overlap-free elements to originals: the element's
    /// length over [`ASSOC_MATCH_FACTOR`], or half of it when `loose` (`getAssocFactor`).
    pub fn assoc_factor(&self, loose: bool) -> f64 {
        let l = if self.is_line { self.s.distsq(self.e).sqrt() } else { approximate_cubic_arc_length(self.s, self.cs, self.ce, self.e) };
        l / if loose { ASSOC_MATCH_FACTOR / 2.0 } else { ASSOC_MATCH_FACTOR }
    }

    /// The charstring operands of the element (`relVals`): two for `rlineto`, six for
    /// `rrcurveto`.
    pub fn rel_vals(&self) -> Vec<f64> {
        if self.is_line {
            let r = self.e - self.s;
            vec![r.x, r.y]
        } else {
            let (r1, r2, r3) = (self.cs - self.s, self.ce - self.cs, self.e - self.ce);
            vec![r1.x, r1.y, r2.x, r2.y, r3.x, r3.y]
        }
    }

    /// Splits a curve at its inflections into [`SegmentSub::Parts`]; true when it did
    /// (`splitAtInflectionsForSegs`). The inflections are used in the order the solver gives
    /// them, as in the original.
    pub fn split_at_inflections_for_segs(&mut self) -> bool {
        if self.is_line {
            return false;
        }
        let (a, b, c, _) = self.cubic_parameters();
        let t2c = 3.0 * (b.x * a.y - a.x * b.y);
        let t1c = 3.0 * (c.x * a.y - a.x * c.y);
        let t0c = c.x * b.y - b.x * c.y;
        let sols: Vec<f64> = crate::bezier::solve_quadratic(t2c, t1c, t0c).into_iter().filter(|t| 0.0 < *t && *t < 1.0).collect();
        if sols.is_empty() {
            return false;
        }
        let parts = crate::bezier::split_cubic_at_t(self.s, self.cs, self.ce, self.e, &sols)
            .into_iter()
            .enumerate()
            .map(|(i, [p0, p1, p2, p3])| {
                let mut spe = PathElement::curve(p0, p1, p2, p3);
                spe.position = self.position;
                spe.segment_sub = Some(SegmentSub::Part(i));
                spe
            })
            .collect();
        self.segment_sub = Some(SegmentSub::Parts(parts));
        true
    }

    fn parts(&self) -> Option<&[PathElement]> {
        match &self.segment_sub {
            Some(SegmentSub::Parts(parts)) => Some(parts.as_slice()),
            _ => None,
        }
    }
}

/// A glyph's outline and hints (`glyphData`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphData {
    pub name: String,
    pub round_coords: bool,
    pub subpaths: Vec<Vec<PathElement>>,
    pub hstems: Vec<Stem>,
    pub vstems: Vec<Stem>,
    pub start_masks: Option<HintMask>,
    pub cntr: Vec<HintMask>,
    pub width: Option<f64>,
    /// The glyph's stems use hint masks (`is_hm`).
    pub is_hm: Option<bool>,
    pub flex_count: usize,
    pub changed: bool,
    pub path_edited: bool,
    /// [`GlyphData::get_bounds`]'s cache, keyed by subpath (`None`: the whole glyph); emptied by
    /// [`GlyphData::set_path_edited`] (`boundsMap`).
    pub bounds_map: BTreeMap<Option<usize>, PathBoundsState>,
}

/// What [`GlyphData::check_assoc_point`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Assoc {
    NoMatch,
    Matched,
    /// The element was split; the part still to match sits at the same index.
    Split,
}

/// The `peMap` key of a point: rounded to one decimal, with -0 and 0 equal as in a Python dict.
fn round_key(p: Pt) -> (u64, u64) {
    let r = p.round_to(1);
    ((r.x + 0.0).to_bits(), (r.y + 0.0).to_bits())
}

impl GlyphData {
    /// Draws `contours` into subpaths with the pen rules of the original: a closing line is added
    /// when the last point is not exactly the start, empty contours are dropped, coordinates are
    /// rounded when `round_coords` is set, quadratic segments are raised to cubics as fontTools'
    /// `BasePen` does. Fails on a coordinate that is not finite or beyond [`MAX_COORDINATE`], or
    /// on more than [`crate::MAX_ELEMENTS`] elements.
    pub fn from_contours(name: &str, contours: &[BezPath], round_coords: bool) -> Result<Self, AutohintError> {
        const TWO_THIRDS: f64 = 2.0 / 3.0;
        let mut gd = Self { name: name.to_string(), round_coords, ..Self::default() };
        let checked = |p: Point| -> Result<Pt, AutohintError> {
            if p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_COORDINATE && p.y.abs() <= MAX_COORDINATE {
                Ok(Pt::new(p.x, p.y))
            } else {
                Err(AutohintError::BadCoordinate { glyph: name.to_string() })
            }
        };
        let rnd = |p: Pt| Pt::rounded_if(p.x, p.y, round_coords);
        let mut count = 0usize;
        for path in contours {
            let mut cur: Option<Pt> = None;
            for el in path.iter() {
                match el {
                    PathEl::MoveTo(p) => {
                        let p = checked(p)?;
                        gd.drop_empty_last();
                        gd.subpaths.push(Vec::new());
                        cur = Some(p);
                    }
                    PathEl::LineTo(p) => {
                        let p = checked(p)?;
                        let Some(c) = cur else { continue };
                        gd.push_drawn(PathElement::line(rnd(c), rnd(p)), &mut count)?;
                        cur = Some(p);
                    }
                    PathEl::QuadTo(p1, p2) => {
                        let (p1, p2) = (checked(p1)?, checked(p2)?);
                        let Some(c) = cur else { continue };
                        let m1 = Pt::new(c.x + TWO_THIRDS * (p1.x - c.x), c.y + TWO_THIRDS * (p1.y - c.y));
                        let m2 = Pt::new(p2.x + TWO_THIRDS * (p1.x - p2.x), p2.y + TWO_THIRDS * (p1.y - p2.y));
                        gd.push_drawn(PathElement::curve(rnd(c), rnd(m1), rnd(m2), rnd(p2)), &mut count)?;
                        cur = Some(p2);
                    }
                    PathEl::CurveTo(p1, p2, p3) => {
                        let (p1, p2, p3) = (checked(p1)?, checked(p2)?, checked(p3)?);
                        let Some(c) = cur else { continue };
                        gd.push_drawn(PathElement::curve(rnd(c), rnd(p1), rnd(p2), rnd(p3)), &mut count)?;
                        cur = Some(p3);
                    }
                    PathEl::ClosePath => {
                        if let Some(c) = cur.take() {
                            gd.close_drawn(rnd(c), &mut count)?;
                        }
                    }
                }
            }
            gd.drop_empty_last();
        }
        Ok(gd)
    }

    fn drop_empty_last(&mut self) {
        if self.subpaths.last().is_some_and(Vec::is_empty) {
            self.subpaths.pop();
        }
    }

    /// Appends a pen-drawn element to the last subpath (`getPosition`, `checkFlex` with no flex
    /// pending).
    fn push_drawn(&mut self, mut pe: PathElement, count: &mut usize) -> Result<(), AutohintError> {
        if *count >= MAX_ELEMENTS {
            return Err(AutohintError::LimitExceeded { what: "path elements in one glyph", limit: MAX_ELEMENTS });
        }
        let si = self.subpaths.len().saturating_sub(1);
        let Some(sp) = self.subpaths.last_mut() else { return Ok(()) };
        pe.position = Some((si, sp.len()));
        pe.flex_unset = false;
        sp.push(pe);
        *count += 1;
        Ok(())
    }

    /// `_closePath`: drops a subpath with nothing after its `moveto`, else adds the closing line
    /// unless the current point is exactly the start.
    fn close_drawn(&mut self, cur: Pt, count: &mut usize) -> Result<(), AutohintError> {
        let Some(sp) = self.subpaths.last() else { return Ok(()) };
        let Some(first) = sp.first() else {
            self.subpaths.pop();
            return Ok(());
        };
        let start = first.s;
        if cur.eq_exact(start) {
            return Ok(());
        }
        let mut pe = PathElement::line(cur, start);
        pe.is_close = true;
        self.push_drawn(pe, count)
    }

    /// Takes existing hints: stems in absolute form, the start mask, masks before elements,
    /// counter masks and flex (`hStem`, `vStem`, `hintmask`, `cntrmask`, `nextIsFlex`).
    ///
    /// A mask at [`crate::ElementPos`] `p` is stored on the element `p` names, so a mask on a
    /// subpath's closing line is in force from the start of that subpath, as the original writes
    /// it. Positions that name no element are ignored. A flex position names the first curve of
    /// the pair; a line in either place ends the pair, as the original's `checkFlex` does.
    pub fn set_hints(&mut self, hints: &GlyphHints) {
        self.hstems.extend(hints.h.iter().copied());
        self.vstems.extend(hints.v.iter().copied());
        let has_masks = hints.start_mask.is_some() || !hints.masks.is_empty() || !hints.counters.is_empty();
        if has_masks || !hints.h.is_empty() || !hints.v.is_empty() {
            self.is_hm = Some(has_masks);
        }
        if let Some(m) = &hints.start_mask {
            self.start_masks = Some(m.clone());
        }
        for (pos, m) in &hints.masks {
            if let Some(pe) = self.subpaths.get_mut(pos.subpath).and_then(|sp| sp.get_mut(pos.element)) {
                pe.masks = Some(m.clone());
            }
        }
        self.cntr.extend(hints.counters.iter().cloned());
        for pos in &hints.flex {
            self.flex_count += 1;
            let Some(sp) = self.subpaths.get_mut(pos.subpath) else { continue };
            match sp.get_mut(pos.element) {
                Some(pe) if !pe.is_line => {
                    pe.flex = Some(FlexPart::First);
                    pe.flex_unset = false;
                }
                _ => continue,
            }
            if let Some(pe) = sp.get_mut(pos.element + 1).filter(|pe| !pe.is_line) {
                pe.flex = Some(FlexPart::Second);
                pe.flex_unset = false;
            }
        }
    }

    /// The element `r` names, substitutes included.
    pub fn element(&self, r: PeRef) -> Option<&PathElement> {
        let pe = self.subpaths.get(r.subpath)?.get(r.offset)?;
        match (r.sub, &pe.segment_sub) {
            (None, _) => Some(pe),
            (Some(i), Some(SegmentSub::Parts(parts))) => parts.get(i),
            (Some(_), _) => None,
        }
    }

    pub fn element_mut(&mut self, r: PeRef) -> Option<&mut PathElement> {
        let pe = self.subpaths.get_mut(r.subpath)?.get_mut(r.offset)?;
        match r.sub {
            None => Some(pe),
            Some(i) => match &mut pe.segment_sub {
                Some(SegmentSub::Parts(parts)) => parts.get_mut(i),
                _ => None,
            },
        }
    }

    /// Every element in path order, without substitutes (`__iter__`).
    pub fn elements(&self) -> impl Iterator<Item = PeRef> + '_ {
        self.subpaths.iter().enumerate().flat_map(|(s, sp)| (0..sp.len()).map(move |o| PeRef::new(s, o)))
    }

    /// Number of elements, without substitutes.
    pub fn element_count(&self) -> usize {
        self.subpaths.iter().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.subpaths.is_empty()
    }

    pub fn has_flex(&self) -> bool {
        self.flex_count > 0
    }

    /// The glyph carries stems in the given dimension (`hasHints`).
    pub fn has_hints(&self, vert: bool) -> bool {
        if vert { !self.vstems.is_empty() } else { !self.hstems.is_empty() }
    }

    /// Notes that subpaths were reordered or elements inserted: positions are rewritten by
    /// [`GlyphData::sync_positions`] and the bounds cache is emptied (`setPathEdited`).
    pub fn set_path_edited(&mut self) {
        self.path_edited = true;
        self.bounds_map.clear();
    }

    /// Rewrites every element's `position`, and the index of every substitute, after an edit
    /// (`syncPositions`). Does nothing when the path was never edited.
    pub fn sync_positions(&mut self) {
        if !self.path_edited {
            return;
        }
        for (si, sp) in self.subpaths.iter_mut().enumerate() {
            for (ofst, pe) in sp.iter_mut().enumerate() {
                pe.position = Some((si, ofst));
                if let Some(SegmentSub::Parts(parts)) = &mut pe.segment_sub {
                    for (i, spe) in parts.iter_mut().enumerate() {
                        spe.position = Some((si, ofst));
                        spe.segment_sub = Some(SegmentSub::Part(i));
                    }
                }
            }
        }
    }

    /// The glyph's bounds, or those of one subpath (`getBounds`). *Shared.*
    ///
    /// Results are cached until [`GlyphData::set_path_edited`]. The original builds a subpath's
    /// bounds in the bounds list of the subpath's first element, so computing them also widens
    /// that element's cached bounds to the subpath's (and its chord bounds when the two are one
    /// list, [`BoundsState::shared`]). Later curve-extreme tests on that element see the wider
    /// bounds; this port does the same so that its hints agree with the original's.
    pub fn get_bounds(&mut self, subpath: Option<usize>) -> Option<PathBoundsState> {
        if let Some(b) = self.bounds_map.get(&subpath) {
            return Some(b.clone());
        }
        if self.subpaths.is_empty() {
            return None;
        }
        let b = match subpath {
            None => {
                let mut b = self.get_bounds(Some(0))?;
                for i in 1..self.subpaths.len() {
                    if let Some(o) = self.get_bounds(Some(i)) {
                        b.merge(&o);
                    }
                }
                b
            }
            Some(si) => {
                let sp = self.subpaths.get_mut(si)?;
                let mut iter = sp.iter_mut().enumerate();
                let (_, first) = iter.next()?;
                let mut b = PathBoundsState::new(PeRef::new(si, 0), first.get_bounds());
                for (k, c) in iter {
                    b.merge(&PathBoundsState::new(PeRef::new(si, k), c.get_bounds()));
                }
                if let Some(fb) = sp.first_mut().and_then(|pe| pe.bounds.as_mut()) {
                    fb.bounds = b.bounds;
                    if fb.shared {
                        fb.linear = b.bounds;
                    }
                }
                b
            }
        };
        self.bounds_map.insert(subpath, b.clone());
        Some(b)
    }

    /// Reorders the subpaths: subpath `i` of the result is subpath `new_order[i]` (`reorder`).
    /// An order that is not a permutation of the subpaths changes nothing. *Shared* (M10.12).
    pub fn reorder(&mut self, new_order: &[usize]) {
        let n = self.subpaths.len();
        let mut seen = vec![false; n];
        for &i in new_order {
            match seen.get_mut(i) {
                Some(s) if !*s => *s = true,
                _ => return,
            }
        }
        if new_order.len() != n {
            return;
        }
        let mut old: Vec<Option<Vec<PathElement>>> = std::mem::take(&mut self.subpaths).into_iter().map(Some).collect();
        self.subpaths = new_order.iter().filter_map(|&i| old.get_mut(i).and_then(Option::take)).collect();
        self.set_path_edited();
        self.sync_positions();
    }

    /// The element at (`subpath`, `offset`), or its first substitute when `seg_sub` and it has
    /// any.
    fn entry(&self, subpath: usize, offset: usize, seg_sub: bool) -> Option<PeRef> {
        let pe = self.subpaths.get(subpath)?.get(offset)?;
        match pe.parts() {
            Some(parts) if seg_sub && !parts.is_empty() => Some(PeRef { subpath, offset, sub: Some(0) }),
            _ => Some(PeRef::new(subpath, offset)),
        }
    }

    /// The first element, or the first substitute of a split first element when `seg_sub`
    /// (`next(self)`). *Shared.*
    pub fn first(&self, seg_sub: bool) -> Option<PeRef> {
        self.entry(0, 0, seg_sub)
    }

    /// The last element of the last subpath (`last`).
    pub fn last(&self) -> Option<PeRef> {
        let si = self.subpaths.len().checked_sub(1)?;
        let off = self.subpaths.get(si)?.len().checked_sub(1)?;
        Some(PeRef::new(si, off))
    }

    /// The element after `c` in path order (`next`): the next substitute of the same parent,
    /// else the next element (its first substitute when `seg_sub`). *Shared.*
    pub fn next(&self, c: PeRef, seg_sub: bool) -> Option<PeRef> {
        let sp = self.subpaths.get(c.subpath)?;
        if let Some(si) = c.sub
            && let Some(parts) = sp.get(c.offset).and_then(PathElement::parts)
            && parts.len() > si + 1
        {
            return Some(PeRef { sub: Some(si + 1), ..c });
        }
        let offset = c.offset + 1;
        if offset < sp.len() {
            return self.entry(c.subpath, offset, seg_sub);
        }
        let subpath = c.subpath + 1;
        if subpath < self.subpaths.len() {
            return self.entry(subpath, 0, seg_sub);
        }
        None
    }

    /// The first element in hint order: the first subpath's closing line when it has one, else
    /// its first element (`nextForHints(self)`).
    pub fn first_for_hints(&self) -> Option<PeRef> {
        self.subpath_start_for_hints(0)
    }

    fn subpath_start_for_hints(&self, subpath: usize) -> Option<PeRef> {
        let sp = self.subpaths.get(subpath)?;
        let last = sp.len().checked_sub(1)?;
        if sp.get(last).is_some_and(|pe| pe.is_close) { Some(PeRef::new(subpath, last)) } else { Some(PeRef::new(subpath, 0)) }
    }

    /// The element after `c` for hint distribution (`nextForHints`): each subpath starts with
    /// its closing line, if any, then runs from its first element. As in the original, the
    /// last element of a subpath that has no closing line is not visited. *Shared.*
    pub fn next_for_hints(&self, c: PeRef) -> Option<PeRef> {
        let sp = self.subpaths.get(c.subpath)?;
        let pe = sp.get(c.offset)?;
        if pe.is_close {
            return if c.offset == 0 { None } else { Some(PeRef::new(c.subpath, 0)) };
        }
        let offset = c.offset + 1;
        if offset + 1 < sp.len() && sp.get(offset).is_some_and(|n| !n.is_close) {
            return Some(PeRef::new(c.subpath, offset));
        }
        self.subpath_start_for_hints(c.subpath + 1)
    }

    /// `inSubpath`: steps `dir` (1 or -1) through `c`'s subpath, through substitutes when
    /// `seg_sub`, past tiny elements when `skip_tiny`; `None` after a full turn, or when the walk
    /// would wrap past the subpath's end and `close_wrap_ok` is false.
    fn in_subpath(&self, c: PeRef, dir: isize, skip_tiny: bool, close_wrap_ok: bool, seg_sub: bool) -> Option<PeRef> {
        let sp = self.subpaths.get(c.subpath)?;
        sp.get(c.offset)?;
        let len = isize::try_from(sp.len()).ok().filter(|l| *l > 0)?;
        let offset = isize::try_from(c.offset).ok()?;
        let mut o = offset;
        let mut cur = c;
        loop {
            let mut done = false;
            if let Some(si) = cur.sub
                && let Some(parts) = usize::try_from(o).ok().and_then(|o| sp.get(o)).and_then(PathElement::parts)
            {
                let suboff = isize::try_from(si).ok()? + dir;
                if let Ok(suboff) = usize::try_from(suboff)
                    && suboff < parts.len()
                {
                    cur = PeRef { sub: Some(suboff), ..cur };
                    done = true;
                }
            }
            if !done {
                o = (o + dir).rem_euclid(len);
                if o == offset {
                    return None;
                }
                let ou = usize::try_from(o).ok()?;
                let t = sp.get(ou)?;
                cur = match t.parts() {
                    Some(parts) if seg_sub && !parts.is_empty() => {
                        let suboff = if dir < 0 { parts.len() - 1 } else { 0 };
                        PeRef { subpath: c.subpath, offset: ou, sub: Some(suboff) }
                    }
                    _ => PeRef::new(c.subpath, ou),
                };
            }
            if !skip_tiny || !self.element(cur)?.is_tiny() {
                break;
            }
            if !close_wrap_ok && ((dir > 0 && o < offset) || (dir < 0 && o > offset)) {
                return None;
            }
        }
        Some(cur)
    }

    /// The next element of `c`'s subpath, wrapping when `close_wrap_ok` (`nextInSubpath`; the
    /// original's defaults are `false, true, false`). *Shared.*
    pub fn next_in_subpath(&self, c: PeRef, skip_tiny: bool, close_wrap_ok: bool, seg_sub: bool) -> Option<PeRef> {
        self.in_subpath(c, 1, skip_tiny, close_wrap_ok, seg_sub)
    }

    /// The previous element of `c`'s subpath (`prevInSubpath`). *Shared.*
    pub fn prev_in_subpath(&self, c: PeRef, skip_tiny: bool, close_wrap_ok: bool, seg_sub: bool) -> Option<PeRef> {
        self.in_subpath(c, -1, skip_tiny, close_wrap_ok, seg_sub)
    }

    /// The slope point at the start of the next non-tiny element (`nextSlopePoint`). *Shared.*
    pub fn next_slope_point(&self, c: PeRef) -> Option<Pt> {
        let n = self.next_in_subpath(c, true, true, true)?;
        Some(self.element(n)?.slope_point(0.0))
    }

    /// The slope point at the end of the previous non-tiny element (`prevSlopePoint`). *Shared.*
    pub fn prev_slope_point(&self, c: PeRef) -> Option<Pt> {
        let p = self.prev_in_subpath(c, true, true, true)?;
        Some(self.element(p)?.slope_point(1.0))
    }

    /// Matches the elements of this overlap-free outline to those of `orig` (`associatePath`),
    /// splitting an element where an original element ends partway along it. Each element's
    /// [`PathElement::association`] is set. *Shared* (M10.9).
    ///
    /// The original takes the elements from a set, in no fixed order; this port takes them in
    /// path order. The outcome for each element does not depend on the order.
    pub fn associate_path(&mut self, orig: &GlyphData, loose: bool) {
        let orig_refs: Vec<PeRef> = orig.elements().collect();
        let mut pe_map: HashMap<(u64, u64), Vec<PeRef>> = HashMap::new();
        for &r in &orig_refs {
            if let Some(oc) = orig.element(r) {
                pe_map.entry(round_key(oc.e)).or_default().push(r);
            }
        }
        let mut si = 0;
        while si < self.subpaths.len() {
            let mut idx = 0;
            while idx < self.subpaths.get(si).map_or(0, Vec::len) {
                let pending = self.subpaths.get(si).and_then(|sp| sp.get(idx)).is_some_and(|pe| pe.association.is_none());
                if !pending || self.associate_one(orig, &orig_refs, &pe_map, si, idx, loose) != Assoc::Split {
                    idx += 1;
                }
            }
            si += 1;
        }
        self.sync_positions();
    }

    /// One pass of `associatePath`'s loop for the element at (`si`, `idx`).
    fn associate_one(
        &mut self,
        orig: &GlyphData,
        orig_refs: &[PeRef],
        pe_map: &HashMap<(u64, u64), Vec<PeRef>>,
        si: usize,
        idx: usize,
        loose: bool,
    ) -> Assoc {
        let Some(c) = self.subpaths.get(si).and_then(|sp| sp.get(idx)) else { return Assoc::NoMatch };
        let (cs, ce) = (c.s, c.e);
        // An original element that ends where this one ends, or the one after it.
        for &oepe in pe_map.get(&round_key(ce)).map_or(&[][..], Vec::as_slice) {
            let Some(oe) = orig.element(oepe) else { continue };
            let r = self.check_assoc_point(si, idx, orig, oepe, cs, oe.s, true, loose, None);
            if r != Assoc::NoMatch {
                return r;
            }
            if let Some(oepen) = orig.next_in_subpath(oepe, false, true, false)
                && let Some(on) = orig.element(oepen)
            {
                let r = self.check_assoc_point(si, idx, orig, oepen, cs, on.e, true, loose, None);
                if r != Assoc::NoMatch {
                    return r;
                }
            }
        }
        // An original element that ends where this one starts: the one after it, or itself.
        for &ospe in pe_map.get(&round_key(cs)).map_or(&[][..], Vec::as_slice) {
            if let Some(ospen) = orig.next_in_subpath(ospe, false, true, false)
                && let Some(on) = orig.element(ospen)
            {
                let r = self.check_assoc_point(si, idx, orig, ospen, ce, on.e, false, loose, None);
                if r != Assoc::NoMatch {
                    return r;
                }
            }
            let Some(os) = orig.element(ospe) else { continue };
            let r = self.check_assoc_point(si, idx, orig, ospe, ce, os.s, false, loose, None);
            if r != Assoc::NoMatch {
                return r;
            }
        }
        // Any original element near this one that holds its start point, then its end point.
        let Some(c_bounds) = self.subpaths.get_mut(si).and_then(|sp| sp.get_mut(idx)).map(|pe| pe.get_bounds().clone()) else {
            return Assoc::NoMatch;
        };
        for from_end in [false, true] {
            for &oc_ref in orig_refs {
                let Some(oc) = orig.element(oc_ref) else { continue };
                let factor = oc.assoc_factor(loose);
                let oc_bounds = oc.bounds.clone().unwrap_or_else(|| BoundsState::new(oc));
                if !c_bounds.intersects(&oc_bounds, factor) {
                    continue;
                }
                let (held, other, map_end) = if from_end { (ce, cs, true) } else { (cs, ce, false) };
                if !oc.contains_point(held, factor, false).0 {
                    continue;
                }
                let (first, second) = if from_end { (oc.s, oc.e) } else { (oc.e, oc.s) };
                for op in [first, second] {
                    let r = self.check_assoc_point(si, idx, orig, oc_ref, other, op, map_end, loose, Some(factor));
                    if r != Assoc::NoMatch {
                        return r;
                    }
                }
            }
        }
        if let Some(pe) = self.subpaths.get_mut(si).and_then(|sp| sp.get_mut(idx)) {
            pe.association = Some(None);
        }
        Assoc::Matched
    }

    /// `checkAssocPoint`: matches the element at (`si`, `idx`) to `ope` of `orig` when point
    /// `sp` of the element is on `ope` (or equals `op`), or splits the element where `op` lies on
    /// it. `map_end` says the element's end is the matched side.
    #[allow(clippy::too_many_arguments)]
    fn check_assoc_point(
        &mut self,
        si: usize,
        idx: usize,
        orig: &GlyphData,
        ope_ref: PeRef,
        sp: Pt,
        op: Pt,
        map_end: bool,
        loose: bool,
        factor: Option<f64>,
    ) -> Assoc {
        let Some(ope) = orig.element(ope_ref) else { return Assoc::NoMatch };
        let factor = factor.unwrap_or_else(|| ope.assoc_factor(loose));
        let total = self.element_count();
        let Some(list) = self.subpaths.get_mut(si) else { return Assoc::NoMatch };
        let Some(spe) = list.get_mut(idx) else { return Assoc::NoMatch };
        if sp.near(op) || ope.contains_point(sp, factor, false).0 {
            if !ope.contains_point(spe.at_t(0.5), factor * MIDDLE_MULT, false).0 {
                return Assoc::NoMatch;
            }
            spe.association = Some(Some(ope_ref));
            return Assoc::Matched;
        }
        let (does, t) = spe.contains_point(op, factor, true);
        let t = t.unwrap_or(0.0);
        if !(does && T_SLOP < t && t < 1.0 - T_SLOP) {
            return Assoc::NoMatch;
        }
        let mid_mapped = if map_end { 1.0 - (1.0 - t) / 2.0 } else { t / 2.0 };
        if !ope.contains_point(spe.at_t(mid_mapped), factor * MIDDLE_MULT, false).0 || total >= MAX_ELEMENTS {
            return Assoc::NoMatch;
        }
        let mut following = spe.split_at(t);
        if map_end {
            following.association = Some(Some(ope_ref));
            list.insert(idx + 1, following);
        } else {
            // The original inserts the second part before the first; kept, as later steps
            // read the subpath in that order.
            spe.association = Some(Some(ope_ref));
            list.insert(idx, following);
        }
        self.set_path_edited();
        Assoc::Split
    }

    /// Adds a zero-length closing line to subpath `si` (`addNullClose`); nothing when the
    /// subpath already ends with one.
    pub fn add_null_close(&mut self, si: usize) {
        let Some(sp) = self.subpaths.get_mut(si) else { return };
        let (Some(first), Some(last)) = (sp.first(), sp.last()) else { return };
        if last.is_close {
            return;
        }
        let mut pe = PathElement::line(first.s, first.s);
        pe.is_close = true;
        pe.position = Some((si, sp.len()));
        sp.push(pe);
    }

    /// Clears every flex mark (`clearFlex`).
    pub fn clear_flex(&mut self) {
        if self.flex_count == 0 {
            return;
        }
        for pe in self.subpaths.iter_mut().flatten() {
            pe.flex = None;
            pe.flex_unset = false;
        }
        self.flex_count = 0;
    }

    /// Drops the stems of one dimension and that half of the start mask, every counter mask and
    /// every element's mask (`clearHints`). *Shared.*
    pub fn clear_hints(&mut self, vert: bool) {
        let keep = |m: HintMask| -> Option<HintMask> {
            match vert {
                true if !m.h.is_empty() => Some(HintMask { h: m.h, v: Vec::new() }),
                false if !m.v.is_empty() => Some(HintMask { h: Vec::new(), v: m.v }),
                _ => None,
            }
        };
        if vert {
            self.vstems.clear();
        } else {
            self.hstems.clear();
        }
        self.start_masks = self.start_masks.take().and_then(keep);
        self.cntr = std::mem::take(&mut self.cntr).into_iter().filter_map(keep).collect();
        for pe in self.subpaths.iter_mut().flatten() {
            pe.clear_hints(vert);
        }
    }

    pub fn clear_temp_state(&mut self) {
        for sp in &mut self.subpaths {
            for pe in sp {
                pe.clear_temp_state();
            }
        }
    }

    /// The result in the public form: the stems, the start mask, every element's mask at the
    /// element's position, the counter masks and the first element of each flex pair. A mask on
    /// a subpath's closing element (the implicit closing line, or the zero-length one added to
    /// match another master) is written before that subpath's `moveto`, as otfautohint does.
    pub fn to_hints(&self) -> crate::GlyphHints {
        let mut masks = Vec::new();
        let mut flex = Vec::new();
        for (subpath, sp) in self.subpaths.iter().enumerate() {
            for (element, pe) in sp.iter().enumerate() {
                let pos = crate::ElementPos { subpath, element };
                if let Some(m) = &pe.masks {
                    masks.push((pos, m.clone()));
                }
                if pe.flex == Some(FlexPart::First) {
                    flex.push(pos);
                }
            }
        }
        crate::GlyphHints {
            h: self.hstems.clone(),
            v: self.vstems.clone(),
            start_mask: self.start_masks.clone(),
            masks,
            counters: self.cntr.clone(),
            flex,
            warnings: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Expected values from `tests/oracle/glyph_data.py`, which runs otfautohint's
    //! `glyphData.py` on the same glyphs.
    use super::*;
    use crate::ElementPos;

    fn pt(x: f64, y: f64) -> Pt {
        Pt::new(x, y)
    }

    fn r(subpath: usize, offset: usize, sub: Option<usize>) -> PeRef {
        PeRef { subpath, offset, sub }
    }

    enum Op {
        M(f64, f64),
        L(f64, f64),
        C(f64, f64, f64, f64, f64, f64),
        Z,
    }
    use Op::*;

    fn path(ops: &[Op]) -> BezPath {
        let mut p = BezPath::new();
        for op in ops {
            match *op {
                M(x, y) => p.move_to((x, y)),
                L(x, y) => p.line_to((x, y)),
                C(a, b, c, d, e, f) => p.curve_to((a, b), (c, d), (e, f)),
                Z => p.close_path(),
            }
        }
        p
    }

    fn glyph(contours: &[&[Op]]) -> GlyphData {
        let paths: Vec<BezPath> = contours.iter().map(|c| path(c)).collect();
        GlyphData::from_contours("g", &paths, false).unwrap()
    }

    /// An `o`: the outer contour ends on its start point, the inner one does not.
    fn bowl() -> GlyphData {
        glyph(&[
            &[
                M(250.0, 0.0),
                C(388.0, 0.0, 500.0, 112.0, 500.0, 250.0),
                C(500.0, 388.0, 388.0, 500.0, 250.0, 500.0),
                C(112.0, 500.0, 0.0, 388.0, 0.0, 250.0),
                C(0.0, 112.0, 112.0, 0.0, 250.0, 0.0),
                Z,
            ],
            &[
                M(250.0, 100.0),
                C(167.0, 100.0, 100.0, 167.0, 100.0, 250.0),
                C(100.0, 333.0, 167.0, 400.0, 250.0, 400.0),
                C(333.0, 400.0, 400.0, 333.0, 400.0, 250.0),
                C(400.0, 167.0, 333.0, 100.0, 260.0, 100.0),
                Z,
            ],
        ])
    }

    /// A cupped serif: the first element rises above its chord.
    fn cup() -> GlyphData {
        glyph(&[&[M(0.0, 0.0), C(30.0, 100.0, 70.0, 100.0, 100.0, 0.0), L(100.0, -200.0), L(0.0, -200.0), Z]])
    }

    #[test]
    fn stems_ghosts_overlaps_and_operands_match_otfautohint() {
        let stems = [
            Stem::new(10.0, 90.0),
            Stem::new(500.0, 480.0),
            Stem::new(-21.0, -42.0),
            Stem::new(100.0, 50.0),
            Stem::new(500.0, 520.0),
            Stem::new(479.0, 500.0),
        ];
        let ghost: Vec<_> = stems.iter().map(Stem::ghost).collect();
        assert_eq!(ghost, [None, Some("top"), Some("bottom"), None, None, None]);
        let bad: Vec<_> = stems.iter().map(Stem::is_bad).collect();
        assert_eq!(bad, [false, false, false, true, false, false]);
        let last = Stem::new(3.33333, 7.77777);
        let rel: Vec<_> = stems.iter().map(|s| (s.rel_vals(None), s.rel_vals(Some(&last)))).collect();
        assert_eq!(
            rel,
            [
                ((10.0, 80.0), (2.2222, 80.0)),
                ((500.0, -20.0), (492.2222, -20.0)),
                ((-21.0, -21.0), (-28.7778, -21.0)),
                ((100.0, -50.0), (92.2222, -50.0)),
                ((500.0, 20.0), (492.2222, 20.0)),
                ((479.0, 21.0), (471.2222, 21.0)),
            ]
        );
        let dist: Vec<Vec<f64>> = stems.iter().map(|s| [0.0, 50.0, 485.0, 600.0].iter().map(|v| s.distance(*v)).collect()).collect();
        assert_eq!(
            dist,
            [
                vec![10.0, 0.0, 395.0, 510.0],
                vec![500.0, 450.0, 15.0, 100.0],
                vec![42.0, 92.0, 527.0, 642.0],
                vec![50.0, 0.0, 385.0, 500.0],
                vec![500.0, 450.0, 15.0, 80.0],
                vec![479.0, 429.0, 0.0, 100.0],
            ]
        );
        let (t, f) = (true, false);
        let overlaps: Vec<Vec<bool>> = stems.iter().map(|a| stems.iter().map(|b| a.overlaps(b)).collect()).collect();
        assert_eq!(
            overlaps,
            [
                vec![t, f, f, t, f, f],
                vec![f, t, f, f, t, t],
                vec![f, f, t, f, f, f],
                vec![t, f, f, f, f, f],
                vec![f, t, f, f, t, t],
                vec![f, t, f, f, t, t],
            ]
        );
        let compat: Vec<(usize, usize)> =
            (0..6).flat_map(|i| (0..6).map(move |j| (i, j))).filter(|&(i, j)| stems[i].ghost_compat(&stems[j])).collect();
        assert_eq!(compat, [(1, 5)]);
    }

    #[test]
    fn python_rounding_is_reproduced() {
        // (value, round(v, 4), round(v, 1), round(v, 6)) as Python gives them.
        let cases = [
            (0.35, 0.35, 0.3, 0.35),
            (1.00005, 1.0001, 1.0, 1.00005),
            (2.00015, 2.0002, 2.0, 2.00015),
            (0.12345, 0.1235, 0.1, 0.12345),
            (-1.23456789, -1.2346, -1.2, -1.234568),
            (1e-5, 0.0, 0.0, 1e-5),
            (12.34565, 12.3456, 12.3, 12.34565),
            (2.675, 2.675, 2.7, 2.675),
            (1e6 + 0.00005, 1000000.0001, 1000000.0, 1000000.00005),
            (-0.00004, -0.0, -0.0, -0.00004),
        ];
        for (v, r4, r1, r6) in cases {
            assert_eq!((round_digits(v, 4), round_digits(v, 1), round_digits(v, 6)), (r4, r1, r6), "{v}");
            assert_eq!(norm_float(v), r4);
        }
    }

    #[test]
    fn contours_draw_closing_lines_and_hint_order_skips_as_the_original() {
        let mut g = bowl();
        let lens: Vec<usize> = g.subpaths.iter().map(Vec::len).collect();
        assert_eq!(lens, [4, 5]);
        let closes: Vec<PeRef> = g.elements().filter(|e| g.element(*e).unwrap().is_close).collect();
        assert_eq!(closes, [r(1, 4, None)]);
        assert!(g.elements().all(|e| !g.element(e).unwrap().flex_unset));

        // Hint order: a subpath starts with its closing line; the last element of a subpath
        // without one, (0, 3), is never visited.
        let mut order = Vec::new();
        let mut c = g.first_for_hints();
        while let Some(e) = c {
            order.push(e);
            c = g.next_for_hints(e);
        }
        let want: Vec<PeRef> = [(0, 0), (0, 1), (0, 2), (1, 4), (1, 0), (1, 1), (1, 2), (1, 3)].iter().map(|&(s, o)| r(s, o, None)).collect();
        assert_eq!(order, want);

        let b = g.get_bounds(None).unwrap();
        assert_eq!(b.bounds, [[0.0, 0.0], [500.0, 500.0]]);
        assert_eq!(b.extpes, [[r(0, 2, None), r(0, 0, None)], [r(0, 0, None), r(0, 1, None)]]);
    }

    #[test]
    fn subpath_bounds_widen_the_first_elements_bounds_as_the_original() {
        let mut g = cup();
        let b = g.get_bounds(Some(0)).unwrap();
        assert_eq!(b.bounds, [[0.0, -200.0], [100.0, 75.0]]);
        assert_eq!(b.extpes, [[r(0, 0, None), r(0, 1, None)], [r(0, 0, None), r(0, 0, None)]]);
        let first = g.subpaths[0][0].get_bounds().clone();
        assert_eq!(first.bounds, [[0.0, -200.0], [100.0, 75.0]]);
        assert_eq!(first.linear, [[0.0, 0.0], [100.0, 0.0]]);
        // The curve-extreme test now reports the subpath's bottom, with stale extreme data.
        assert_eq!(first.farthest_extreme(true), (200.0, pt(0.0, 0.0), 0.0, false));
    }

    #[test]
    fn subpath_walks_skip_tiny_elements_and_follow_substitutes() {
        let g = glyph(&[&[M(0.0, 0.0), L(100.0, 0.0), L(101.0, 1.0), L(101.0, 100.0), L(0.0, 100.0), Z]]);
        assert_eq!(g.subpaths[0].len(), 5);
        assert_eq!(g.next_in_subpath(r(0, 0, None), true, true, false), Some(r(0, 2, None)));
        assert_eq!(g.prev_in_subpath(r(0, 2, None), true, true, false), Some(r(0, 0, None)));
        assert_eq!(g.next_in_subpath(r(0, 4, None), false, true, false), Some(r(0, 0, None)));
        assert_eq!(g.next_in_subpath(r(0, 4, None), false, false, false), Some(r(0, 0, None)));
        assert_eq!(g.prev_in_subpath(r(0, 0, None), false, true, false), Some(r(0, 4, None)));
        assert_eq!(g.next_slope_point(r(0, 0, None)), Some(pt(101.0, 100.0)));
        assert_eq!(g.prev_slope_point(r(0, 2, None)), Some(pt(0.0, 0.0)));

        let mut g = glyph(&[&[M(0.0, 0.0), C(150.0, 50.0, -50.0, 150.0, 100.0, 200.0), L(0.0, 200.0), Z]]);
        assert!(g.subpaths[0][0].split_at_inflections_for_segs());
        let part = |g: &GlyphData, i| {
            let e = g.element(r(0, 0, Some(i))).unwrap();
            (e.s, e.cs, e.ce, e.e, e.segment_sub.clone())
        };
        assert_eq!(part(&g, 0), (pt(0.0, 0.0), pt(75.0, 25.0), pt(62.5, 62.5), pt(50.0, 100.0), Some(SegmentSub::Part(0))));
        assert_eq!(part(&g, 1), (pt(50.0, 100.0), pt(37.5, 137.5), pt(25.0, 175.0), pt(100.0, 200.0), Some(SegmentSub::Part(1))));
        let mut walk = Vec::new();
        let mut c = g.first(true);
        while let Some(e) = c {
            walk.push(e);
            c = g.next(e, true);
        }
        assert_eq!(walk, [r(0, 0, Some(0)), r(0, 0, Some(1)), r(0, 1, None), r(0, 2, None)]);
        assert_eq!(g.next_in_subpath(r(0, 0, Some(0)), false, true, true), Some(r(0, 0, Some(1))));
        assert_eq!(g.prev_in_subpath(r(0, 1, None), false, true, true), Some(r(0, 0, Some(1))));
        assert_eq!(g.prev_in_subpath(r(0, 0, Some(0)), false, true, true), Some(r(0, 2, None)));
        g.clear_temp_state();
        assert_eq!(g.first(true), Some(r(0, 0, None)));
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn overlap_free_outline_is_matched_and_split_as_the_original() {
        // Two overlapping rectangles and their union, a T on its side.
        let orig = glyph(&[
            &[M(0.0, 0.0), L(100.0, 0.0), L(100.0, 300.0), L(0.0, 300.0), Z],
            &[M(0.0, 200.0), L(300.0, 200.0), L(300.0, 300.0), L(0.0, 300.0), Z],
        ]);
        let mut derived = glyph(&[&[M(0.0, 0.0), L(100.0, 0.0), L(100.0, 200.0), L(300.0, 200.0), L(300.0, 300.0), L(0.0, 300.0), Z]]);
        derived.associate_path(&orig, false);
        let got: Vec<(Option<(usize, usize)>, Pt, Pt, Option<Option<PeRef>>)> =
            derived.subpaths[0].iter().map(|pe| (pe.position, pe.s, pe.e, pe.association)).collect();
        let a = |s, o| Some(Some(r(s, o, None)));
        assert_eq!(
            got,
            [
                (Some((0, 0)), pt(0.0, 0.0), pt(100.0, 0.0), a(0, 0)),
                (Some((0, 1)), pt(100.0, 0.0), pt(100.0, 200.0), a(0, 1)),
                (Some((0, 2)), pt(100.0, 200.0), pt(300.0, 200.0), a(1, 0)),
                (Some((0, 3)), pt(300.0, 200.0), pt(300.0, 300.0), a(1, 1)),
                (Some((0, 4)), pt(300.0, 300.0), pt(100.0, 300.0), a(1, 2)),
                (Some((0, 5)), pt(100.0, 300.0), pt(0.0, 300.0), a(0, 2)),
                (Some((0, 6)), pt(0.0, 300.0), pt(0.0, 0.0), a(0, 3)),
            ]
        );
        // The split part is a new element: the original's `flex` is False on it.
        assert!(derived.subpaths[0][5].flex_unset && !derived.subpaths[0][4].flex_unset);
    }

    #[test]
    fn hints_masks_and_flex_are_taken_and_cleared() {
        let mut g = bowl();
        let mask = |h: &[bool], v: &[bool]| HintMask { h: h.to_vec(), v: v.to_vec() };
        let hints = GlyphHints {
            h: vec![Stem::new(0.0, 100.0)],
            v: vec![Stem::new(0.0, 100.0), Stem::new(400.0, 500.0)],
            start_mask: Some(mask(&[true], &[true, false])),
            masks: vec![
                (ElementPos { subpath: 1, element: 4 }, mask(&[], &[false, true])),
                (ElementPos { subpath: 9, element: 0 }, mask(&[true], &[])),
            ],
            counters: vec![mask(&[true], &[true, true])],
            flex: vec![ElementPos { subpath: 0, element: 0 }],
            warnings: Vec::new(),
        };
        g.set_hints(&hints);
        assert_eq!(g.is_hm, Some(true));
        assert_eq!((g.subpaths[0][0].flex, g.subpaths[0][1].flex, g.flex_count), (Some(FlexPart::First), Some(FlexPart::Second), 1));
        assert_eq!(g.subpaths[1][4].masks, Some(mask(&[], &[false, true])));

        g.clear_hints(true);
        assert!(g.vstems.is_empty() && g.has_hints(false));
        assert_eq!(g.start_masks, Some(mask(&[true], &[])));
        assert_eq!(g.cntr, [mask(&[true], &[])]);
        assert_eq!(g.subpaths[1][4].masks, None);
        g.clear_hints(false);
        assert_eq!((g.start_masks.clone(), g.cntr.len()), (None, 0));

        g.clear_flex();
        assert!(g.elements().all(|e| g.element(e).unwrap().flex.is_none()) && !g.has_flex());
    }

    #[test]
    fn edits_follow_the_original() {
        let mut g = cup();
        // A drawn curve becomes a line; a curve made by splitting does not (`flex` is False).
        let mut part = g.subpaths[0][0].split_at(0.5);
        part.convert_to_line();
        assert!(!part.is_line);
        g.subpaths[0][0].convert_to_line();
        let pe = &g.subpaths[0][0];
        assert!(pe.is_line && pe.cs == pe.s && pe.ce == pe.e);

        let mut line = PathElement::line(pt(0.0, 0.0), pt(300.0, 0.0));
        line.convert_to_curve(Some(0.25), None);
        assert_eq!((line.is_line, line.cs, line.ce), (false, pt(75.0, 0.0), pt(225.0, 0.0)));
        assert_eq!(line.rel_vals(), [75.0, 0.0, 150.0, 0.0, 75.0, 0.0]);
        assert!(PathElement::line(pt(0.0, 0.0), pt(1.5, -1.5)).is_tiny());
        // `isShort` weighs the larger difference by .336, so a 17-unit flat line is short.
        assert!(PathElement::line(pt(0.0, 0.0), pt(17.0, 0.0)).is_short());
        assert!(!PathElement::line(pt(0.0, 0.0), pt(5.0, 5.0)).is_short());

        let mut g = bowl();
        g.add_null_close(0);
        g.add_null_close(1);
        assert_eq!((g.subpaths[0].len(), g.subpaths[1].len()), (5, 5));
        assert_eq!(g.subpaths[0][4].position, Some((0, 4)));
        g.reorder(&[1, 1]);
        assert_eq!(g.subpaths[0][0].s, pt(250.0, 0.0));
        g.reorder(&[1, 0]);
        assert_eq!((g.subpaths[0][0].s, g.subpaths[0][0].position), (pt(250.0, 100.0), Some((0, 0))));
    }

    #[test]
    fn hostile_contours_fail_with_a_message() {
        let nan = path(&[M(0.0, 0.0), L(f64::NAN, 10.0), Z]);
        assert!(matches!(GlyphData::from_contours("a", &[nan], true), Err(AutohintError::BadCoordinate { .. })));
        let far = path(&[M(0.0, 0.0), L(2.0e6, 10.0), Z]);
        assert!(matches!(GlyphData::from_contours("a", &[far], true), Err(AutohintError::BadCoordinate { .. })));
        let mut long = BezPath::new();
        long.move_to((0.0, 0.0));
        for i in 0..=MAX_ELEMENTS {
            long.line_to(((i % 2) as f64, i as f64));
        }
        assert!(matches!(GlyphData::from_contours("a", &[long], false), Err(AutohintError::LimitExceeded { .. })));
        // Empty contours and drawing without a start point are dropped, not fatal.
        let odd = path(&[M(0.0, 0.0), Z, M(5.0, 5.0)]);
        assert!(GlyphData::from_contours("a", &[odd], false).unwrap().is_empty());
        // Walks from references that name nothing end.
        let g = bowl();
        assert_eq!(g.next(r(7, 0, None), false), None);
        assert_eq!(g.next_in_subpath(r(0, 99, None), true, false, true), None);
        assert_eq!(g.next_for_hints(r(0, 99, None)), None);
    }

    #[test]
    fn rounded_drawing_rounds_every_point() {
        let p = path(&[M(0.5, 1.5), L(10.4, 2.5), L(10.6, 20.0), Z]);
        let g = GlyphData::from_contours("a", &[p], true).unwrap();
        let pts: Vec<(Pt, Pt)> = g.subpaths[0].iter().map(|pe| (pe.s, pe.e)).collect();
        assert_eq!(pts, [(pt(0.0, 2.0), pt(10.0, 2.0)), (pt(10.0, 2.0), pt(11.0, 20.0)), (pt(11.0, 20.0), pt(0.0, 2.0))]);
    }
}
