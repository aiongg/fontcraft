//! The glyph driver: hints both dimensions, removes flares, converts stems and distributes the
//! hint masks and counter masks over the path.
//!
//! Ported from otfautohint's `hinter.py` lines 2420–2992 (`glyphHinter`) and the per-glyph part
//! of `autohint.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`).
//!
//! Python passed hint masks around as `[hmask, vmask]` lists and changed them in place, so one
//! list could be the glyph's start mask and the working mask at once. Here a mask is a
//! [`HintMask`] value, and [`GlyphHinter::distribute_masks`] keeps track of which element (or
//! the start of the charstring) owns the mask it is building, writing it back when it moves on.

use std::sync::Arc;

use fontcraft_geom::BezPath;

use crate::fddict::FdDict;
use crate::glyph::{Dim, FlexPart, GlyphData, PeRef, Stem};
use crate::hinter::{BAND_MARGIN, DimState, FLARE_VALUE_LIMIT, GlyphHinter, Glyphs, MAX_FLARE, MAX_HALF_MARGIN, SPC_BONUS};
use crate::state::{GlyphHintState, Links, OverlapState, SegId};
use crate::{AutohintError, GlyphHints, HintInput, HintMask, Options};

/// Hints one glyph. An empty glyph gets empty hints; a direction with more segments than the
/// limit is left unhinted with a warning. The zones and stems were checked when the
/// dictionary was made ([`FdDict::new`]).
pub fn hint_glyph(input: &HintInput) -> Result<GlyphHints, AutohintError> {
    if input.contours.is_empty() {
        return Ok(GlyphHints::default());
    }
    let glyph = GlyphData::from_contours(input.name, input.contours, input.options.round_coords)?;
    if glyph.is_empty() {
        return Ok(GlyphHints::default());
    }
    let mut warnings = Vec::new();
    let overlap = overlap_state(&glyph, input, false, &mut warnings);
    let mut hinter = GlyphHinter::new(Arc::new(input.options.clone()));
    hinter.glyphs = Glyphs { main: glyph, ..Default::default() };
    hinter.hint_with_overlap(input.name, vec![Arc::new(input.dict.clone())], overlap)?;
    let mut hints = hinter.result(0).unwrap_or_default();
    warnings.append(&mut hints.warnings);
    hints.warnings = warnings;
    Ok(hints)
}

/// Whether the hinter works on the overlap-free outline of a glyph, as
/// `DimHinter::handle_overlap` decides once the glyph is set.
pub(crate) fn overlap_wanted(options: &Options, name: &str, multi: bool) -> bool {
    match options.overlap_forcing {
        Some(force) => force,
        None => options.overlap_list.contains(name) || multi,
    }
}

/// The overlap-free outline of `glyph`, associated with its elements, when overlaps are handled
/// for this glyph. An overlap-free outline that cannot be read is reported and left out.
pub(crate) fn overlap_state(glyph: &GlyphData, input: &HintInput, multi: bool, warnings: &mut Vec<String>) -> OverlapState {
    if !overlap_wanted(input.options, input.name, multi) {
        return OverlapState::Unchecked;
    }
    let Some(contours) = input.without_overlap else {
        return OverlapState::NoOverlap;
    };
    match overlap_from(glyph, contours, input.options.loose_overlap_mapping) {
        Ok(Some(removed)) => OverlapState::Removed(Box::new(removed)),
        Ok(None) => OverlapState::NoOverlap,
        Err(e) => {
            warnings.push(format!("{}: hinted with its overlaps, because the outline without them could not be used: {e}", input.name));
            OverlapState::NoOverlap
        }
    }
}

fn overlap_from(glyph: &GlyphData, contours: &[BezPath], loose: bool) -> Result<Option<GlyphData>, AutohintError> {
    crate::overlap::associate(glyph, contours, loose)
}

/// Where the mask being built in [`GlyphHinter::distribute_masks`] is stored: the start of the
/// charstring or the element before which it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Start,
    Pe(PeRef),
}

/// `distributeMasks`' modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    NotShort,
    Short,
    Conflict,
}

/// The default-master stems of one dimension (`hs.stems[0]`), empty before conversion.
fn stems0(hs: &GlyphHintState) -> &[Stem] {
    hs.stems.as_ref().and_then(|s| s.first()).map(Vec::as_slice).unwrap_or(&[])
}

/// Element-wise `or` of two masks, as long as the shorter one (Python's `zip`).
fn or_masks(a: &[bool], b: &[bool]) -> Vec<bool> {
    a.iter().zip(b).map(|(x, y)| *x || *y).collect()
}

