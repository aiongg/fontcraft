//! Stems in the other masters, conflicts between overlapping stems, and the hint masks of each
//! path element (hint substitution).
//!
//! Ported from otfautohint's `hinter.py` lines 1904–2277 (`calcInstanceStems` to `OKToRem`;
//! AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Rows:
//! `hinting.ps.export.hintReplacement`, `autohint.ps.hintSubstitution`,
//! `autohint.ps.variable.stemOverlap`.
//!
//! Where the original's behaviour comes from a slip in its code, the port keeps it, so that the
//! masks match otfautohint's glyph by glyph; each such place says so. Where the original would
//! stop with an exception, the port carries on with the nearest sensible value.

use crate::glyph::{PeRef, Pt, Stem, feq, fne};
use crate::hinter::{BIG_DIST_FACTOR, CONFLICT_VAL_MIN, CONFLICT_VAL_MULT, DimHinter, MAX_STEM_DIST, NO_SEG_SCORE, SPC_BONUS};
use crate::segments::diff_sign;
use crate::state::{GlyphHintState, InstanceStemState, SegId, SegPlace, StemId};

/// Calls of the conflict search ([`DimHinter::unconflict`]) before it stops branching. The
/// search tries both ways of resolving each conflict, which doubles with every conflict; past
/// this budget it keeps the first way only, which still resolves every conflict.
pub const MAX_UNCONFLICT_CALLS: usize = 4096;
/// Recursion depth of the conflict search before it stops branching.
const MAX_UNCONFLICT_DEPTH: usize = 256;

/// Which kind of ghost a stem is (`stem.isGhost`): a top ghost has a width of −20, a bottom
/// ghost −21.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ghost {
    High,
    Low,
}

fn ghost_kind(s: &Stem) -> Option<Ghost> {
    let width = s.rt - s.lb;
    if width == -20.0 {
        Some(Ghost::High)
    } else if width == -21.0 {
        Some(Ghost::Low)
    } else {
        None
    }
}

/// Python's tuple order on stems: `a > b` by lower edge, then upper edge.
fn stem_gt(a: &Stem, b: &Stem) -> bool {
    a.lb > b.lb || (a.lb == b.lb && a.rt > b.rt)
}

fn at(v: &[bool], i: usize) -> bool {
    v.get(i).copied().unwrap_or(false)
}

fn at2(m: &[Vec<bool>], i: usize, j: usize) -> bool {
    m.get(i).is_some_and(|r| at(r, j))
}

fn set(v: &mut [bool], i: usize, b: bool) {
    if let Some(x) = v.get_mut(i) {
        *x = b;
    }
}

fn set2(m: &mut [Vec<bool>], i: usize, j: usize, b: bool) {
    if let Some(r) = m.get_mut(i) {
        set(r, j, b);
    }
}

/// The segment `segments` holds for stem `idx` with the highest value (`max` over
/// `(compVal(SpcBonus), segment)`): on equal values the segment at the higher location, the
/// first one when the locations are equal too.
fn best_seg(hs: &GlyphHintState, segments: &[SegId], idx: usize) -> Option<(SegId, StemId)> {
    let mut best: Option<((f64, f64), f64, SegId, StemId)> = None;
    for &sid in segments {
        let Some(seg) = hs.seg(sid) else { continue };
        if seg.suppressed {
            continue;
        }
        let Some(vid) = seg.hint_val else { continue };
        let Some(v) = hs.value(vid) else { continue };
        if v.idx != Some(idx) {
            continue;
        }
        let key = v.comp_val(SPC_BONUS, 1.0);
        let better = match best {
            None => true,
            Some((bk, bloc, ..)) => {
                if key != bk {
                    key.0 > bk.0 || (key.0 == bk.0 && key.1 > bk.1)
                } else {
                    !feq(seg.loc, bloc) && bloc < seg.loc
                }
            }
        };
        if better {
            best = Some((key, seg.loc, sid, vid));
        }
    }
    best.map(|(_, _, sid, vid)| (sid, vid))
}

