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

use crate::glyph::{PeRef, Stem, feq};

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
        if !self.is_ghost {
            return (self.lloc, self.uloc);
        }
        if segments.get(self.lseg.0).is_some_and(HintSegment::is_ghost) { (self.uloc, self.uloc - 20.0) } else { (self.lloc + 21.0, self.lloc) }
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

impl PeHintState {
    /// The start, middle and end segments in that order (`segments`).
    pub fn segments(&self) -> Vec<SegId> {
        self.s_segs.iter().chain(&self.m_segs).chain(&self.e_segs).copied().collect()
    }

    /// The three lists, the one at `first` first (`segLists`): start, middle, end; middle,
    /// start, end; or end, middle, start.
    pub fn seg_lists(&self, first: SegPlace) -> [(SegPlace, &[SegId]); 3] {
        let s = (SegPlace::Start, self.s_segs.as_slice());
        let m = (SegPlace::Middle, self.m_segs.as_slice());
        let e = (SegPlace::End, self.e_segs.as_slice());
        match first {
            SegPlace::Start => [s, m, e],
            SegPlace::Middle => [m, s, e],
            SegPlace::End => [e, m, s],
        }
    }

    /// Follows replacements and drops deleted segments (`pathElementHintState.cleanup`).
    fn cleanup(&mut self, segments: &[HintSegment]) {
        for list in [&mut self.s_segs, &mut self.m_segs, &mut self.e_segs] {
            let kept = list.iter().map(|&id| current_in(segments, id)).filter(|id| segments.get(id.0).is_some_and(|s| !s.deleted)).collect();
            *list = kept;
        }
    }

    /// Drops segments without a stem value (`pathElementHintState.pruneHintSegs`).
    fn prune_hint_segs(&mut self, segments: &[HintSegment]) {
        for list in [&mut self.s_segs, &mut self.m_segs, &mut self.e_segs] {
            list.retain(|id| segments.get(id.0).is_some_and(|s| s.hint_val.is_some()));
        }
    }
}

/// The segment `id` stands for after replacements. The original stops only on a segment that
/// replaces itself and recurses forever on a longer cycle; here a walk longer than the arena
/// stops too.
fn current_in(segments: &[HintSegment], id: SegId) -> SegId {
    let mut cur = id;
    for _ in 0..=segments.len() {
        match segments.get(cur.0).and_then(|s| s.replaced_by) {
            None => return cur,
            Some(next) if next == cur => return cur,
            Some(next) => cur = next,
        }
    }
    cur
}

/// The fields of a segment that compacting reads and writes.
fn extent(segments: &[HintSegment], id: SegId) -> Option<(f64, f64, f64, f64)> {
    segments.get(id.0).map(|s| (s.loc, s.min, s.max, s.bonus))
}