/// Keeps the entries of `l` whose flag in `keep` is set (`delUnused`, one dimension).
fn del_unused<T>(l: &mut Vec<T>, keep: &[bool]) {
    let mut i = 0;
    l.retain(|_| {
        let k = keep.get(i).copied().unwrap_or(true);
        i += 1;
        k
    });
}

fn del_unused_mask(m: &mut HintMask, used: &HintMask) {
    del_unused(&mut m.h, &used.h);
    del_unused(&mut m.v, &used.v);
}

/// `isUSeg`: `loc` is nearer the upper edge of the stem than the lower.
fn is_u_seg(loc: f64, uloc: f64, lloc: f64) -> bool {
    (uloc - loc).abs() <= (lloc - loc).abs()
}

/// The first element in hint order (`nextForHints(glyph)`): a subpath's closing element comes
/// before its first one.
fn first_for_hints(glyph: &GlyphData) -> Option<PeRef> {
    let sp = glyph.subpaths.first()?;
    let last = sp.len().checked_sub(1)?;
    if sp.get(last).is_some_and(|pe| pe.is_close) { Some(PeRef::new(0, last)) } else { Some(PeRef::new(0, 0)) }
}

/// The last element of the path (`last`).
fn last_element(glyph: &GlyphData) -> Option<PeRef> {
    let si = glyph.subpaths.len().checked_sub(1)?;
    let off = glyph.subpaths.get(si)?.len().checked_sub(1)?;
    Some(PeRef::new(si, off))
}

/// `new_order` lists every index below `len` once.
fn is_permutation(new_order: &[usize], len: usize) -> bool {
    if new_order.len() != len {
        return false;
    }
    let mut seen = vec![false; len];
    for &i in new_order {
        match seen.get_mut(i) {
            Some(s) if !*s => *s = true,
            _ => return false,
        }
    }
    true
}

impl GlyphHinter {
    /// Hints the glyphs in [`GlyphHinter::glyphs`] (one per master) against their dictionaries
    /// (`_hint`). Returns false when the glyph keeps its own hints in both dimensions or the
    /// masters are not compatible.
    pub fn hint(&mut self, name: &str, fddicts: Vec<Arc<FdDict>>) -> Result<bool, AutohintError> {
        self.hint_with_overlap(name, fddicts, OverlapState::Unchecked)
    }

    /// [`GlyphHinter::hint`], with the main master's overlap-free outline already associated
    /// (`OverlapState::Unchecked` leaves overlap handling to segment generation). The other
    /// masters' outlines are those already in [`Glyphs::without_overlap`].
    pub fn hint_with_overlap(&mut self, name: &str, fddicts: Vec<Arc<FdDict>>, overlap: OverlapState) -> Result<bool, AutohintError> {
        let masters = self.glyphs.len();
        let report = Some(crate::report::GlyphReport::new(name, self.options.report_all_stems));
        self.h.set_glyph(fddicts.clone(), report.clone(), masters, name);
        self.v.set_glyph(fddicts.clone(), report, masters, name);
        self.glyphs.set_without_overlap(0, overlap);

        if !self.compatible_paths(&fddicts) {
            return Ok(false);
        }

        self.glyphs.main.changed = false;

        if !self.options.no_flex {
            self.dim(Dim::H).add_flex(true, false);
            self.dim(Dim::V).add_flex(true, true);
        }

        let mut lnks = Links::new(self.glyphs.main.subpaths.len());

        self.dim(Dim::H).calc_hint_values(&mut lnks, true, true);
        self.dim(Dim::V).calc_hint_values(&mut lnks, true, true);

        if self.options.just_reporting() {
            return Ok(true);
        }

        if self.h.keep_hints && self.v.keep_hints {
            return Ok(false);
        }

        if self.options.allow_changes
            && let Some(new_order) = lnks.shuffle()
            && is_permutation(&new_order, self.glyphs.main.subpaths.len())
        {
            for gi in 0..self.glyphs.len() {
                if let Some(g) = self.glyphs.get_mut(gi) {
                    g.reorder(&new_order);
                }
            }
            self.remap_subpaths(&new_order);
        }

        self.do_v = false;
        self.list_hint_info();

        if !self.h.keep_hints {
            self.rem_flares();
        }

        self.do_v = true;
        if !self.v.keep_hints {
            self.rem_flares();
        }

        self.dim(Dim::H).convert_to_stem_lists();
        self.dim(Dim::V).convert_to_stem_lists();

        self.other_instance_stems();

        self.dim(Dim::H).convert_to_masks();
        self.dim(Dim::V).convert_to_masks();

        let used = self.distribute_masks();
        if let Some(used) = &used {
            self.cleanup_unused(used);
        }

        self.other_instance_masks();

        for gi in 0..self.glyphs.len() {
            if let Some(g) = self.glyphs.get_mut(gi) {
                g.clear_temp_state();
            }
        }
        Ok(true)
    }

