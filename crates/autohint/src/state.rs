//! Intermediate state built while one glyph is hinted in one dimension: hint segments, stem
//! values, per-element segment lists, location candidates for other masters, and subpath links.
//!
//! Ported from otfautohint's `hintstate.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`). The types are fixed by M10.1; M10.11 fills the bodies marked `// M10.11:`.
//!
//! Python object references are arena indexes here: [`SegId`] into
//! [`GlyphHintState::segments`], [`StemId`] into [`GlyphHintState::values`]. The ordered lists
//! the original keeps (`stemValues`, `mainValues`, `increasingSegs`…) hold ids. Arenas only grow
//! during one glyph; a removed segment is marked `deleted` or replaced (`replaced_by`).

use std::collections::BTreeMap;

use crate::glyph::{PeRef, Stem};

/// Index of a [`HintSegment`] in [`GlyphHintState::segments`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SegId(pub usize);

/// Index of a [`StemValue`] in [`GlyphHintState::values`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StemId(pub usize);

/// Where a segment came from (`hintSegment.sType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SegType {
    Line = 1,
    Bend = 2,
    Curve = 3,
    /// From the lower bound of a subpath.
    LsBbox = 4,
    /// From the upper bound of a subpath.
    UsBbox = 5,
    /// From the lower bound of the whole glyph.
    LgBbox = 6,
    /// From the upper bound of the whole glyph.
    UgBbox = 7,
    Ghost = 8,
}

/// One side of a possible stem (`hintSegment`).
#[derive(Clone, Debug, PartialEq)]
pub struct HintSegment {
    /// Location in the aligned dimension ([`crate::glyph::Pt::a`]).
    pub loc: f64,
    /// Extent in the opposite dimension.
    pub min: f64,
    pub max: f64,
    /// 0 for ordinary segments, more for special ones (inside a zone…).
    pub bonus: f64,
    pub typ: SegType,
    pub is_v: bool,
    /// The segment runs towards increasing opposite coordinates.
    pub is_inc: bool,
    /// How the segment was found, for reports and debugging.
    pub desc: String,
    /// The element the segment came from (a weak reference in the original).
    pub pe: Option<PeRef>,
    pub hint_val: Option<StemId>,
    pub replaced_by: Option<SegId>,
    pub deleted: bool,
    pub suppressed: bool,
}

impl HintSegment {
    pub fn is_bend(&self) -> bool {
        self.typ == SegType::Bend
    }
    pub fn is_curve(&self) -> bool {
        self.typ == SegType::Curve
    }
    /// A line, or a bounding-box segment (`isLine`).
    pub fn is_line(&self) -> bool {
        self.typ == SegType::Line || self.is_bbox()
    }
    pub fn is_bbox(&self) -> bool {
        matches!(self.typ, SegType::LsBbox | SegType::UsBbox | SegType::LgBbox | SegType::UgBbox)
    }
    pub fn is_gbbox(&self) -> bool {
        matches!(self.typ, SegType::LgBbox | SegType::UgBbox)
    }
    pub fn is_ubbox(&self) -> bool {
        matches!(self.typ, SegType::UsBbox | SegType::UgBbox)
    }
    pub fn is_ghost(&self) -> bool {
        self.typ == SegType::Ghost
    }
}

/// A possible stem (`stemValue`).
#[derive(Clone, Debug, PartialEq)]
pub struct StemValue {
    pub val: f64,
    /// Spacing bonus (`spc`).
    pub spc: f64,
    /// Lower and upper location; `lloc <= uloc`.
    pub lloc: f64,
    pub uloc: f64,
    pub is_ghost: bool,
    pub pruned: bool,
    pub merge: bool,
    pub lseg: SegId,
    pub useg: SegId,
    pub best: Option<StemId>,
    pub initial_val: f64,
    pub idx: Option<usize>,
}

impl StemValue {
    /// The stem range with ghosts normalised (`ghosted`): the lower segment a ghost gives
    /// `(uloc, uloc − 20)`, else `(lloc + 21, lloc)`. Needs the segments to tell which side is
    /// the ghost.
    pub fn ghosted(&self, segments: &[HintSegment]) -> (f64, f64) {
        // M10.11: ghosted.
        let _ = segments;
        (self.lloc, self.uloc)
    }

    /// `val` and `initial_val` as a comparable pair, scaled for ghosts and spacing (`compVal`).
    pub fn comp_val(&self, spc_factor: f64, ghost_factor: f64) -> (f64, f64) {
        let mut v = self.val;
        if self.is_ghost {
            v *= ghost_factor;
        }
        if self.spc > 0.0 {
            v *= spc_factor;
        }
        (v, self.initial_val)
    }
}