/// Merges overlapping segments at the same location in one sorted list (`compactList`): the
/// longer one absorbs the other's extent and bonus and the other is marked replaced by it.
fn compact_list(list: &mut Vec<SegId>, segments: &mut [HintSegment]) {
    let mut i = 0;
    'outer: while i < list.len() {
        let mut j = i + 1;
        while j < list.len() {
            let (Some(&si), Some(&sj)) = (list.get(i), list.get(j)) else { break };
            let (Some((iloc, imin, imax, ibonus)), Some((jloc, jmin, jmax, jbonus))) = (extent(segments, si), extent(segments, sj)) else {
                break;
            };
            if !feq(jloc, iloc) {
                break;
            }
            if imax >= jmin && imin <= jmax {
                let (min, max, bonus) = (imin.min(jmin), imax.max(jmax), ibonus.max(jbonus));
                if (imax - imin).abs() > (jmax - jmin).abs() {
                    if let Some(s) = segments.get_mut(si.0) {
                        (s.min, s.max, s.bonus) = (min, max, bonus);
                    }
                    if let Some(s) = segments.get_mut(sj.0) {
                        s.replaced_by = Some(si);
                    }
                    list.remove(j);
                    continue;
                }
                if let Some(s) = segments.get_mut(sj.0) {
                    (s.min, s.max, s.bonus) = (min, max, bonus);
                }
                if let Some(s) = segments.get_mut(si.0) {
                    s.replaced_by = Some(sj);
                }
                list.remove(i);
                // The original steps `i` back and forward again: the same index is examined
                // next, now holding the following segment.
                continue 'outer;
            }
            j += 1;
        }
        i += 1;
    }
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
    /// While set, [`GlyphHintState::add_segment`] appends to the segment lists and
    /// [`GlyphHintState::sort_segment_lists`] orders them once at the end: inserting each segment
    /// in place costs the length of the list, and a hostile glyph makes tens of thousands.
    pub defer_sort: bool,
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
        current_in(&self.segments, id)
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
        // pe1 receives the segment at its end (or middle), pe2 at its start (or middle): a
        // segment made at the first point of a subpath belongs to the previous element too.
        let (min, max, is_inc) = if fr > to { (to, fr, false) } else { (fr, to, true) };
        let pe = pe2.or(pe1)?;
        let id = self.push_segment(HintSegment {
            loc,
            min,
            max,
            bonus,
            typ,
            is_v,
            is_inc,
            desc: desc.to_string(),
            pe: Some(pe),
            hint_val: None,
            replaced_by: None,
            deleted: false,
            suppressed: false,
        });
        if let Some(p) = pe1 {
            let st = self.pe_states.entry(p).or_default();
            if mid1 { st.m_segs.push(id) } else { st.e_segs.push(id) }
        }
        if let Some(p) = pe2 {
            let st = self.pe_states.entry(p).or_default();
            if mid2 { st.m_segs.push(id) } else { st.s_segs.push(id) }
        }
        // bisect.insort: after every segment at the same or a lower location.
        let segments = &self.segments;
        let list = if is_inc { &mut self.increasing_segs } else { &mut self.decreasing_segs };
        if self.defer_sort {
            list.push(id);
        } else {
            let pos = list.partition_point(|s| segments.get(s.0).is_some_and(|s| s.loc <= loc));
            list.insert(pos, id);
        }
        Some(id)
    }

    /// Ends [`GlyphHintState::defer_sort`]: a stable sort by location gives the order that
    /// inserting each segment after those at the same or a lower location gives.
    pub fn sort_segment_lists(&mut self) {
        self.defer_sort = false;
        let segments = &self.segments;
        // `+ 0.0` makes -0 equal to 0, as the comparison in `add_segment` does.
        let key = |id: &SegId| segments.get(id.0).map_or(f64::INFINITY, |s| s.loc + 0.0);
        for list in [&mut self.increasing_segs, &mut self.decreasing_segs] {
            list.sort_by(|a, b| key(a).total_cmp(&key(b)));
        }
    }

    /// Merges segments that coincide in each list (`compactLists`, `compactList`). *Shared*.
    pub fn compact_lists(&mut self) {
        compact_list(&mut self.decreasing_segs, &mut self.segments);
        compact_list(&mut self.increasing_segs, &mut self.segments);
    }

    /// Removes bends that duplicate other segments (`remExtraBends`). *Shared*.
    ///
    /// A bend is deleted when a segment of the other direction at the same location, neither a
    /// bend nor a ghost, overlaps it and is more than three times as long.
    pub fn rem_extra_bends(&mut self) {
        let dec = self.decreasing_segs.clone();
        for &hsi_id in &self.increasing_segs {
            let Some(hsi) = self.segments.get(hsi_id.0) else { continue };
            let (iloc, imin, imax, ityp) = (hsi.loc, hsi.min, hsi.max, hsi.typ);
            let segments = &self.segments;
            let lo = dec.partition_point(|d| segments.get(d.0).is_some_and(|d| d.loc < iloc));
            let rest = dec.get(lo..).unwrap_or_default();
            let hi = rest.partition_point(|d| segments.get(d.0).is_some_and(|d| d.loc <= iloc));
            for &hsd_id in rest.get(..hi).unwrap_or_default() {
                let Some(hsd) = self.segments.get(hsd_id.0) else { continue };
                let (dmin, dmax, dtyp) = (hsd.min, hsd.max, hsd.typ);
                if !(dmin < imax && dmax > imin) {
                    continue;
                }
                if ityp == SegType::Bend && dtyp != SegType::Bend && dtyp != SegType::Ghost && (dmax - dmin) > (imax - imin) * 3.0 {
                    if let Some(s) = self.segments.get_mut(hsi_id.0) {
                        s.deleted = true;
                    }
                    break;
                } else if dtyp == SegType::Bend
                    && ityp != SegType::Bend
                    && ityp != SegType::Ghost
                    && (imax - imin) > (dmax - dmin) * 3.0
                    && let Some(s) = self.segments.get_mut(hsd_id.0)
                {
                    s.deleted = true;
                }
            }
        }
    }

    /// Drops deleted segments from every list (`deleteSegments`). *Shared*.
    pub fn delete_segments(&mut self) {
        for id in self.increasing_segs.iter().chain(&self.decreasing_segs) {
            if let Some(s) = self.segments.get_mut(id.0) {
                s.deleted = true;
            }
        }
        self.increasing_segs.clear();
        self.decreasing_segs.clear();
        self.cleanup();
    }

    /// Applies replacements and deletions to every element's lists (`cleanup`). *Shared*.
    pub fn cleanup(&mut self) {
        let segments = &self.segments;
        let live = |id: &SegId| segments.get(id.0).is_some_and(|s| !s.deleted);
        self.increasing_segs.retain(live);
        self.decreasing_segs.retain(live);
        for pes in self.pe_states.values_mut() {
            pes.cleanup(segments);
        }
    }

    /// Drops segments without a stem value from every element's lists (`pruneHintSegs`).
    /// *Shared*.
    pub fn prune_hint_segs(&mut self) {
        for pes in self.pe_states.values_mut() {
            pes.prune_hint_segs(&self.segments);
        }
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
    ///
    /// `seg` names the segment the score came from (its [`SegId`] index); a segment counts once.
    /// The original rounds `loc` here when coordinates are rounded; the caller does that, since
    /// this state does not hold the options.
    pub fn add_to_loc(&mut self, loc: f64, score: f64, strong: bool, bb: bool, seg: Option<usize>) {
        // Mixing bounding-box and segment data is only logged by the original.
        if self.bb.is_none() {
            self.bb = Some(bb);
        }
        if let Some(seg) = seg
            && !self.used_segs.insert(seg)
        {
            return;
        }
        match self.candidates.iter_mut().find(|c| c.loc == loc) {
            Some(c) => c.add_score(score, strong),
            None => {
                let mut c = StemLocCandidate { loc, ..StemLocCandidate::default() };
                c.add_score(score, strong);
                self.candidates.push(c);
            }
        }
    }

    /// The best location, or `None` when no candidate exists (`bestLocation`): the highest
    /// weight, doubled inside a zone; the higher location on a tie.
    pub fn best_location(&self, is_bottom: bool, in_band: &dyn Fn(f64, bool) -> bool) -> Option<f64> {
        let mut best: Option<(f64, f64)> = None;
        for c in &self.candidates {
            let w = c.weight(in_band(c.loc, is_bottom));
            let better = match best {
                None => true,
                Some((bw, bl)) => w > bw || (w == bw && c.loc > bl),
            };
            if better {
                best = Some((w, c.loc));
            }
        }
        best.map(|(_, loc)| loc)
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
    ///
    /// Reads [`GlyphHintState::stem_values`]; `is_v` only named the dimension in the original's
    /// log.
    pub fn mark(&mut self, hs: &GlyphHintState, is_v: bool) {
        let _ = is_v;
        if self.cnt == 0 {
            return;
        }
        for &sv in &hs.stem_values {
            let Some(v) = hs.value(sv) else { continue };
            let (Some(lpe), Some(upe)) = (hs.seg(v.lseg).and_then(|s| s.pe), hs.seg(v.useg).and_then(|s| s.pe)) else { continue };
            let (l, u) = (lpe.subpath, upe.subpath);
            if l == u {
                continue;
            }
            for (a, b) in [(l, u), (u, l)] {
                if let Some(cell) = self.links.get_mut(a).and_then(|row| row.get_mut(b)) {
                    *cell = 1;
                }
            }
        }
    }

    /// A subpath order that reduces substitution, or `None` to keep the order (`shuffle`).
    /// *Shared* (M10.12).
    ///
    /// The first subpath is the one with the most links (the lowest index on a tie); each next
    /// one is the remaining subpath with the most links to those already placed, then the most
    /// links overall, then the lowest index. A subpath with no link to those placed starts a new
    /// run.
    pub fn shuffle(&self) -> Option<Vec<usize>> {
        if self.cnt == 0 {
            return None;
        }
        let n = self.cnt;
        let link = |a: usize, b: usize| u32::from(self.links.get(a).and_then(|r| r.get(b)).copied().unwrap_or(0));
        let sumlinks: Vec<u32> = (0..n).map(|s| (0..n).map(|j| link(j, s)).sum()).collect();
        let sum = |s: usize| sumlinks.get(s).copied().unwrap_or(0);
        let mut outlinks = vec![0u32; n];
        let mut subidxs: Vec<usize> = (0..n).collect();
        let mut suborder = Vec::with_capacity(n);
        let mut move_idx = |idx: usize, subidxs: &mut Vec<usize>, outlinks: &mut Vec<u32>| {
            subidxs.retain(|&s| s != idx);
            suborder.push(idx);
            for (i, o) in outlinks.iter_mut().enumerate() {
                *o += link(idx, i);
            }
        };
        while let Some(bst) = subidxs.iter().copied().max_by_key(|&s| (sum(s), std::cmp::Reverse(s))) {
            move_idx(bst, &mut subidxs, &mut outlinks);
            loop {
                let out = |s: usize| outlinks.get(s).copied().unwrap_or(0);
                let next = subidxs.iter().copied().filter(|&s| out(s) > 0).max_by_key(|&s| (out(s), sum(s), std::cmp::Reverse(s)));
                let Some(next) = next else { break };
                move_idx(next, &mut subidxs, &mut outlinks);
            }
        }
        Some(suborder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values: `crates/autohint/tests/oracle/masks_state.py`, which runs otfautohint's
    // own functions on the same input.

    fn pe(i: usize) -> PeRef {
        PeRef::new(0, i)
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn segments_at_one_location_merge_and_short_bends_go() {
        let mut hs = GlyphHintState::default();
        let calls: [(f64, f64, f64, Option<usize>, Option<usize>, SegType, f64, bool); 7] = [
            (0.0, 100.0, 50.0, None, Some(0), SegType::Line, 0.0, false),
            (20.0, 60.0, 50.0, Some(1), None, SegType::Curve, 5.0, false),
            (200.0, 300.0, 50.000001, Some(2), None, SegType::Bend, 0.0, false),
            (90.0, 400.0, 50.0, None, Some(3), SegType::Line, 2.0, false),
            (300.0, 0.0, 50.0, Some(4), None, SegType::Line, 0.0, false),
            (10.0, 30.0, 120.0, Some(5), None, SegType::Bend, 0.0, false),
            (100.0, 0.0, 120.0, Some(0), Some(1), SegType::Line, 0.0, true),
        ];
        for (i, (fr, to, loc, pe1, pe2, typ, bonus, mid1)) in calls.into_iter().enumerate() {
            let id = hs.add_segment(fr, to, loc, pe1.map(pe), pe2.map(pe), typ, bonus, false, mid1, false, "");
            assert_eq!(id, Some(SegId(i)));
        }
        let ids = |v: &[SegId]| v.iter().map(|s| s.0).collect::<Vec<_>>();
        assert_eq!(ids(&hs.increasing_segs), [0, 1, 3, 2, 5]);

        hs.compact_lists();
        hs.rem_extra_bends();
        hs.cleanup();

        let expected: [(f64, f64, f64, Option<usize>, bool); 7] = [
            (0.0, 100.0, 5.0, Some(3), false),
            (20.0, 60.0, 5.0, Some(0), false),
            (200.0, 300.0, 0.0, Some(3), false),
            (0.0, 400.0, 5.0, None, false),
            (0.0, 300.0, 0.0, None, false),
            (10.0, 30.0, 0.0, None, true),
            (0.0, 100.0, 0.0, None, false),
        ];
        for (s, (min, max, bonus, replaced, deleted)) in hs.segments.iter().zip(expected) {
            assert_eq!((s.min, s.max, s.bonus, s.replaced_by.map(|r| r.0), s.deleted), (min, max, bonus, replaced, deleted));
        }
        assert_eq!(ids(&hs.increasing_segs), [3]);
        assert_eq!(ids(&hs.decreasing_segs), [4, 6]);
        let lists = |i: usize| hs.pe_states.get(&pe(i)).map(|p| [ids(&p.s_segs), ids(&p.m_segs), ids(&p.e_segs)]);
        let expected_lists: [[&[usize]; 3]; 6] =
            [[&[3], &[6], &[]], [&[6], &[], &[3]], [&[], &[], &[3]], [&[3], &[], &[]], [&[], &[], &[4]], [&[], &[], &[]]];
        for (i, want) in expected_lists.into_iter().enumerate() {
            let got = lists(i).unwrap();
            assert_eq!([got[0].as_slice(), got[1].as_slice(), got[2].as_slice()], want, "element {i}");
        }
        assert_eq!(hs.current(SegId(1)), SegId(3));
    }

    #[test]
    fn a_replacement_cycle_does_not_hang() {
        let mut hs = GlyphHintState::default();
        let a = hs.add_segment(0.0, 10.0, 5.0, Some(pe(0)), None, SegType::Line, 0.0, false, false, false, "").unwrap();
        let b = hs.add_segment(0.0, 10.0, 5.0, Some(pe(1)), None, SegType::Line, 0.0, false, false, false, "").unwrap();
        hs.seg_mut(a).unwrap().replaced_by = Some(b);
        hs.seg_mut(b).unwrap().replaced_by = Some(a);
        let _ = hs.current(a);
        hs.cleanup();
    }

    #[test]
    fn subpaths_linked_by_stems_are_ordered_together() {
        let mut hs = GlyphHintState::default();
        let segs: Vec<SegId> = (0..5)
            .map(|i| {
                hs.push_segment(HintSegment {
                    loc: 0.0,
                    min: 0.0,
                    max: 10.0,
                    bonus: 0.0,
                    typ: SegType::Line,
                    is_v: false,
                    is_inc: true,
                    desc: String::new(),
                    pe: Some(PeRef::new(i, 0)),
                    hint_val: None,
                    replaced_by: None,
                    deleted: false,
                    suppressed: false,
                })
            })
            .collect();
        for (a, b) in [(0, 3), (3, 4), (1, 2), (0, 0), (4, 0)] {
            let v = hs.push_value(StemValue {
                val: 1.0,
                spc: 0.0,
                lloc: 0.0,
                uloc: 10.0,
                is_ghost: false,
                pruned: false,
                merge: false,
                lseg: segs[a],
                useg: segs[b],
                best: None,
                initial_val: 1.0,
                idx: None,
            });
            hs.stem_values.push(v);
        }
        let mut links = Links::new(5);
        links.mark(&hs, false);
        assert_eq!(links.links, [[0, 0, 0, 1, 1], [0, 0, 1, 0, 0], [0, 1, 0, 0, 0], [1, 0, 0, 0, 1], [1, 0, 0, 1, 0]]);
        assert_eq!(links.shuffle(), Some(vec![0, 3, 4, 1, 2]));
        assert_eq!(Links::new(3).shuffle(), None);
    }

    #[test]
    fn the_heaviest_location_wins_with_zones_doubling_the_weight() {
        let in_band = |loc: f64, is_bottom: bool| if is_bottom { (-15.0..=0.0).contains(&loc) } else { (500.0..=515.0).contains(&loc) };
        let mut iss = InstanceStemState::new(0.0);
        assert_eq!(iss.best_location(true, &in_band), None);
        iss.add_to_loc(10.0, 30.0, true, false, Some(0));
        iss.add_to_loc(10.0, 5.0, false, false, None);
        iss.add_to_loc(0.0, 20.5, false, false, None);
        iss.add_to_loc(10.0, 30.0, true, false, Some(0));
        iss.add_to_loc(505.0, 12.0, false, false, None);
        assert_eq!(iss.candidates.len(), 3);
        assert_eq!(iss.best_location(true, &in_band), Some(10.0));
        assert_eq!(iss.best_location(false, &in_band), Some(10.0));
    }

    #[test]
    fn ghost_values_compare_by_their_charstring_edges() {
        let mut segments = vec![];
        for typ in [SegType::Ghost, SegType::Line] {
            segments.push(HintSegment {
                loc: 0.0,
                min: 0.0,
                max: 0.0,
                bonus: 0.0,
                typ,
                is_v: false,
                is_inc: true,
                desc: String::new(),
                pe: None,
                hint_val: None,
                replaced_by: None,
                deleted: false,
                suppressed: false,
            });
        }
        let value = |lseg: usize, useg: usize, is_ghost: bool| StemValue {
            val: 1.0,
            spc: 0.0,
            lloc: 700.0,
            uloc: 700.0,
            is_ghost,
            pruned: false,
            merge: false,
            lseg: SegId(lseg),
            useg: SegId(useg),
            best: None,
            initial_val: 1.0,
            idx: None,
        };
        assert_eq!(value(0, 1, true).ghosted(&segments), (700.0, 680.0));
        assert_eq!(value(1, 0, true).ghosted(&segments), (721.0, 700.0));
        assert_eq!(value(1, 0, false).ghosted(&segments), (700.0, 700.0));
    }
}