    /// Python kept the hint state keyed by element objects, which survive a reordering of the
    /// subpaths; here elements are named by subpath index, so the keys move with the subpaths.
    /// Subpath `new_order[j]` has become subpath `j`.
    pub(crate) fn remap_subpaths(&mut self, new_order: &[usize]) {
        let mut new_index = vec![0; new_order.len()];
        for (j, &old) in new_order.iter().enumerate() {
            if let Some(slot) = new_index.get_mut(old) {
                *slot = j;
            }
        }
        let map = |r: PeRef| PeRef { subpath: new_index.get(r.subpath).copied().unwrap_or(r.subpath), ..r };
        for st in [&mut self.h, &mut self.v] {
            let states = std::mem::take(&mut st.hs.pe_states);
            st.hs.pe_states = states.into_iter().map(|(k, v)| (map(k), v)).collect();
            for seg in &mut st.hs.segments {
                seg.pe = seg.pe.map(map);
            }
        }
    }

    /// The state of the dimension [`GlyphHinter::do_v`] selects, to change.
    fn current_mut(&mut self) -> &mut DimState {
        if self.do_v { &mut self.v } else { &mut self.h }
    }

    /// The segments of element `pe` in the current dimension, or the other when `oppo`
    /// (`getSegments`).
    pub fn get_segments(&self, pe: PeRef, oppo: bool) -> Vec<SegId> {
        let st = if self.do_v != oppo { &self.v } else { &self.h };
        st.hs.pe_states.get(&pe).map(|p| p.s_segs.iter().chain(&p.m_segs).chain(&p.e_segs).copied().collect()).unwrap_or_default()
    }

    /// The masks of element `pe` in both dimensions (`getMasks`).
    pub fn get_masks(&self, pe: PeRef) -> HintMask {
        let one = |hs: &GlyphHintState, vert: bool| -> Vec<bool> {
            let mask = if hs.keep_hints == Some(true) {
                self.glyphs
                    .main
                    .element(pe)
                    .and_then(|e| e.masks.as_ref())
                    .map(|m| if vert { m.v.clone() } else { m.h.clone() })
                    .filter(|m| !m.is_empty())
            } else if hs.counter_hinted {
                Some(hs.main_mask.clone())
            } else {
                hs.pe_states.get(&pe).map(|p| p.mask.clone()).filter(|m| !m.is_empty())
            };
            mask.unwrap_or_else(|| vec![false; stems0(hs).len()])
        };
        HintMask { h: one(&self.h.hs, false), v: one(&self.v.hs, true) }
    }

    fn store_mask(&mut self, slot: Slot, mask: HintMask) {
        match slot {
            Slot::Start => self.glyphs.main.start_masks = Some(mask),
            Slot::Pe(pe) => {
                if let Some(e) = self.glyphs.main.element_mut(pe) {
                    e.masks = Some(mask);
                }
            }
        }
    }