/// The segments found at the start, middle and end of one element, and its mask
/// (`pathElementHintState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeHintState {
    pub s_segs: Vec<SegId>,
    pub m_segs: Vec<SegId>,
    pub e_segs: Vec<SegId>,
    pub mask: Vec<bool>,
}

/// Which list of a [`PeHintState`] (`segLists` keys `s`, `m`, `e`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegPlace {
    Start,
    Middle,
    End,
}

/// Whether overlaps were looked at and what came of it (`overlapRemoved`: `None`, `False` or a
/// glyph).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum OverlapState {
    #[default]
    Unchecked,
    NoOverlap,
    Removed(Box<crate::glyph::GlyphData>),
}

/// The state of one glyph in one dimension (`glyphHintState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphHintState {
    /// Arena of every segment made for this glyph.
    pub segments: Vec<HintSegment>,
    /// Arena of every stem value made for this glyph.
    pub values: Vec<StemValue>,
    /// Per-element segment lists (`peStates`).
    pub pe_states: BTreeMap<PeRef, PeHintState>,
    pub overlap_removed: OverlapState,
    pub increasing_segs: Vec<SegId>,
    pub decreasing_segs: Vec<SegId>,
    /// In increasing order of position (`stemValues`).
    pub stem_values: Vec<StemId>,
    /// Non-overlapping main values in decreasing order of value (`mainValues`).
    pub main_values: Vec<StemId>,
    pub reject_values: Vec<StemId>,
    pub counter_hinted: bool,
    /// The chosen stems, per master (`stems`); `None` until converted.
    pub stems: Option<Vec<Vec<Stem>>>,
    pub weights: Vec<f64>,
    pub keep_hints: Option<bool>,
    pub has_overlaps: bool,
    /// `stem_overlaps[n][m]`: stem n overlaps stem m.
    pub stem_overlaps: Vec<Vec<bool>>,
    /// For a ghost stem m, `ghost_compat[m][n]`: n can stand for m.
    pub ghost_compat: Vec<Option<Vec<bool>>>,
    pub good_mask: Vec<bool>,
    pub main_mask: Vec<bool>,
}

impl GlyphHintState {
    pub fn seg(&self, id: SegId) -> Option<&HintSegment> {
        self.segments.get(id.0)
    }

    pub fn seg_mut(&mut self, id: SegId) -> Option<&mut HintSegment> {
        self.segments.get_mut(id.0)
    }

    pub fn value(&self, id: StemId) -> Option<&StemValue> {
        self.values.get(id.0)
    }

    pub fn value_mut(&mut self, id: StemId) -> Option<&mut StemValue> {
        self.values.get_mut(id.0)
    }

    /// Adds a segment to the arena and returns its id.
    pub fn push_segment(&mut self, seg: HintSegment) -> SegId {
        self.segments.push(seg);
        SegId(self.segments.len() - 1)
    }

    /// Adds a stem value to the arena (not to [`GlyphHintState::stem_values`]) and returns its id.
    pub fn push_value(&mut self, value: StemValue) -> StemId {
        self.values.push(value);
        StemId(self.values.len() - 1)
    }

    /// The segment `id` stands for after replacements (`current`), stopping on a cycle.
    pub fn current(&self, id: SegId) -> SegId {
        // M10.11: follow replaced_by with cycle detection.
        id
    }

    /// The state of element `pe`, made when `make` (`getPEState`, `getPEStateForced`).
    pub fn pe_state(&mut self, pe: PeRef, make: bool) -> Option<&mut PeHintState> {
        if make { Some(self.pe_states.entry(pe).or_default()) } else { self.pe_states.get_mut(&pe) }
    }

    /// Adds a segment from `fr` to `to` at `loc` for elements `pe1` (and `pe2`) at the given
    /// places, recording it in the increasing or decreasing list (`addSegment`). *Shared*
    /// (M10.9).
    #[allow(clippy::too_many_arguments)]
    pub fn add_segment(
        &mut self,
        fr: f64,
        to: f64,
        loc: f64,
        pe1: Option<PeRef>,
        pe2: Option<PeRef>,
        typ: SegType,
        bonus: f64,
        is_v: bool,
        mid1: bool,
        mid2: bool,
        desc: &str,
    ) -> Option<SegId> {
        // M10.11: addSegment.
        let _ = (fr, to, loc, pe1, pe2, typ, bonus, is_v, mid1, mid2, desc);
        None
    }

