//! The glyph as the autohinter sees it: points, stems, path elements and the glyph's subpaths.
//!
//! Ported from otfautohint's `glyphData.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`). Package M10.8 fills the bodies marked `// M10.8:`; the types are fixed by M10.1.
//!
//! Differences from the original, by design:
//! - `pt.a` / `pt.o` take the dimension as an argument instead of reading a thread-local.
//! - A path element is named by [`PeRef`] instead of by object identity. `glyphData` used itself
//!   as the "before the first element" cursor; here [`GlyphData::first`] starts a walk and
//!   [`GlyphData::next`] continues it.
//! - The pen methods (`moveTo`, `lineTo`, `curveToOne`, `closePath`) become
//!   [`GlyphData::from_contours`], which draws every contour with the same rules.
//! - Existing hints (the `hStem`/`vStem`/`hintmask` quasi-pen methods) are read by
//!   [`GlyphData::set_hints`]; FontCraft passes hints that way when a glyph keeps manual ones.

use std::ops::{Add, Mul, Sub};

use fontcraft_geom::BezPath;

use crate::bezier::BoundsState;
use crate::{AutohintError, HintMask};

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

/// `a` and `b` are equal within [`FEQ_FACTOR`] (`feq`).
pub fn feq(a: f64, b: f64) -> bool {
    (a - b).abs() < FEQ_FACTOR
}

/// `a` and `b` differ by at least [`FEQ_FACTOR`] (`fne`).
pub fn fne(a: f64, b: f64) -> bool {
    (a - b).abs() >= FEQ_FACTOR
}