    /// Chooses where hint masks change along the path and what each holds (`distributeMasks`).
    /// Returns which stems any mask uses, or `None` when every stem is used.
    pub fn distribute_masks(&mut self) -> Option<HintMask> {
        let hstems = stems0(&self.h.hs).to_vec();
        let vstems = stems0(&self.v.hs).to_vec();
        let (lh, lv) = (hstems.len(), vstems.len());
        self.glyphs.main.hstems = hstems;
        self.glyphs.main.vstems = vstems;

        let start = self.glyphs.main.start_masks.clone();
        // The original tests the horizontal overlaps for both dimensions here.
        let h_overlaps = self.h.hs.has_overlaps;
        let kept = |keep: bool, existing: Option<&Vec<bool>>, len: usize| -> Vec<bool> {
            if !keep {
                return vec![false; len];
            }
            match existing.filter(|m| !m.is_empty()) {
                Some(m) => m.clone(),
                None if !h_overlaps => vec![true; len],
                None => Vec::new(),
            }
        };
        let mut cur =
            HintMask { h: kept(self.h.keep_hints, start.as_ref().map(|m| &m.h), lh), v: kept(self.v.keep_hints, start.as_ref().map(|m| &m.v), lv) };

        self.build_counter_masks();

        if !self.h.hs.has_overlaps && !self.v.hs.has_overlaps {
            if self.h.hs.good_mask.contains(&false) || self.v.hs.good_mask.contains(&false) {
                return Some(HintMask { h: self.h.hs.good_mask.clone(), v: self.v.hs.good_mask.clone() });
            }
            self.glyphs.main.start_masks = None;
            self.glyphs.main.is_hm = Some(false);
            return None;
        }

        let mut used = cur.clone();
        if self.h.hs.counter_hinted {
            used.h = or_masks(&self.h.hs.main_mask, &used.h);
        }
        if self.v.hs.counter_hinted {
            used.v = or_masks(&self.v.hs.main_mask, &used.v);
        }

        self.glyphs.main.is_hm = Some(true);
        // `cur` is the mask owned by `slot`; in short mode `work` is the copy being extended
        // (the original's `masks`, with `cur` as its `oldmasks`).
        let mut slot = Slot::Start;
        let mut work = HintMask::default();
        let mut mode = Mode::NotShort;
        let mut ns: Option<PeRef> = None;
        let mut incompat = HintMask::default();

        let total: usize = self.glyphs.main.subpaths.iter().map(Vec::len).sum();
        let mut c = first_for_hints(&self.glyphs.main);
        let mut steps = 0;
        while let Some(cc) = c {
            steps += 1;
            if steps > total + 1 {
                break;
            }
            let Some(pe) = self.glyphs.main.element(cc) else { break };
            if pe.is_short() || pe.flex == Some(FlexPart::Second) {
                if mode == Mode::NotShort {
                    if let Some(nsr) = ns {
                        mode = Mode::Short;
                        work = cur.clone();
                        incompat = self.get_masks(nsr);
                    } else {
                        mode = Mode::Conflict;
                    }
                }
            } else {
                ns = Some(cc);
                if mode == Mode::Short {
                    cur = std::mem::take(&mut work);
                    incompat = HintMask::default();
                }
                mode = Mode::NotShort;
            }
            let cmasks = self.get_masks(cc);
            let (cand, conflict) = self.join_masks(if mode == Mode::Short { &work } else { &cur }, &cmasks);
            if conflict {
                match mode {
                    Mode::NotShort => {
                        let mut n = cmasks;
                        self.bridge_masks(&mut cur, Some(&mut n), &mut used, cc);
                        self.store_mask(slot, std::mem::replace(&mut cur, n));
                        slot = Slot::Pe(cc);
                    }
                    Mode::Short => {
                        let nsr = ns.unwrap_or(cc);
                        let (mut newinc, _) = self.join_masks(&incompat, &cmasks);
                        self.bridge_masks(&mut cur, Some(&mut newinc), &mut used, nsr);
                        self.store_mask(slot, std::mem::replace(&mut cur, newinc));
                        slot = Slot::Pe(nsr);
                        work = HintMask::default();
                        mode = Mode::Conflict;
                    }
                    Mode::Conflict => cur = cand,
                }
            } else if mode == Mode::Short {
                work = cand;
                incompat = self.join_masks(&incompat, &cmasks).0;
            } else {
                cur = cand;
            }
            c = self.glyphs.main.next_for_hints(cc);
        }
        if mode == Mode::Short {
            cur = work;
        }
        if let Some(last) = last_element(&self.glyphs.main) {
            self.bridge_masks(&mut cur, None, &mut used, last);
        }
        self.store_mask(slot, cur);
        Some(used)
    }

    /// For a counter-hinted dimension, one counter mask with every stem of that dimension
    /// (`buildCounterMasks`): only stem3-style counters are made.
    pub fn build_counter_masks(&mut self) {
        let hk = self.h.hs.keep_hints == Some(true);
        let vk = self.v.hs.keep_hints == Some(true);
        let hcmsk = vec![self.h.hs.counter_hinted; stems0(&self.h.hs).len()];
        let vcmsk = vec![self.v.hs.counter_hinted; stems0(&self.v.hs).len()];
        let cntr = if (hk || vk) && !self.glyphs.main.cntr.is_empty() {
            self.glyphs
                .main
                .cntr
                .iter()
                .map(|cm| HintMask { h: if hk { cm.h.clone() } else { hcmsk.clone() }, v: if vk { cm.v.clone() } else { vcmsk.clone() } })
                .collect()
        } else if self.h.hs.counter_hinted || self.v.hs.counter_hinted {
            vec![HintMask { h: hcmsk, v: vcmsk }]
        } else {
            Vec::new()
        };
        self.glyphs.main.cntr = cntr;
    }