impl DimHinter<'_> {
    /// Makes master `glidx` the current glyph and its dictionary the current one, or puts the
    /// main master back: the original reassigns `self.glyph` and `self.fddict`.
    fn swap_master(&mut self, glidx: usize) {
        if let Some(other) = self.glyphs.others.get_mut(glidx.wrapping_sub(1)) {
            std::mem::swap(&mut self.glyphs.main, other);
        }
        if glidx < self.glyphs.without_overlap.len() {
            self.glyphs.without_overlap.swap(0, glidx);
        }
        if glidx < self.st.fddicts.len() {
            self.st.fddicts.swap(0, glidx);
        }
    }

    /// The stems of master `glidx` from the main master's (`calcInstanceStems`). *Shared*
    /// (M10.12).
    ///
    /// Segments are generated for master `glidx`; for each element, the segments that set a
    /// main-master stem's edge vote for a location in the other master, from the same element's
    /// segments there (stronger at the same place in the element), from the glyph's bounds for
    /// a bounding-box segment, or from the element's points when nothing else matches. The
    /// result is appended to `hs.stems[glidx]`, parallel to `hs.stems[0]`.
    ///
    /// As in the original, the zones and the widest-stem distance stay those of master `glidx`
    /// afterwards: the mask conversion that follows reads them.
    pub fn calc_instance_stems(&mut self, glidx: usize) {
        if glidx == 0 || glidx >= self.glyphs.len() {
            return;
        }
        let Some(dsl) = self.st.hs.stems.as_ref().and_then(|s| s.first()).cloned() else { return };
        let num_stems = dsl.len();
        if num_stems == 0 {
            return;
        }

        self.swap_master(glidx);
        let hs0 = std::mem::take(&mut self.st.hs);
        self.st.keep_hints = false;
        self.start_hint();
        let big = self.dominant_stems().into_iter().fold(MAX_STEM_DIST, f64::max);
        self.st.big_dist = big * BIG_DIST_FACTOR;
        self.gen_segs();
        let hsi = std::mem::replace(&mut self.st.hs, hs0);
        self.swap_master(glidx);

        let dim = self.dim();
        let is_v = self.is_v();
        let round = self.st.options.round_coords;
        let rnd = |v: f64| if round { v.round_ties_even() } else { v };
        let gbounds = self.glyphs.get_mut(glidx).and_then(|g| g.get_bounds(None));
        let mut iss: Vec<InstanceStemState> = dsl.iter().flat_map(|s| [InstanceStemState::new(s.lb), InstanceStemState::new(s.rt)]).collect();
        let mut warnings = Vec::new();

        for c in self.masters_elements(Some(glidx)) {
            let (Some(c0), Some(ci)) = (c.first(), c.get(1)) else { continue };
            let hs0 = &self.st.hs;
            let Some(pes0) = hs0.pe_states.get(&c0.pe) else { continue };
            let pesi = hsi.pe_states.get(&ci.pe);
            let mut done = vec![[false; 2]; num_stems];
            for (pos0, seg0l) in pes0.seg_lists(SegPlace::Start) {
                for &seg0_id in seg0l {
                    let Some(seg0) = hs0.seg(seg0_id) else { continue };
                    let Some(sidx) = seg0.hint_val.and_then(|v| hs0.value(v)).and_then(|v| v.idx) else { continue };
                    let ul = usize::from(seg0.is_inc == is_v);
                    let Some(flags) = done.get_mut(sidx) else { continue };
                    if flags.get(ul).copied().unwrap_or(true) {
                        continue;
                    }
                    let Some(state) = iss.get_mut(2 * sidx + ul) else { continue };
                    let mut found = false;
                    if seg0.is_bbox() {
                        if !seg0.is_gbbox() {
                            // The original cannot reach this case (it would stop with an error).
                            warnings.push(format!("no {} location for a subpath bounding-box segment of stem {sidx}", self.a_desc()));
                        } else if let Some(pbs) = &gbounds {
                            let [[x0, y0], [x1, y1]] = pbs.bounds;
                            let (mn, mx) = (Pt { x: x0, y: y0 }, Pt { x: x1, y: y1 });
                            let loc = if seg0.is_ubbox() { mx.o(dim) } else { mn.o(dim) };
                            state.add_to_loc(rnd(loc), mx.a(dim) - mn.a(dim), true, true, None);
                            found = true;
                        }
                    } else if let Some(pesi) = pesi {
                        for (posi, segil) in pesi.seg_lists(pos0) {
                            for &segi_id in segil {
                                let Some(segi) = hsi.seg(segi_id) else { continue };
                                if segi.is_inc != seg0.is_inc {
                                    continue;
                                }
                                state.add_to_loc(rnd(segi.loc), segi.max - segi.min, posi == pos0, false, Some(segi_id.0));
                                found = true;
                            }
                            if found && posi == pos0 {
                                break;
                            }
                        }
                    }
                    if !found && let Some(pe) = self.glyphs.get(glidx).and_then(|g| g.element(ci.pe)) {
                        let loc = match pos0 {
                            SegPlace::Start => pe.s.o(dim),
                            SegPlace::End => pe.e.o(dim),
                            SegPlace::Middle => (pe.s.o(dim) + pe.e.o(dim)) / 2.0,
                        };
                        state.add_to_loc(rnd(loc), NO_SEG_SCORE, false, false, None);
                    }
                    if let Some(f) = flags.get_mut(ul) {
                        *f = true;
                    }
                }
            }
        }

        // `lo` and `hi` carry over from one stem to the next, as in the original: a ghost only
        // recomputes the edge it has.
        let mut out = Vec::with_capacity(num_stems);
        let (mut lo, mut hi): (Option<f64>, Option<f64>) = (None, None);
        for (sidx, s0) in dsl.iter().enumerate() {
            let ghost = ghost_kind(s0);
            for ul in [0, 1] {
                let skip = if ul == 0 { ghost == Some(Ghost::High) } else { ghost == Some(Ghost::Low) };
                if skip {
                    continue;
                }
                let loc = match self.located(sidx, ul, &iss, &self.st.hs) {
                    Some(loc) => loc,
                    None => {
                        warnings.push(format!("no data for the {} location of stem {sidx}", if ul == 0 { "lower" } else { "upper" }));
                        if ul == 0 { s0.lb } else { s0.rt }
                    }
                };
                if ul == 0 {
                    lo = Some(loc);
                } else {
                    hi = Some(loc);
                }
            }
            match ghost {
                Some(Ghost::Low) => {
                    if let Some(l) = lo {
                        let l = l + 21.0;
                        lo = Some(l);
                        hi = Some(l - 21.0);
                    }
                }
                Some(Ghost::High) => {
                    if let Some(h) = hi {
                        lo = Some(h);
                        hi = Some(h - 20.0);
                    }
                }
                None => {
                    if let (Some(l), Some(h)) = (lo, hi)
                        && h < l
                    {
                        // A width other than −20 or −21 marks the stem bad, not a ghost.
                        lo = Some(h + 50.0);
                        warnings.push(format!("stem {sidx} ends before it starts in another master and is removed"));
                    }
                }
            }
            out.push(Stem::new(lo.unwrap_or(s0.lb), hi.unwrap_or(s0.rt)));
        }

        self.st.warnings.append(&mut warnings);
        if let Some(stems) = self.st.hs.stems.as_mut() {
            while stems.len() <= glidx {
                stems.push(Vec::new());
            }
            if let Some(sl) = stems.get_mut(glidx) {
                sl.extend(out);
            }
        }
    }

    /// The location chosen for edge `ul` of stem `sidx`, or `None` when no master data exists:
    /// the stem's own candidates, else those of a stem that shares the edge's segment.
    fn located(&self, sidx: usize, ul: usize, iss: &[InstanceStemState], hs0: &GlyphHintState) -> Option<f64> {
        let in_band = |loc: f64, is_bottom: bool| self.in_band(loc, is_bottom);
        if let Some(loc) = iss.get(2 * sidx + ul).and_then(|s| s.best_location(ul == 0, &in_band)) {
            return Some(loc);
        }
        for &svid in &hs0.stem_values {
            let Some(sv) = hs0.value(svid) else { continue };
            if sv.idx != Some(sidx) {
                continue;
            }
            let seg = if ul == 1 { sv.useg } else { sv.lseg };
            let Some(other) = hs0.seg(seg).and_then(|s| s.hint_val).and_then(|v| hs0.value(v)).and_then(|v| v.idx) else { continue };
            if other != sidx
                && let Some(loc) = iss.get(2 * other + ul).and_then(|s| s.best_location(ul == 0, &in_band))
            {
                return Some(loc);
            }
        }
        None
    }

    /// The best location of stem `sidx`'s lower (`ul` 0) or upper (`ul` 1) edge in another
    /// master (`bestLocation`). `iss` holds two states per stem: `iss[2 * sidx + ul]`. Without
    /// any data, the main master's edge.
    pub fn best_location(&self, sidx: usize, ul: usize, iss: &[InstanceStemState], hs0: &GlyphHintState) -> Option<f64> {
        self.located(sidx, ul, iss, hs0).or_else(|| {
            let s = hs0.stems.as_ref()?.first()?.get(sidx)?;
            Some(if ul == 0 { s.lb } else { s.rt })
        })
    }

    /// Resolves conflicts in a mask (`unconflict`): returns the stems kept on.
    ///
    /// `sc[i][j]` marks stems that conflict; `cur` is the set to start from (all on by default)
    /// and `pin` the stems already decided (none by default). Each conflict is resolved both
    /// ways, keeping the stem or dropping it, and the heavier result by
    /// [`crate::state::GlyphHintState::weights`] wins.
    pub fn unconflict(&self, sc: &[Vec<bool>], cur: Option<&[bool]>, pin: Option<&[bool]>) -> Vec<bool> {
        let l = sc.len();
        let cur = cur.map_or_else(|| vec![true; l], <[bool]>::to_vec);
        let pin = pin.map_or_else(|| vec![false; l], <[bool]>::to_vec);
        let mut budget = MAX_UNCONFLICT_CALLS;
        self.unconflict_rec(sc, cur, pin, &mut budget, 0).1
    }

    fn unconflict_rec(&self, sc: &[Vec<bool>], mut cur: Vec<bool>, mut pin: Vec<bool>, budget: &mut usize, depth: usize) -> (f64, Vec<bool>) {
        *budget = budget.saturating_sub(1);
        let l = sc.len();
        let mut do_idx = None;
        let mut do_set = Vec::new();
        for sidx in 0..l {
            if at(&pin, sidx) || !at(&cur, sidx) {
                continue;
            }
            // The original tests `cur[sidx]` here where `cur[sjdx]` was meant, so a stem
            // already switched off still counts as a conflict; kept so the masks match.
            let conflicts: Vec<usize> = (sidx..l).filter(|&sjdx| !at(&pin, sjdx) && at2(sc, sidx, sjdx)).collect();
            if conflicts.is_empty() {
                set(&mut pin, sidx, true);
            } else if do_idx.is_none() {
                do_idx = Some(sidx);
                do_set = conflicts;
            }
        }

        let Some(d) = do_idx else {
            // The original sums the weights under a test of its loop variable, left at the last
            // stem: every weight counts when the last stem is on, none otherwise. Kept.
            let last_on = l.checked_sub(1).is_some_and(|i| at(&cur, i));
            let weight = if last_on { (0..l).map(|x| self.st.hs.weights.get(x).copied().unwrap_or(0.0)).sum() } else { 0.0 };
            return (weight, cur);
        };

        set(&mut pin, d, true);
        for &s in &do_set {
            set(&mut cur, s, false);
        }
        let r1 = self.unconflict_rec(sc, cur.clone(), pin.clone(), budget, depth + 1);
        if *budget == 0 || depth >= MAX_UNCONFLICT_DEPTH {
            return r1;
        }
        set(&mut pin, d, false);
        set(&mut cur, d, false);
        for &s in &do_set {
            set(&mut cur, s, true);
        }
        let r2 = self.unconflict_rec(sc, cur, pin, budget, depth + 1);
        if r1.0 > r2.0 { r1 } else { r2 }
    }

    /// Builds the overlap and ghost tables and every element's mask (`convertToMasks`).
    /// *Shared* (M10.12).
    ///
    /// Stems that are bad in some master, or out of order against another stem in some master,
    /// are switched off in `good_mask` (when conflicts are removed). Two good stems overlap when
    /// they come within the band margin of each other, or change order, in any master.
    /// `main_mask` holds the main values that do not overlap one another.
    pub fn convert_to_masks(&mut self) {
        if self.st.keep_hints {
            return;
        }
        let Some(stems) = self.st.hs.stems.clone() else { return };
        let Some(dsl) = stems.first() else { return };
        let l = dsl.len();
        if l > crate::MAX_STEMS {
            self.st.warnings.push(format!("more than {} {} stems: no hint masks", crate::MAX_STEMS, self.a_desc()));
            return;
        }

        let mut good = vec![true; l];
        let mut has_conflicts = false;
        let mut sc = vec![vec![false; l]; l];
        if self.st.options.remove_conflicts {
            for sl in &stems {
                for sidx in 0..l {
                    let Some(a) = sl.get(sidx) else { continue };
                    if a.is_bad() {
                        set(&mut good, sidx, false);
                        continue;
                    }
                    for sjdx in sidx + 1..l {
                        if sl.get(sjdx).is_some_and(|b| stem_gt(a, b)) {
                            has_conflicts = true;
                            set2(&mut sc, sidx, sjdx, true);
                            set2(&mut sc, sjdx, sidx, true);
                        }
                    }
                }
            }
        }
        if has_conflicts {
            let pin: Vec<bool> = good.iter().map(|g| !g).collect();
            good = self.unconflict(&sc, Some(&good), Some(&pin));
        }

        let mut has_overlaps = false;
        let mut so = vec![vec![false; l]; l];
        for sidx in 0..l {
            if !at(&good, sidx) {
                continue;
            }
            for sjdx in sidx + 1..l {
                if !at(&good, sjdx) {
                    continue;
                }
                for sl in &stems {
                    if let (Some(a), Some(b)) = (sl.get(sidx), sl.get(sjdx))
                        && (a.overlaps(b) || stem_gt(a, b))
                    {
                        has_overlaps = true;
                        set2(&mut so, sidx, sjdx, true);
                        set2(&mut so, sjdx, sidx, true);
                    }
                }
            }
        }
        if self.st.hs.counter_hinted && has_overlaps && !self.st.is_multi {
            self.st.warnings.push(format!("overlapping {} counter hints", self.a_desc()));
        }

        // A ghost stem in the main master is a ghost in every master.
        let mut gc: Vec<Option<Vec<bool>>> = vec![None; l];
        for (sidx, s) in dsl.iter().enumerate() {
            if !at(&good, sidx) || ghost_kind(s).is_none() {
                continue;
            }
            let mut row = vec![false; l];
            for sjdx in 0..l {
                if sidx == sjdx || !at(&good, sjdx) {
                    continue;
                }
                let compat = stems.iter().all(|sl| match (sl.get(sidx), sl.get(sjdx)) {
                    (Some(a), Some(b)) => a.ghost_compat(b),
                    _ => false,
                });
                if compat {
                    set(&mut row, sjdx, true);
                }
            }
            if let Some(g) = gc.get_mut(sidx) {
                *g = Some(row);
            }
        }

        // The main values may overlap after conversion; keep them in order, each only if it
        // overlaps none already kept.
        let mut mm = vec![false; l];
        let mut kept: Vec<usize> = Vec::new();
        for &c in &self.st.hs.main_values {
            let Some(ci) = self.st.hs.value(c).and_then(|v| v.idx) else { continue };
            if kept.iter().all(|&k| !at2(&so, ci, k)) && at(&good, ci) {
                set(&mut mm, ci, true);
                kept.push(ci);
            }
        }

        let hs = &mut self.st.hs;
        hs.good_mask = good;
        hs.has_overlaps = has_overlaps;
        hs.stem_overlaps = so;
        hs.ghost_compat = gc;
        hs.main_mask = mm;

        let elements: Vec<PeRef> = self.glyph().elements().collect();
        for pe in elements {
            if self.st.hs.pe_states.contains_key(&pe) {
                self.make_pe_mask(pe);
            }
        }
    }

    /// The mask of element `c` (`makePEMask`): the good stems its segments ask for. Where two of
    /// them overlap, one is dropped by the original's rules (a weak stem outside the zones, a
    /// line, the direction of the neighbouring elements, which edge the element ends on), or
    /// both when none applies. An element that asks for no stem gets an empty mask.
    pub fn make_pe_mask(&mut self, c: PeRef) {
        let dim = self.dim();
        let hs = &self.st.hs;
        let l = hs.stems.as_ref().and_then(|s| s.first()).map_or(0, Vec::len);
        let Some(pestate) = hs.pe_states.get(&c) else { return };
        let segments = pestate.segments();
        let mut mask = vec![false; l];
        for &sid in &segments {
            let Some(seg) = hs.seg(sid) else { continue };
            let Some(idx) = seg.hint_val.and_then(|v| hs.value(v)).and_then(|v| v.idx) else { continue };
            if !at(&hs.good_mask, idx) || seg.suppressed {
                continue;
            }
            set(&mut mask, idx, true);
        }

        let glyph = self.glyph();
        if hs.has_overlaps
            && mask.contains(&true)
            && let Some(pe) = glyph.element(c)
        {
            // The original takes the first control point of a curve, not the second.
            let (p0, p1) = if pe.is_line { (pe.s, pe.e) } else { (pe.cs, pe.e) };
            let prev = glyph.prev_in_subpath(c, false, true, false).and_then(|p| glyph.element(p));
            let next = glyph.next_in_subpath(c, false, true, false).and_then(|n| glyph.element(n));
            for sidx in 0..l {
                for sjdx in sidx + 1..l {
                    if !(at(&mask, sidx) && at(&mask, sjdx) && at2(&hs.stem_overlaps, sidx, sjdx)) {
                        continue;
                    }
                    let (Some((segi, vali)), Some((segj, valj))) = (best_seg(hs, &segments, sidx), best_seg(hs, &segments, sjdx)) else {
                        continue;
                    };
                    let (Some(segi), Some(segj), Some(vali), Some(valj)) = (hs.seg(segi), hs.seg(segj), hs.value(vali), hs.value(valj)) else {
                        continue;
                    };
                    let (o0, o1) = (p0.o(dim), p1.o(dim));
                    let remidx = if vali.val < CONFLICT_VAL_MIN && self.ok_to_rem(segi.loc, vali.spc) {
                        Some(sidx)
                    } else if valj.val < CONFLICT_VAL_MIN && vali.val > valj.val * CONFLICT_VAL_MULT && self.ok_to_rem(segj.loc, valj.spc) {
                        Some(sjdx)
                    } else if pe.is_line
                        || (self.flat_quo(p0, p1, false) > 0.0 && self.ok_to_rem(segi.loc, vali.spc))
                        || prev.is_some_and(|p| diff_sign(o1 - o0, p.s.o(dim) - o0))
                    {
                        Some(sidx)
                    } else if next.is_some_and(|n| diff_sign(n.e.o(dim) - o1, o0 - o1)) {
                        Some(sjdx)
                    } else if (feq(o1, valj.lloc) || feq(o1, valj.uloc)) && fne(o0, vali.lloc) && fne(o0, vali.uloc) {
                        Some(sidx)
                    } else if (feq(o0, vali.lloc) || feq(o0, vali.uloc)) && fne(o1, valj.lloc) && fne(o1, valj.uloc) {
                        Some(sjdx)
                    } else {
                        None
                    };
                    match remidx {
                        Some(r) => set(&mut mask, r, false),
                        // Whatever the previous elements hint will then stand for this one.
                        None => {
                            set(&mut mask, sidx, false);
                            set(&mut mask, sjdx, false);
                        }
                    }
                }
            }
        }

        let any = mask.contains(&true);
        if let Some(ps) = self.st.hs.pe_states.get_mut(&c) {
            ps.mask = if any { mask } else { Vec::new() };
        }
    }

    /// A stem at `loc` may be dropped (`OKToRem`): it has no spacing bonus, or lies outside
    /// every zone.
    pub fn ok_to_rem(&self, loc: f64, spc: f64) -> bool {
        spc == 0.0 || (!self.in_band(loc, false) && !self.in_band(loc, true))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_geom::BezPath;

    use super::*;
    use crate::Options;
    use crate::glyph::{Dim, GlyphData};
    use crate::hinter::{DimState, Glyphs};
    use crate::state::{HintSegment, SegType, StemValue};

    // Expected values: `crates/autohint/tests/oracle/masks_state.py`, which runs otfautohint's
    // own functions on the same input.

    fn state(dim: Dim) -> DimState {
        DimState::new(dim, Arc::new(Options::default()))
    }

    fn conflicts(n: usize, pairs: &[(usize, usize)]) -> Vec<Vec<bool>> {
        let mut m = vec![vec![false; n]; n];
        for &(a, b) in pairs {
            m[a][b] = true;
            m[b][a] = true;
        }
        m
    }

    #[test]
    fn conflicting_stems_are_resolved_as_the_original_resolves_them() {
        let mut glyphs = Glyphs::default();
        let mut st = state(Dim::V);
        let chain = conflicts(4, &[(0, 1), (1, 2), (2, 3)]);
        let f = false;
        let t = true;

        st.hs.weights = vec![10.0, 1.0, 1.0, 10.0];
        let h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        assert_eq!(h.unconflict(&chain, None, None), [f, f, t, t]);

        st.hs.weights = vec![1.0, 10.0, 10.0, 1.0];
        let h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        assert_eq!(h.unconflict(&chain, None, None), [f, f, t, t]);

        st.hs.weights = vec![5.0; 5];
        let h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        assert_eq!(h.unconflict(&conflicts(5, &[(0, 4), (1, 2)]), Some(&[t; 5][..]), Some(&[f, f, f, f, t][..])), [t, f, t, t, t]);
    }

    #[test]
    fn many_conflicts_stay_within_the_work_budget() {
        let n = crate::MAX_STEMS;
        let pairs: Vec<(usize, usize)> = (0..n).flat_map(|i| [(i, i + 1), (i, i + 2)]).filter(|&(_, j)| j < n).collect();
        let mut glyphs = Glyphs::default();
        let mut st = state(Dim::V);
        st.hs.weights = vec![1.0; n];
        let h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        let kept = h.unconflict(&conflicts(n, &pairs), None, None);
        assert_eq!(kept.len(), n);
    }

    #[test]
    fn a_located_edge_falls_back_to_a_shared_segment_then_the_main_master() {
        let mut hs0 = GlyphHintState::default();
        let seg = |hs: &mut GlyphHintState| {
            hs.push_segment(HintSegment {
                loc: 0.0,
                min: 0.0,
                max: 10.0,
                bonus: 0.0,
                typ: SegType::Line,
                is_v: false,
                is_inc: true,
                desc: String::new(),
                pe: Some(PeRef::new(0, 0)),
                hint_val: None,
                replaced_by: None,
                deleted: false,
                suppressed: false,
            })
        };
        let (s0, s1, s2) = (seg(&mut hs0), seg(&mut hs0), seg(&mut hs0));
        for (lseg, useg, idx) in [(s0, s1, 0), (s1, s2, 1)] {
            let v = hs0.push_value(StemValue {
                val: 1.0,
                spc: 0.0,
                lloc: 0.0,
                uloc: 10.0,
                is_ghost: false,
                pruned: false,
                merge: false,
                lseg,
                useg,
                best: None,
                initial_val: 1.0,
                idx: Some(idx),
            });
            hs0.stem_values.push(v);
        }
        hs0.seg_mut(s1).unwrap().hint_val = Some(StemId(1));
        hs0.stems = Some(vec![vec![Stem::new(10.0, 50.0), Stem::new(60.0, 90.0)]]);
        let mut iss = vec![InstanceStemState::new(10.0), InstanceStemState::new(50.0), InstanceStemState::new(60.0), InstanceStemState::new(90.0)];
        iss[3].add_to_loc(77.0, 1.0, true, false, None);

        let mut glyphs = Glyphs::default();
        let mut st = state(Dim::H);
        let h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        assert_eq!(h.best_location(0, 1, &iss, &hs0), Some(77.0));
        assert_eq!(h.best_location(0, 0, &iss, &hs0), Some(10.0));
    }

    /// One closed contour: a line, a curve, then six lines (the last one closes the path).
    fn test_glyph() -> GlyphData {
        let mut p = BezPath::new();
        p.move_to((100.0, 0.0));
        p.line_to((100.0, 500.0));
        p.curve_to((100.0, 600.0), (230.0, 650.0), (230.0, 500.0));
        p.line_to((230.0, 0.0));
        p.line_to((480.0, 0.0));
        p.line_to((480.0, 300.0));
        p.line_to((400.0, 300.0));
        p.line_to((400.0, 0.0));
        p.close_path();
        let gd = GlyphData::from_contours("test", &[p], true).unwrap();
        assert_eq!(gd.subpaths.first().map(Vec::len), Some(8));
        gd
    }

    struct MaskResult {
        good: Vec<bool>,
        has_overlaps: bool,
        overlaps: Vec<Vec<bool>>,
        ghost_compat: Vec<Option<Vec<bool>>>,
        main: Vec<bool>,
        masks: Vec<Option<Vec<bool>>>,
    }

    /// Vertical stems 0 and 1 overlap, 2 is apart, 3 is a top ghost on 2's right edge. Each
    /// element asks for some of them; element 3 asks for none.
    fn mask_case(masters: &[&[(f64, f64)]]) -> MaskResult {
        let gd = test_glyph();
        let mut glyphs = Glyphs { main: gd.clone(), others: vec![gd; masters.len() - 1], ..Default::default() };
        let mut st = state(Dim::V);
        st.is_multi = masters.len() > 1;
        let hs = &mut st.hs;
        let seg = |hs: &mut GlyphHintState, loc: f64, typ: SegType, is_inc: bool, e: usize| {
            hs.push_segment(HintSegment {
                loc,
                min: 0.0,
                max: 10.0,
                bonus: 0.0,
                typ,
                is_v: true,
                is_inc,
                desc: String::new(),
                pe: Some(PeRef::new(0, e)),
                hint_val: None,
                replaced_by: None,
                deleted: false,
                suppressed: false,
            })
        };
        let value = |hs: &mut GlyphHintState, lloc: f64, uloc: f64, val: f64, lseg: SegId, useg: SegId, is_ghost: bool, idx: usize| {
            hs.push_value(StemValue {
                val,
                spc: 0.0,
                lloc,
                uloc,
                is_ghost,
                pruned: false,
                merge: false,
                lseg,
                useg,
                best: None,
                initial_val: val,
                idx: Some(idx),
            })
        };
        let ghost_seg = seg(hs, 480.0, SegType::Ghost, true, 0);
        let mut vals = vec![];
        for (i, (lloc, uloc, val)) in [(100.0, 180.0, 100.0), (150.0, 230.0, 40.0), (400.0, 480.0, 200.0)].into_iter().enumerate() {
            let lo = seg(hs, lloc, SegType::Line, false, 0);
            let up = seg(hs, uloc, SegType::Line, true, 0);
            vals.push(value(hs, lloc, uloc, val, lo, up, false, i));
        }
        let up3 = seg(hs, 480.0, SegType::Line, true, 0);
        vals.push(value(hs, 480.0, 480.0, 20.0, ghost_seg, up3, true, 3));

        let wants: [(usize, SegPlace, usize, bool); 12] = [
            (0, SegPlace::Start, 0, false),
            (1, SegPlace::Start, 0, false),
            (1, SegPlace::End, 1, false),
            (2, SegPlace::Start, 1, false),
            (4, SegPlace::Start, 2, false),
            (4, SegPlace::Middle, 3, false),
            (5, SegPlace::Start, 0, false),
            (5, SegPlace::End, 2, false),
            (6, SegPlace::Start, 2, false),
            (7, SegPlace::Start, 0, false),
            (7, SegPlace::End, 1, false),
            (7, SegPlace::Middle, 2, true),
        ];
        for (e, place, v, suppressed) in wants {
            let loc = hs.value(vals[v]).unwrap().lloc;
            let s = seg(hs, loc, SegType::Line, true, e);
            let sm = hs.seg_mut(s).unwrap();
            sm.hint_val = Some(vals[v]);
            sm.suppressed = suppressed;
            let ps = hs.pe_state(PeRef::new(0, e), true).unwrap();
            match place {
                SegPlace::Start => ps.s_segs.push(s),
                SegPlace::Middle => ps.m_segs.push(s),
                SegPlace::End => ps.e_segs.push(s),
            }
        }
        hs.stems = Some(masters.iter().map(|m| m.iter().map(|&(lb, rt)| Stem::new(lb, rt)).collect()).collect());
        hs.weights = vec![100.0, 40.0, 200.0, 20.0];
        hs.main_values = vec![vals[2], vals[0], vals[1], vals[3]];

        DimHinter { st: &mut st, glyphs: &mut glyphs }.convert_to_masks();
        let hs = &st.hs;
        MaskResult {
            good: hs.good_mask.clone(),
            has_overlaps: hs.has_overlaps,
            overlaps: hs.stem_overlaps.clone(),
            ghost_compat: hs.ghost_compat.clone(),
            main: hs.main_mask.clone(),
            masks: (0..8).map(|e| hs.pe_states.get(&PeRef::new(0, e)).map(|p| p.mask.clone())).collect(),
        }
    }

    const ONE: [(f64, f64); 4] = [(100.0, 180.0), (150.0, 230.0), (400.0, 480.0), (480.0, 460.0)];
    const TWO: [(f64, f64); 4] = [(160.0, 240.0), (140.0, 220.0), (410.0, 490.0), (490.0, 470.0)];

    #[test]
    fn overlapping_stems_are_split_over_the_element_masks() {
        let (f, t) = (false, true);
        let r = mask_case(&[&ONE]);
        assert_eq!(r.good, [t, t, t, t]);
        assert!(r.has_overlaps);
        assert_eq!(r.overlaps, [[f, t, f, f], [t, f, f, f], [f, f, f, t], [f, f, t, f]]);
        assert_eq!(r.ghost_compat, [None, None, None, Some(vec![f, f, t, f])]);
        assert_eq!(r.main, [t, f, t, f]);
        let expected: [Option<&[bool]>; 8] = [
            Some(&[t, f, f, f]),
            // Both stems of the curve are dropped: no rule decides between them.
            Some(&[]),
            Some(&[f, t, f, f]),
            None,
            Some(&[f, f, f, t]),
            Some(&[t, f, t, f]),
            Some(&[f, f, t, f]),
            Some(&[f, t, f, f]),
        ];
        for (e, want) in expected.into_iter().enumerate() {
            assert_eq!(r.masks[e].as_deref(), want, "element {e}");
        }
    }

    #[test]
    fn a_stem_out_of_order_in_another_master_is_removed() {
        let (f, t) = (false, true);
        let r = mask_case(&[&ONE, &TWO]);
        assert_eq!(r.good, [f, t, t, t]);
        assert!(r.has_overlaps);
        assert_eq!(r.overlaps, [[f, f, f, f], [f, f, f, f], [f, f, f, t], [f, f, t, f]]);
        assert_eq!(r.ghost_compat, [None, None, None, Some(vec![f, f, t, f])]);
        assert_eq!(r.main, [f, t, t, f]);
        let expected: [Option<&[bool]>; 8] = [
            Some(&[]),
            Some(&[f, t, f, f]),
            Some(&[f, t, f, f]),
            None,
            Some(&[f, f, f, t]),
            Some(&[f, f, t, f]),
            Some(&[f, f, t, f]),
            Some(&[f, t, f, f]),
        ];
        for (e, want) in expected.into_iter().enumerate() {
            assert_eq!(r.masks[e].as_deref(), want, "element {e}");
        }
    }
}