/// A value rounded to 4 decimals, as `norm_float` does before printing.
pub fn norm_float(v: f64) -> f64 {
    // M10.8: Python's round(v, 4) rounds the decimal expansion correctly; check the cases where
    // this differs and match them.
    (v * 1e4).round_ties_even() / 1e4
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

    /// Equal within [`FEQ_FACTOR`] on both coordinates (`pt.__eq__`).
    pub fn near(self, other: Pt) -> bool {
        feq(self.x, other.x) && feq(self.y, other.y)
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

    /// `Some("top")` / `Some("bottom")` for a ghost hint (`isGhost`).
    pub fn ghost(&self) -> Option<&'static str> {
        // M10.8: isGhost.
        None
    }

    /// Two stems that may stand for each other as ghosts (`ghostCompat`).
    pub fn ghost_compat(&self, other: &Stem) -> bool {
        // M10.8: ghostCompat.
        let _ = other;
        false
    }

    /// A stem whose edges are reversed without being a ghost (`isBad`).
    pub fn is_bad(&self) -> bool {
        // M10.8: isBad.
        false
    }

    /// The charstring operands of this stem after `last` (`relVals`).
    pub fn rel_vals(&self, last: Option<&Stem>) -> (f64, f64) {
        // M10.8: relVals.
        let _ = last;
        (self.lb, self.rt - self.lb)
    }

    /// The stem as UFO hint data stores it: position and width (`UFOVals`).
    pub fn ufo_vals(&self) -> (f64, f64) {
        // M10.8: UFOVals.
        (self.lb, self.rt - self.lb)
    }

    /// The two stems overlap, ghosts counted with the band margin (`overlaps`).
    pub fn overlaps(&self, other: &Stem) -> bool {
        // M10.8: overlaps.
        let _ = other;
        false
    }

    /// Distance from `loc` to the stem, 0 inside it (`distance`).
    pub fn distance(&self, loc: f64) -> f64 {
        // M10.8: distance.
        let _ = loc;
        0.0
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
    /// The hint mask written before this element.
    pub masks: Option<HintMask>,
    pub flex: Option<FlexPart>,
    /// Cached bounds (`getBounds`); cleared by [`PathElement::clear_temp_state`].
    pub bounds: Option<BoundsState>,
    /// (subpath, offset) in the glyph; `(-1, -1)` style "unknown" is `None`.
    pub position: Option<(usize, usize)>,
    pub segment_sub: Option<SegmentSub>,
}

/// `pathElement.assocMatchFactor`.
pub const ASSOC_MATCH_FACTOR: f64 = 93.0;
/// `pathElement.tSlop`.
pub const T_SLOP: f64 = 0.005;
/// `pathElement.middleMult`.
pub const MIDDLE_MULT: f64 = 2.0;

impl PathElement {
    pub fn line(s: Pt, e: Pt) -> Self {
        Self { is_line: true, is_close: false, s, cs: s, ce: e, e, masks: None, flex: None, bounds: None, position: None, segment_sub: None }
    }

    pub fn curve(s: Pt, cs: Pt, ce: Pt, e: Pt) -> Self {
        Self { is_line: false, is_close: false, s, cs, ce, e, masks: None, flex: None, bounds: None, position: None, segment_sub: None }
    }

    /// Bounds, computed and cached on first use (`getBounds`).
    pub fn get_bounds(&mut self) -> &BoundsState {
        // M10.8: compute BoundsState::new(self) when None.
        self.bounds.get_or_insert_with(BoundsState::default)
    }

    pub fn clear_temp_state(&mut self) {
        self.bounds = None;
        self.segment_sub = None;
    }

    /// The first element of its subpath (`isStart`).
    pub fn is_start(&self) -> bool {
        matches!(self.position, Some((_, 0)))
    }

    /// Shorter than otfautohint's tiny limit (`isTiny`).
    pub fn is_tiny(&self) -> bool {
        // M10.8: isTiny.
        false
    }

    /// Shorter than otfautohint's short limit (`isShort`).
    pub fn is_short(&self) -> bool {
        // M10.8: isShort.
        false
    }

    /// Turns a curve whose control points are on its chord into a line (`convertToLine`).
    pub fn convert_to_line(&mut self) {
        // M10.8: convertToLine.
    }

    /// Turns a line into a curve with control points at thirds, or at the given ones
    /// (`convertToCurve`).
    pub fn convert_to_curve(&mut self, s_ratio: Option<f64>, e_ratio: Option<f64>) {
        // M10.8: convertToCurve.
        let _ = (s_ratio, e_ratio);
    }

    /// Drops the masks of this element in one dimension or both (`clearHints`).
    pub fn clear_hints(&mut self, vert: bool) {
        // M10.8: clearHints.
        let _ = vert;
    }

    /// The tolerance used when matching overlap-free elements to originals (`getAssocFactor`).
    pub fn assoc_factor(&self, loose: bool) -> f64 {
        // M10.8: getAssocFactor.
        let _ = loose;
        0.0
    }

    /// The charstring operands of the element (`relVals`).
    pub fn rel_vals(&self) -> Vec<f64> {
        // M10.8: relVals.
        Vec::new()
    }

    /// Splits a curve at its inflections into [`SegmentSub::Parts`]; true when it did
    /// (`splitAtInflectionsForSegs`).
    pub fn split_at_inflections_for_segs(&mut self) -> bool {
        // M10.8: splitAtInflectionsForSegs.
        false
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
}

impl GlyphData {
    /// Draws `contours` into subpaths with the pen rules of the original: a closing line is added
    /// when the last point is not the start, empty contours are dropped, coordinates are rounded
    /// when `round_coords` is set. Fails on a coordinate that is not finite or on more than
    /// [`crate::MAX_ELEMENTS`] elements.
    pub fn from_contours(name: &str, contours: &[BezPath], round_coords: bool) -> Result<Self, AutohintError> {
        // M10.8: the pen methods.
        let _ = contours;
        Ok(Self { name: name.to_string(), round_coords, ..Self::default() })
    }

    /// Takes existing hints: stems in absolute form, the start mask, masks before elements,
    /// counter masks and flex (`hStem`, `vStem`, `hintmask`, `cntrmask`, `nextIsFlex`).
    pub fn set_hints(&mut self, hints: &crate::GlyphHints) {
        // M10.8: the quasi-pen methods.
        let _ = hints;
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

    pub fn is_empty(&self) -> bool {
        self.subpaths.is_empty()
    }

    pub fn has_flex(&self) -> bool {
        self.flex_count > 0
    }

    /// The glyph carries hints in the given dimension (`hasHints`).
    pub fn has_hints(&self, vert: bool) -> bool {
        // M10.8: hasHints also looks at masks.
        if vert { !self.vstems.is_empty() } else { !self.hstems.is_empty() }
    }

    /// Rewrites every element's `position` after an edit (`syncPositions`).
    pub fn sync_positions(&mut self) {
        // M10.8: syncPositions.
    }

    /// The glyph's bounds, or those of one subpath (`getBounds`). *Shared.*
    pub fn get_bounds(&mut self, subpath: Option<usize>) -> Option<crate::bezier::PathBoundsState> {
        // M10.8: getBounds.
        let _ = subpath;
        None
    }

    /// Reorders the subpaths (`reorder`). *Shared* (M10.12).
    pub fn reorder(&mut self, new_order: &[usize]) {
        // M10.8: reorder.
        let _ = new_order;
    }

    /// The first element, or the first substitute of a split first element when `seg_sub`
    /// (`next(self)`). *Shared.*
    pub fn first(&self, seg_sub: bool) -> Option<PeRef> {
        // M10.8: next(self, segSub).
        let _ = seg_sub;
        None
    }

    /// The element after `c` in path order (`next`). *Shared.*
    pub fn next(&self, c: PeRef, seg_sub: bool) -> Option<PeRef> {
        // M10.8: next.
        let _ = (c, seg_sub);
        None
    }

    /// The element after `c` for hint distribution (`nextForHints`). *Shared.*
    pub fn next_for_hints(&self, c: PeRef) -> Option<PeRef> {
        // M10.8: nextForHints.
        let _ = c;
        None
    }

    /// The next element of `c`'s subpath, wrapping when `close_wrap_ok` (`nextInSubpath`).
    /// *Shared.*
    pub fn next_in_subpath(&self, c: PeRef, skip_tiny: bool, close_wrap_ok: bool, seg_sub: bool) -> Option<PeRef> {
        // M10.8: inSubpath(c, 1, …).
        let _ = (c, skip_tiny, close_wrap_ok, seg_sub);
        None
    }

    /// The previous element of `c`'s subpath (`prevInSubpath`). *Shared.*
    pub fn prev_in_subpath(&self, c: PeRef, skip_tiny: bool, close_wrap_ok: bool, seg_sub: bool) -> Option<PeRef> {
        // M10.8: inSubpath(c, -1, …).
        let _ = (c, skip_tiny, close_wrap_ok, seg_sub);
        None
    }

    /// The slope point at the start of the next non-tiny element (`nextSlopePoint`). *Shared.*
    pub fn next_slope_point(&self, c: PeRef) -> Option<Pt> {
        // M10.8: nextSlopePoint.
        let _ = c;
        None
    }

    /// The slope point at the end of the previous non-tiny element (`prevSlopePoint`). *Shared.*
    pub fn prev_slope_point(&self, c: PeRef) -> Option<Pt> {
        // M10.8: prevSlopePoint.
        let _ = c;
        None
    }

    /// Matches the elements of `orig` to this overlap-free outline (`associatePath`). *Shared*
    /// (M10.9).
    pub fn associate_path(&mut self, orig: &GlyphData, loose: bool) {
        // M10.8: associatePath, checkAssocPoint.
        let _ = (orig, loose);
    }

    /// Adds a zero-length closing line to subpath `si` (`addNullClose`).
    pub fn add_null_close(&mut self, si: usize) {
        // M10.8: addNullClose.
        let _ = si;
    }

    pub fn clear_flex(&mut self) {
        // M10.8: clearFlex.
    }

    /// Drops the hints of one dimension (`clearHints`). *Shared.*
    pub fn clear_hints(&mut self, vert: bool) {
        // M10.8: clearHints (masks of every element too).
        if vert { self.vstems.clear() } else { self.hstems.clear() }
    }

    pub fn clear_temp_state(&mut self) {
        for sp in &mut self.subpaths {
            for pe in sp {
                pe.clear_temp_state();
            }
        }
    }

    /// The result in the public form.
    pub fn to_hints(&self) -> crate::GlyphHints {
        // M10.12: masks before elements and flex positions from the elements.
        crate::GlyphHints {
            h: self.hstems.clone(),
            v: self.vstems.clone(),
            start_mask: self.start_masks.clone(),
            counters: self.cntr.clone(),
            ..Default::default()
        }
    }
}