    /// Adds the stems of `cm` to `m` where they do not conflict (`joinMasks`): returns the
    /// joined mask and whether some stem of `cm` conflicted, so that a new mask must start.
    pub fn join_masks(&self, m: &HintMask, cm: &HintMask) -> (HintMask, bool) {
        let mut conflict = false;
        let mut nm = HintMask::default();
        for (hs, mv, cv, out) in [(&self.h.hs, &m.h, &cm.h, &mut nm.h), (&self.v.hs, &m.v, &cm.v, &mut nm.v)] {
            let l = mv.len();
            let mut n = mv.clone();
            if hs.keep_hints == Some(true) {
                conflict = cv.contains(&true);
                *out = n;
                continue;
            }
            for i in 0..l {
                let ci = cv.get(i).copied().unwrap_or(false);
                if !ci || n.get(i).copied().unwrap_or(false) {
                    continue;
                }
                let mut iconflict = false;
                let mut ireplaced = false;
                for j in 0..l {
                    if !hs.has_overlaps {
                        break;
                    }
                    if j == i {
                        continue;
                    }
                    let overlaps = hs.stem_overlaps.get(i).and_then(|r| r.get(j)).copied().unwrap_or(false);
                    if n.get(j).copied().unwrap_or(false) && overlaps {
                        // A ghost stem may be stood in for by a stem already on.
                        if let Some(Some(ghost)) = hs.ghost_compat.get(i) {
                            ireplaced = (0..l).any(|k| n.get(k).copied().unwrap_or(false) && ghost.get(k).copied().unwrap_or(false));
                        }
                        if !ireplaced {
                            iconflict = true;
                        }
                    }
                    if ireplaced {
                        break;
                    }
                }
                if !iconflict && !ireplaced {
                    if let Some(x) = n.get_mut(i) {
                        *x = true;
                    }
                } else if iconflict {
                    conflict = true;
                }
            }
            *out = n;
        }
        (nm, conflict)
    }