    /// Merges segments that coincide in each list (`compactLists`, `compactList`). *Shared*.
    pub fn compact_lists(&mut self) {
        // M10.11: compactLists.
    }

    /// Removes bends that duplicate other segments (`remExtraBends`). *Shared*.
    pub fn rem_extra_bends(&mut self) {
        // M10.11: remExtraBends.
    }

    /// Drops deleted segments from every list (`deleteSegments`). *Shared*.
    pub fn delete_segments(&mut self) {
        // M10.11: deleteSegments.
    }

    /// Applies replacements and deletions to every element's lists (`cleanup`). *Shared*.
    pub fn cleanup(&mut self) {
        // M10.11: cleanup.
    }

    /// Drops segments without a stem value from every element's lists (`pruneHintSegs`).
    /// *Shared*.
    pub fn prune_hint_segs(&mut self) {
        // M10.11: pruneHintSegs.
    }
}

/// `stemLocCandidate.strongMultiplier`.
pub const STRONG_MULTIPLIER: f64 = 1.2;
/// `stemLocCandidate.bandMultiplier`.
pub const BAND_MULTIPLIER: f64 = 2.0;

/// A candidate location for a stem in another master (`stemLocCandidate`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StemLocCandidate {
    pub loc: f64,
    pub strong: f64,
    pub weak: f64,
}

impl StemLocCandidate {
    pub fn add_score(&mut self, score: f64, strong: bool) {
        if strong {
            self.strong += score;
        } else {
            self.weak += score;
        }
    }

    pub fn weight(&self, in_band: bool) -> f64 {
        (self.strong * STRONG_MULTIPLIER + self.weak) * if in_band { BAND_MULTIPLIER } else { 1.0 }
    }

    pub fn is_strong(&self) -> bool {
        self.strong > 0.0
    }

    pub fn is_mixed(&self) -> bool {
        self.strong > 0.0 && self.weak > 0.0
    }
}

/// Choosing the lower and upper location of one stem in another master (`instanceStemState`).
/// The original keeps the hinter to ask `inBand`; here [`InstanceStemState::best_location`]
/// takes that test as an argument.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstanceStemState {
    pub default_loc: f64,
    /// Candidates by location, in the order they were added (`candDict`).
    pub candidates: Vec<StemLocCandidate>,
    pub used_segs: std::collections::BTreeSet<usize>,
    /// Whether the data came from bounding boxes (`bb`); mixing is reported.
    pub bb: Option<bool>,
}

impl InstanceStemState {
    pub fn new(loc: f64) -> Self {
        Self { default_loc: loc, ..Self::default() }
    }

    /// Adds `score` at `loc` (`addToLoc`). *Shared* (M10.11 masks).
    pub fn add_to_loc(&mut self, loc: f64, score: f64, strong: bool, bb: bool, seg: Option<usize>) {
        // M10.11: addToLoc.
        let _ = (loc, score, strong, bb, seg);
    }

    /// The best location, or `None` when no candidate exists (`bestLocation`).
    pub fn best_location(&self, is_bottom: bool, in_band: &dyn Fn(f64, bool) -> bool) -> Option<f64> {
        // M10.11: bestLocation.
        let _ = (is_bottom, in_band);
        None
    }
}

/// Which subpaths need which stems, to choose a subpath order that needs fewer substitutions
/// (`links`). Glyphs with fewer than 4 or more than [`Links::MAX_SUBPATHS`] subpaths get no links.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Links {
    pub cnt: usize,
    /// `cnt × cnt` of 0 or 1.
    pub links: Vec<Vec<u8>>,
}

impl Links {
    /// `links.__init__`'s upper bound on subpaths.
    pub const MAX_SUBPATHS: usize = 100;

    pub fn new(subpaths: usize) -> Self {
        if !(4..=Self::MAX_SUBPATHS).contains(&subpaths) {
            return Self::default();
        }
        Self { cnt: subpaths, links: vec![vec![0; subpaths]; subpaths] }
    }

    /// Marks the subpaths linked by each stem value (`mark`). *Shared* (M10.9).
    pub fn mark(&mut self, hs: &GlyphHintState, is_v: bool) {
        // M10.11: mark.
        let _ = (hs, is_v);
    }

    /// A subpath order that reduces substitution, or `None` to keep the order (`shuffle`).
    /// *Shared* (M10.12).
    pub fn shuffle(&self) -> Option<Vec<usize>> {
        // M10.11: shuffle, moveIdx.
        None
    }
}