    /// When the mask changes before element `pe` (`bridgeMasks`): completes the outgoing mask
    /// `o` (the stem nearest `pe` when a dimension has none, then the main stems when the glyph
    /// is small), and carries into the incoming mask `n` the stems of `o` that lie within the
    /// band margin of `pe`'s end. `used` collects every stem any mask has held.
    pub fn bridge_masks(&self, o: &mut HintMask, n: Option<&mut HintMask>, used: &mut HintMask, pe: PeRef) {
        let glyph = &self.glyphs.main;
        let Some(el) = glyph.element(pe) else { return };
        let po = if el.is_line { el.e } else { el.cs };
        let mut carry = HintMask { h: vec![false; o.h.len()], v: vec![false; o.v.len()] };
        let dlimit = (BAND_MARGIN / 2.0).max(MAX_HALF_MARGIN);
        for (stems, good, om, cm, nloc, oloc) in [
            (&glyph.hstems, &self.h.hs.good_mask, &mut o.h, &mut carry.h, el.e.y, po.y),
            (&glyph.vstems, &self.v.hs.good_mask, &mut o.v, &mut carry.v, el.e.x, po.x),
        ] {
            for (i, on) in om.iter().enumerate() {
                if *on
                    && stems.get(i).is_some_and(|s| s.distance(nloc) < dlimit)
                    && let Some(c) = cm.get_mut(i)
                {
                    *c = true;
                }
            }
            if !om.contains(&true) {
                let mut best: Option<(f64, usize)> = None;
                for i in 0..om.len() {
                    if !good.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    let Some(s) = stems.get(i) else { continue };
                    let d = s.distance(oloc);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, i));
                    }
                }
                if let Some((_, ms)) = best
                    && let Some(x) = om.get_mut(ms)
                {
                    *x = true;
                }
            }
        }
        if self.merge_main() {
            let main = HintMask { h: self.h.hs.main_mask.clone(), v: self.v.hs.main_mask.clone() };
            *o = self.join_masks(o, &main).0;
        }
        used.h = or_masks(&o.h, &used.h);
        used.v = or_masks(&o.v, &used.v);
        if let Some(n) = n {
            *n = self.join_masks(n, &carry).0;
        }
    }

    /// The main stems join every outgoing mask in glyphs of at most 5 subpaths (`mergeMain`).
    pub fn merge_main(&self) -> bool {
        self.glyphs.main.subpaths.len() <= 5
    }

    /// Drops stems no mask uses, in every master (`cleanupUnused`, `delUnused`).
    pub fn cleanup_unused(&mut self, used: &HintMask) {
        if !used.h.contains(&false) && !used.v.contains(&false) {
            return;
        }
        for gi in 0..self.glyphs.len() {
            if let Some(g) = self.glyphs.get_mut(gi) {
                del_unused(&mut g.hstems, &used.h);
                del_unused(&mut g.vstems, &used.v);
            }
        }
        // The rest of the state is in the main master's masks at this point.
        let glyph = &mut self.glyphs.main;
        if let Some(sm) = &mut glyph.start_masks {
            del_unused_mask(sm, used);
        }
        for cm in &mut glyph.cntr {
            del_unused_mask(cm, used);
        }
        let mut found = false;
        for pe in glyph.subpaths.iter_mut().flatten() {
            if let Some(m) = &mut pe.masks {
                found = true;
                del_unused_mask(m, used);
            }
        }
        if !found {
            glyph.start_masks = None;
            glyph.is_hm = Some(false);
        }
    }

    /// `listHintInfo` only writes debug messages; nothing is logged at that level here.
    pub fn list_hint_info(&mut self) {}

    /// Removes flares in the current dimension (`remFlares`, `isFlare`, `isUSeg`): when two
    /// elements of a subpath within `MAX_FLARE` of each other, joined by elements that stay
    /// within it, want different stems, the weaker segment is suppressed.
    pub fn rem_flares(&mut self) {
        let elements: Vec<PeRef> = self.glyphs.main.elements().collect();
        for c in elements {
            let csl = self.get_segments(c, false);
            if csl.is_empty() {
                continue;
            }
            let cap = self.glyphs.main.subpaths.get(c.subpath).map_or(0, Vec::len) + 1;
            let Some(mut n) = self.glyphs.main.next_in_subpath(c, false, true, false) else { continue };
            let mut c_done = false;
            let mut steps = 0;
            while c != n && !c_done {
                steps += 1;
                if steps > cap {
                    break;
                }
                let nsl = self.get_segments(n, false);
                if nsl.is_empty() {
                    if self.get_segments(n, true).is_empty() {
                        break;
                    }
                    match self.glyphs.main.next_in_subpath(n, false, true, false) {
                        Some(x) => n = x,
                        None => break,
                    }
                    continue;
                }
                for &cs in &csl {
                    let Some(cseg) = self.current().hs.seg(cs).cloned() else { continue };
                    if cseg.deleted {
                        continue;
                    }
                    for &nsid in &nsl {
                        let Some(nseg) = self.current().hs.seg(nsid).cloned() else { continue };
                        if nseg.deleted {
                            continue;
                        }
                        let diff = (cseg.loc - nseg.loc).abs();
                        if diff > MAX_FLARE {
                            c_done = true;
                            continue;
                        }
                        if !self.is_flare(cseg.loc, c, n) {
                            c_done = true;
                            continue;
                        }
                        let hs = &self.current().hs;
                        let (Some(chv), Some(nhv)) =
                            (cseg.hint_val.and_then(|v| hs.value(v)).cloned(), nseg.hint_val.and_then(|v| hs.value(v)).cloned())
                        else {
                            continue;
                        };
                        if diff != 0.0 && is_u_seg(cseg.loc, chv.uloc, chv.lloc) == is_u_seg(nseg.loc, nhv.uloc, nhv.lloc) {
                            if chv.comp_val(SPC_BONUS, 1.0) > nhv.comp_val(SPC_BONUS, 1.0) {
                                if nhv.spc == 0.0
                                    && nhv.val < FLARE_VALUE_LIMIT
                                    && let Some(s) = self.current_mut().hs.seg_mut(nsid)
                                {
                                    s.suppressed = true;
                                }
                            } else if chv.spc == 0.0 && chv.val < FLARE_VALUE_LIMIT {
                                if let Some(s) = self.current_mut().hs.seg_mut(cs) {
                                    s.suppressed = true;
                                }
                                break;
                            }
                        }
                    }
                }
                match self.glyphs.main.next_in_subpath(n, false, true, false) {
                    Some(x) => n = x,
                    None => break,
                }
            }
        }
    }

    /// Every element from `c` up to `n` ends within `MAX_FLARE` of `loc` (`isFlare`).
    fn is_flare(&self, loc: f64, mut c: PeRef, n: PeRef) -> bool {
        let glyph = &self.glyphs.main;
        let cap = glyph.subpaths.get(c.subpath).map_or(0, Vec::len) + 1;
        let mut steps = 0;
        while c != n {
            steps += 1;
            if steps > cap {
                return false;
            }
            let Some(pe) = glyph.element(c) else { return false };
            let v = if self.do_v { pe.e.x } else { pe.e.y };
            if (v - loc).abs() > MAX_FLARE {
                return false;
            }
            match glyph.next_in_subpath(c, false, true, false) {
                Some(x) => c = x,
                None => return false,
            }
        }
        true
    }

    /// The result for master `glidx`.
    pub fn result(&self, glidx: usize) -> Option<GlyphHints> {
        let mut hints = self.glyphs.get(glidx).map(GlyphData::to_hints)?;
        hints.warnings.extend(self.h.warnings.iter().chain(&self.v.warnings).cloned());
        Some(hints)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;

    use fontcraft_geom::BezPath;

    use super::*;
    use crate::fddict::{StemSet, Zone, ZoneSet};
    use crate::state::{HintSegment, PeHintState, SegType};

    /// A counter-clockwise rectangle, as `tests/oracle/driver.py` draws it.
    pub(crate) fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((x0, y0));
        p.line_to((x1, y0));
        p.line_to((x1, y1));
        p.line_to((x0, y1));
        p.close_path();
        p
    }

    /// BlueValues [-15 0 500 515 700 715], StdHW 50, StdVW 80, BlueFuzz 1: the dictionary of
    /// `tests/oracle/driver.py`.
    pub(crate) fn test_dict() -> FdDict {
        let zone = |bottom: f64, top: f64, is_bottom: bool, name: &str| Zone { bottom, top, is_bottom, name: name.to_string() };
        let zones = ZoneSet {
            blue_values: vec![
                zone(-15.0, 0.0, true, "BaselineYCoord"),
                zone(500.0, 515.0, false, "CapHeight"),
                zone(700.0, 715.0, false, "LcHeight"),
            ],
            blue_fuzz: 1.0,
            ..ZoneSet::default()
        };
        let stems = StemSet { h: vec![50.0], v: vec![80.0] };
        FdDict::new("test", &zones, &stems, false).unwrap()
    }

    fn mask(h: &[bool], v: &[bool]) -> HintMask {
        HintMask { h: h.to_vec(), v: v.to_vec() }
    }

    #[test]
    fn hints_read_back_are_the_hints_written() {
        // `to_hints` and `set_hints` place a mask on the same element, the closing line of the
        // second bar included, so manual hints survive a pass through the glyph data.
        let contours = [rect(100.0, 0.0, 180.0, 300.0), rect(140.0, 400.0, 220.0, 700.0)];
        let dict = test_dict();
        let options = Options::default();
        let input = HintInput { name: "staggered", contours: &contours, without_overlap: None, dict: &dict, options: &options };
        let hints = hint_glyph(&input).unwrap();
        assert!(!hints.masks.is_empty());
        let mut g = GlyphData::from_contours("staggered", &contours, true).unwrap();
        g.set_hints(&hints);
        assert_eq!(g.to_hints(), hints);
    }

    #[test]
    fn staggered_stems_get_a_second_hint_mask() {
        // otfautohint 5.0.1 (tests/oracle/driver.py, "staggered"): the overlapping vertical
        // stems are split over the start mask and a mask on the second bar's closing element.
        let contours = [rect(100.0, 0.0, 180.0, 300.0), rect(140.0, 400.0, 220.0, 700.0)];
        let dict = test_dict();
        let options = Options::default();
        let input = HintInput { name: "staggered", contours: &contours, without_overlap: None, dict: &dict, options: &options };
        let hints = hint_glyph(&input).unwrap();
        assert_eq!(hints.h, vec![Stem::new(21.0, 0.0), Stem::new(700.0, 680.0)]);
        assert_eq!(hints.v, vec![Stem::new(100.0, 180.0), Stem::new(140.0, 220.0)]);
        assert_eq!(hints.start_mask, Some(mask(&[true, true], &[true, false])));
        assert_eq!(hints.masks, vec![(crate::ElementPos { subpath: 1, element: 3 }, mask(&[true, true], &[false, true]))]);
        assert!(hints.counters.is_empty());
        assert!(hints.flex.is_empty());
    }

    #[test]
    fn m_gets_a_vertical_counter_mask_and_no_substitution() {
        // otfautohint 5.0.1 (tests/oracle/driver.py, "m").
        let contours = [rect(50.0, 0.0, 130.0, 500.0), rect(250.0, 0.0, 330.0, 500.0), rect(450.0, 0.0, 530.0, 500.0)];
        let dict = test_dict();
        let options = Options::default();
        let input = HintInput { name: "m", contours: &contours, without_overlap: None, dict: &dict, options: &options };
        let hints = hint_glyph(&input).unwrap();
        assert_eq!(hints.h, vec![Stem::new(21.0, 0.0), Stem::new(500.0, 480.0)]);
        assert_eq!(hints.v, vec![Stem::new(50.0, 130.0), Stem::new(250.0, 330.0), Stem::new(450.0, 530.0)]);
        assert_eq!(hints.start_mask, None);
        assert!(hints.masks.is_empty());
        assert_eq!(hints.counters, vec![mask(&[false, false], &[true, true, true])]);
    }

    /// Three horizontal stems (0 overlaps 1, 2 is a ghost that 1 can stand for) and two vertical
    /// stems without overlaps.
    fn conflict_hinter() -> GlyphHinter {
        let mut gh = GlyphHinter::new(Arc::new(Options::default()));
        gh.h.hs.has_overlaps = true;
        gh.h.hs.stem_overlaps = vec![vec![false, true, false], vec![true, false, true], vec![false, true, false]];
        gh.h.hs.ghost_compat = vec![None, None, Some(vec![false, true, false])];
        gh.v.hs.ghost_compat = vec![None, None];
        gh
    }

    #[test]
    fn join_masks_reports_conflicts_and_lets_ghosts_be_replaced() {
        // Expected values from otfautohint's joinMasks on the same state.
        let gh = conflict_hinter();
        let (m, conflict) = gh.join_masks(&mask(&[true, false, false], &[false, true]), &mask(&[false, true, true], &[true, false]));
        assert_eq!((m, conflict), (mask(&[true, false, true], &[true, true]), true));
        let (m, conflict) = gh.join_masks(&mask(&[false, true, false], &[false, false]), &mask(&[false, false, true], &[false, false]));
        assert_eq!((m, conflict), (mask(&[false, true, false], &[false, false]), false));
    }

    #[test]
    fn unused_stems_leave_every_master_and_every_mask() {
        let mut gh = GlyphHinter::new(Arc::new(Options::default()));
        let g = |dx: f64| GlyphData {
            subpaths: vec![vec![crate::glyph::PathElement::line(crate::glyph::Pt::new(0.0, 0.0), crate::glyph::Pt::new(10.0, 0.0))]],
            hstems: vec![Stem::new(0.0, 50.0), Stem::new(100.0 + dx, 150.0 + dx)],
            vstems: vec![Stem::new(10.0, 90.0)],
            ..GlyphData::default()
        };
        gh.glyphs = Glyphs { main: g(0.0), others: vec![g(10.0)], ..Default::default() };
        gh.glyphs.main.start_masks = Some(mask(&[true, false], &[true]));
        gh.glyphs.main.cntr = vec![mask(&[true, true], &[false])];
        if let Some(pe) = gh.glyphs.main.element_mut(PeRef::new(0, 0)) {
            pe.masks = Some(mask(&[false, true], &[true]));
        }
        gh.cleanup_unused(&mask(&[true, false], &[true]));
        assert_eq!(gh.glyphs.main.hstems, vec![Stem::new(0.0, 50.0)]);
        assert_eq!(gh.glyphs.others[0].hstems, vec![Stem::new(0.0, 50.0)]);
        assert_eq!(gh.glyphs.main.start_masks, Some(mask(&[true], &[true])));
        assert_eq!(gh.glyphs.main.cntr, vec![mask(&[true], &[false])]);
        assert_eq!(gh.glyphs.main.subpaths[0][0].masks, Some(mask(&[false], &[true])));
    }

    #[test]
    fn reordering_subpaths_moves_the_hint_state_with_them() {
        let mut gh = GlyphHinter::new(Arc::new(Options::default()));
        let seg = HintSegment {
            loc: 0.0,
            min: 0.0,
            max: 10.0,
            bonus: 0.0,
            typ: SegType::Line,
            is_v: false,
            is_inc: true,
            desc: String::new(),
            pe: Some(PeRef::new(2, 1)),
            hint_val: None,
            replaced_by: None,
            deleted: false,
            suppressed: false,
        };
        let id = gh.h.hs.push_segment(seg);
        gh.h.hs.pe_states = BTreeMap::from([(PeRef::new(2, 1), PeHintState { s_segs: vec![id], ..PeHintState::default() })]);
        // Old subpath 2 becomes subpath 0.
        gh.remap_subpaths(&[2, 0, 1]);
        assert_eq!(gh.h.hs.segments[0].pe, Some(PeRef::new(0, 1)));
        assert_eq!(gh.h.hs.pe_states.keys().copied().collect::<Vec<_>>(), vec![PeRef::new(0, 1)]);
        assert!(!is_permutation(&[0, 0, 1], 3));
    }
}
