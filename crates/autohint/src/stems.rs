//! Stem values: pairs of segments scored as stems, pruned, merged and narrowed to the main
//! values, then turned into stem lists.
//!
//! Ported from otfautohint's `hinter.py` lines 1073–1904 (`dimensionHinter`; AFDKO 5.0.1,
//! Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Functions marked *shared* are called from
//! other packages and keep their signature.
//!
//! Python compares stem values with `==` and `<` on their ghost-normalised locations
//! ([`StemValue::ghosted`]), and `max` over `(score, value)` tuples breaks score ties by that
//! order. [`DimHinter::sv_lt`], [`DimHinter::sv_eq`] and [`DimHinter::py_max`] keep those
//! semantics, because the chosen stems depend on them. Messages the original logs at info level
//! (stem near misses, trimming) are not kept: [`crate::GlyphHints::warnings`] holds warnings only.
//!
//! Work is bounded by the segment cap ([`crate::Options::segment_limit`]): at most
//! `cap × cap` pairs plus one ghost per segment, and pruning compares every pair of them, as the
//! original does.

use std::cmp::Ordering;
use std::collections::HashSet;

use crate::glyph::{Pt, Stem, feq, fne};
use crate::hinter::{
    BAND_MARGIN, CLOSE_MERGE, CP_FRAC, DELTA_DIFF_MIN, DimHinter, GAP_DIST_DENOM, GHOST_FACTOR, GHOST_LENGTH, GHOST_SPECIAL, GHOST_VAL, GHOST_WIDTH,
    MAX_BEND_MERGE, MAX_MERGE, MAX_VALUE, MIN_DIST, MIN_HIGH_SEG_VAL, MIN_VALUE, MUCH_PF, NO_OVERLAP_PENALTY, PRUNE_A, PRUNE_B, PRUNE_C, PRUNE_D,
    PRUNE_DISTANCE, PRUNE_FACTOR, PRUNE_FACTOR_LT, PRUNE_VALUE, S_FACTOR, SPC_BONUS, STEM_LIMIT, VERY_MUCH_PF,
};
use crate::state::{HintSegment, SegId, SegType, StemId, StemValue};

/// A `(compVal, stem value)` tuple as the original's `max` and `sort` see it.
type Scored = ((f64, f64), StemId);

/// A location pair as an exact dictionary key: `-0.0` and `0.0` are one key, as in Python.
fn loc_key(l: f64, u: f64) -> (u64, u64) {
    ((l + 0.0).to_bits(), (u + 0.0).to_bits())
}

impl DimHinter<'_> {
    // Arena access. Ids always come from this glyph's arenas; a missing one reads as nothing.

    fn sv(&self, id: StemId) -> Option<&StemValue> {
        self.st.hs.value(id)
    }

    fn sg(&self, id: SegId) -> Option<&HintSegment> {
        self.st.hs.seg(id)
    }

    fn seg_loc(&self, id: SegId) -> f64 {
        self.sg(id).map_or(0.0, |s| s.loc)
    }

    fn seg_has(&self, id: SegId, test: fn(&HintSegment) -> bool) -> bool {
        self.sg(id).is_some_and(test)
    }

    /// `stemValue.ghosted`.
    fn ghosted_of(&self, id: StemId) -> (f64, f64) {
        self.sv(id).map_or((0.0, 0.0), |v| v.ghosted(&self.st.hs.segments))
    }

    /// `stemValue.__lt__`: by ghosted lower, then upper location.
    fn sv_lt(&self, a: StemId, b: StemId) -> bool {
        let (al, au) = self.ghosted_of(a);
        let (bl, bu) = self.ghosted_of(b);
        al < bl || (al == bl && au < bu)
    }

    /// `==` between stem values in a Python list operation: identity, else equal ghosted
    /// locations.
    fn sv_eq(&self, a: StemId, b: StemId) -> bool {
        a == b || self.ghosted_of(a) == self.ghosted_of(b)
    }

    fn sv_order(&self, a: StemId, b: StemId) -> Ordering {
        if self.sv_lt(a, b) {
            Ordering::Less
        } else if self.sv_lt(b, a) {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }

    /// Python's `a > b` on `(compVal, stemValue)` tuples.
    fn scored_gt(&self, a: Scored, b: Scored) -> bool {
        let ((a0, a1), ai) = a;
        let ((b0, b1), bi) = b;
        if a0 != b0 {
            return a0 > b0;
        }
        if a1 != b1 {
            return a1 > b1;
        }
        if self.sv_eq(ai, bi) {
            return false;
        }
        self.sv_lt(bi, ai)
    }

    fn scored_order(&self, a: Scored, b: Scored) -> Ordering {
        if self.scored_gt(a, b) {
            Ordering::Greater
        } else if self.scored_gt(b, a) {
            Ordering::Less
        } else {
            Ordering::Equal
        }
    }

    /// Python's `max` over `(compVal, stemValue)` tuples: the first of equal maxima wins.
    fn py_max(&self, cands: impl IntoIterator<Item = Scored>) -> Option<StemId> {
        let mut best: Option<Scored> = None;
        for c in cands {
            best = match best {
                Some(b) if !self.scored_gt(c, b) => Some(b),
                _ => Some(c),
            };
        }
        best.map(|b| b.1)
    }

    fn comp_val_of(&self, id: StemId, spc_factor: f64, ghost_factor: f64) -> (f64, f64) {
        self.sv(id).map_or((0.0, 0.0), |v| v.comp_val(spc_factor, ghost_factor))
    }

    fn set_pruned(&mut self, id: StemId, pruned: bool) {
        if let Some(v) = self.st.hs.value_mut(id) {
            v.pruned = pruned;
        }
    }

    fn retain_unpruned(&mut self) {
        let hs = &mut self.st.hs;
        let values = &hs.values;
        hs.stem_values.retain(|id| values.get(id.0).is_some_and(|v| !v.pruned));
    }

    /// The ghost-normalised range of a value for overlap tests in `mainVals`: a ghost collapses
    /// to its real edge.
    fn collapsed(&self, id: StemId) -> (f64, f64) {
        let Some(v) = self.sv(id) else { return (0.0, 0.0) };
        let (mut l, mut u) = (v.lloc, v.uloc);
        if v.is_ghost {
            if self.seg_has(v.lseg, HintSegment::is_ghost) {
                l = u;
            } else {
                u = l;
            }
        }
        (l, u)
    }

    /// `genStemVals`. *Shared* (M10.9).
    pub fn gen_stem_vals(&mut self) {
        let (ll, ul) = self.segment_lists();
        for &ls in &ll {
            for &us in &ul {
                let (lloc, uloc) = (self.seg_loc(ls), self.seg_loc(us));
                if lloc > uloc {
                    continue;
                }
                let (val, spc) = self.eval_pair(ls, us);
                let _ = self.stem_miss(ls, us);
                self.add_stem_value(lloc, uloc, val, spc, ls, us);
            }
        }

        if self.has_bands() {
            // The original makes one ghost segment and moves it for each use; every ghost stem
            // value refers to it, so later reads see its last position. Kept that way.
            let ghost = self.st.hs.push_segment(HintSegment {
                loc: 0.0,
                min: 0.0,
                max: 0.0,
                bonus: 0.0,
                typ: SegType::Ghost,
                is_v: self.is_v(),
                is_inc: false,
                desc: "ghost".to_string(),
                pe: None,
                hint_val: None,
                replaced_by: None,
                deleted: false,
                suppressed: false,
            });
            for (list, is_bottom) in [(&ll, true), (&ul, false)] {
                for &s in list {
                    let Some(seg) = self.sg(s) else { continue };
                    let (loc, cntr, inc) = (seg.loc, (seg.max + seg.min) / 2.0, seg.is_inc);
                    if !self.in_band(loc, is_bottom) {
                        continue;
                    }
                    let gloc = if is_bottom { loc + GHOST_WIDTH } else { loc - GHOST_WIDTH };
                    if let Some(g) = self.st.hs.seg_mut(ghost) {
                        g.loc = gloc;
                        g.max = (cntr + GHOST_LENGTH / 2.0).round_ties_even();
                        g.min = (cntr - GHOST_LENGTH / 2.0).round_ties_even();
                        g.is_inc = !inc;
                    }
                    if is_bottom {
                        self.add_stem_value(loc, gloc, GHOST_VAL, GHOST_SPECIAL, s, ghost);
                    } else {
                        self.add_stem_value(gloc, loc, GHOST_VAL, GHOST_SPECIAL, ghost, s);
                    }
                }
            }
        }
        self.combine_stem_values();
    }

    /// `evalPair`: (value, spacing bonus) of the segments as a stem.
    pub fn eval_pair(&self, ls: SegId, us: SegId) -> (f64, f64) {
        let (Some(l), Some(u)) = (self.sg(ls), self.sg(us)) else { return (0.0, 0.0) };
        let mut spc = 0.0;
        let loc_d = (u.loc - l.loc).abs();
        if loc_d < MIN_DIST {
            return (0.0, spc);
        }
        let in_b = self.in_band(l.loc, true);
        let in_t = self.in_band(u.loc, false);
        if in_b && in_t {
            return (0.0, spc);
        }
        if in_b || in_t {
            spc += 2.0;
        }
        let mut dist;
        if u.min <= l.max && u.max >= l.min {
            let overlen = u.max.min(l.max) - u.min.max(l.min);
            let minlen = (u.max - u.min).min(l.max - l.min);
            // overlen <= minlen for overlapping ranges, so minlen 0 means overlen 0.
            if feq(overlen, minlen) || minlen <= 0.0 {
                dist = loc_d;
            } else {
                dist = loc_d * (1.0 + CP_FRAC * (1.0 - overlen / minlen));
            }
        } else {
            let o_d = (u.min - l.max).abs().min((u.max - l.min).abs());
            dist = (loc_d * NO_OVERLAP_PENALTY + o_d * o_d / GAP_DIST_DENOM).round_ties_even();
            if o_d > loc_d {
                dist *= o_d / loc_d;
            }
        }
        dist = dist.max(2.0 * MIN_DIST);
        if l.bonus.min(u.bonus) > 0.0 {
            spc += 2.0;
        }
        if self.dominant_stems().contains(&loc_d) {
            spc += 1.0;
        }
        dist = dist.max(2.0);
        let bl = (l.max - l.min).max(2.0);
        let ul = (u.max - u.min).max(2.0);
        let q = dist * dist;
        let mut v = 1000.0 * (bl * bl) * (ul * ul) / (q * q);
        let big = self.st.big_dist;
        if loc_d > big {
            let fac = big / loc_d;
            if fac > 0.5 {
                v *= fac.powf(8.0);
            } else {
                return (0.0, spc);
            }
        }
        if !v.is_finite() {
            return (0.0, spc);
        }
        (v.clamp(MIN_VALUE, MAX_VALUE), spc)
    }

    /// `stemMiss`: `Some(0)` for a pair too close to be a stem. The original only logs the near
    /// misses it finds otherwise, at info level.
    pub fn stem_miss(&self, ls: SegId, us: SegId) -> Option<i32> {
        let loc_d = (self.seg_loc(us) - self.seg_loc(ls)).abs();
        if loc_d < MIN_DIST { Some(0) } else { None }
    }

    /// `addStemValue`.
    pub fn add_stem_value(&mut self, lloc: f64, uloc: f64, val: f64, spc: f64, lseg: SegId, useg: SegId) {
        if val == 0.0 {
            return;
        }
        let pruning = self.st.pruning;
        if pruning && val < PRUNE_VALUE && spc <= 0.0 {
            return;
        }
        let (Some(l), Some(u)) = (self.sg(lseg), self.sg(useg)) else { return };
        let (l_bend, u_bend) = (l.is_bend(), u.is_bend());
        if l_bend && u_bend {
            return;
        }
        let ghst = l.is_ghost() || u.is_ghost();
        let (l_pe, u_pe) = (l.pe, u.pe);
        if !ghst && pruning && val <= PRUNE_D && spc <= 0.0 {
            if l_bend || u_bend {
                return;
            }
            // Weak stems across subpaths survive only when one subpath encloses the other.
            if let (Some(lp), Some(up)) = (l_pe, u_pe) {
                let (lsub, usub) = (self.subpath_of(lp), self.subpath_of(up));
                if lsub != usub {
                    let lsb = self.glyph_mut().get_bounds(Some(lsub));
                    let usb = self.glyph_mut().get_bounds(Some(usub));
                    let (Some(lsb), Some(usb)) = (lsb, usb) else { return };
                    if !lsb.within(&usb) && !usb.within(&lsb) {
                        return;
                    }
                }
            }
        }
        let (lloc, uloc) = if self.st.options.round_coords { (lloc.round_ties_even(), uloc.round_ties_even()) } else { (lloc, uloc) };
        let id = self.st.hs.push_value(StemValue {
            val,
            spc,
            lloc,
            uloc,
            is_ghost: ghst,
            pruned: false,
            merge: false,
            lseg,
            useg,
            best: None,
            initial_val: val,
            idx: None,
        });
        self.insert_stem_value(id, "add");
    }

    /// The subpath of an element (`pe.position[0]`).
    fn subpath_of(&self, pe: crate::glyph::PeRef) -> usize {
        self.glyph().element(pe).and_then(|e| e.position).map_or(pe.subpath, |p| p.0)
    }

    /// `insertStemValue`: keeps `stem_values` ordered.
    pub fn insert_stem_value(&mut self, sv: StemId, note: &str) {
        let _ = note;
        let n = self.st.hs.stem_values.len();
        let (mut lo, mut hi) = (0, n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.st.hs.stem_values.get(mid) {
                Some(&m) if self.sv_lt(m, sv) => lo = mid + 1,
                _ => hi = mid,
            }
        }
        let Some(new) = self.sv(sv).cloned() else { return };
        if new.is_ghost {
            let mut j = lo;
            while let Some(&o) = self.st.hs.stem_values.get(j) {
                if !self.sv_eq(o, sv) {
                    break;
                }
                if let Some(old) = self.sv(o)
                    && !old.is_ghost
                    && (feq(self.seg_loc(old.lseg), self.seg_loc(new.lseg)) || feq(self.seg_loc(old.useg), self.seg_loc(new.useg)))
                    && old.val > new.val
                {
                    return;
                }
                j += 1;
            }
        }
        self.st.hs.stem_values.insert(lo, sv);
    }

    /// `combineStemValues`.
    pub fn combine_stem_values(&mut self) {
        let svl = self.st.hs.stem_values.clone();
        let mut i = 0;
        while let Some(&first) = svl.get(i) {
            let mut val = self.sv(first).map_or(0.0, |v| v.val);
            let mut j = i + 1;
            while let Some(&other) = svl.get(j) {
                if !self.sv_eq(first, other) {
                    break;
                }
                if let Some(o) = self.sv(other) {
                    if o.is_ghost {
                        val = o.val;
                    } else {
                        val += o.val + 2.0 * (val * o.val).max(0.0).sqrt();
                    }
                }
                j += 1;
            }
            for id in svl.get(i..j).unwrap_or_default() {
                if let Some(v) = self.st.hs.value_mut(*id) {
                    v.val = val;
                }
            }
            i = j;
        }
    }

    /// `pruneStemVals`. *Shared* (M10.9).
    pub fn prune_stem_vals(&mut self) {
        let is_v = self.is_v();
        let ignore_pruned = !is_v && (self.st.options.report_zones || !self.st.options.report_stems);
        let svl = self.st.hs.stem_values.clone();
        for &c in &svl {
            let Some(cv) = self.sv(c).cloned() else { continue };
            let (mut other_low, mut other_high) = (false, false);
            let u_in = self.in_band(cv.uloc, false);
            let l_in = self.in_band(cv.lloc, true);
            for &d in &svl {
                let Some(dv) = self.sv(d).cloned() else { continue };
                if dv.pruned && !ignore_pruned {
                    continue;
                }
                if !cv.is_ghost && dv.is_ghost && dv.val <= cv.val * VERY_MUCH_PF {
                    continue;
                }
                if feq(cv.lloc, dv.lloc) && feq(cv.uloc, dv.uloc) {
                    continue;
                }
                if is_v && dv.val <= cv.val * PRUNE_FACTOR {
                    continue;
                }
                let csl = self.close_segs(cv.lseg, dv.lseg);
                let csu = self.close_segs(cv.useg, dv.useg);
                let cs_tst = if cv.val < 100.0 && dv.val > cv.val * MUCH_PF { csl || csu } else { csl && csu };
                if dv.val > cv.val * PRUNE_FACTOR
                    && cv.lloc - PRUNE_DISTANCE <= dv.lloc
                    && cv.uloc + PRUNE_DISTANCE >= dv.uloc
                    && cs_tst
                    && (is_v || cv.val < 16.0 || ((!u_in || feq(cv.uloc, dv.uloc)) && (!l_in || feq(cv.lloc, dv.lloc))))
                {
                    self.prune(c, d, "close and higher value");
                    break;
                }
                if (cv.lloc - dv.lloc).abs() < 1.0 {
                    if !other_low && cv.val < dv.val * PRUNE_FACTOR_LT && (dv.uloc - dv.lloc).abs() < (cv.uloc - cv.lloc).abs() && csl {
                        other_low = true;
                    }
                    if (is_v || (dv.val > cv.val * PRUNE_FACTOR && !u_in)) && self.seg_has(cv.useg, HintSegment::is_bend) && csl {
                        self.prune(c, d, "lower bend");
                        break;
                    }
                }
                if (cv.uloc - dv.uloc).abs() < 1.0 {
                    if !other_high && cv.val < dv.val * PRUNE_FACTOR_LT && (dv.uloc - dv.lloc).abs() < (cv.uloc - cv.lloc).abs() && csu {
                        other_high = true;
                    }
                    if (is_v || (dv.val > cv.val * PRUNE_FACTOR && !l_in)) && self.seg_has(cv.lseg, HintSegment::is_bend) && csu {
                        self.prune(c, d, "upper bend");
                        break;
                    }
                }
                if other_low && other_high {
                    self.prune(c, d, "low and high");
                    break;
                }
            }
        }
        self.retain_unpruned();
    }

    /// `closeSegs`: the segments, and the path between them, stay within `CLOSE_MERGE` of each
    /// other. A segment without an element (a ghost) is close to anything.
    pub fn close_segs(&self, s1: SegId, s2: SegId) -> bool {
        let (Some(a), Some(b)) = (self.sg(s1), self.sg(s2)) else { return false };
        let (Some(pe1), Some(pe2)) = (a.pe, b.pe) else { return true };
        if s1 == s2 || pe1 == pe2 {
            return true;
        }
        let (mut loca, mut locb) = if a.loc < b.loc { (a.loc, b.loc) } else { (b.loc, a.loc) };
        if locb - loca > 5.0 * CLOSE_MERGE {
            return false;
        }
        loca -= CLOSE_MERGE;
        locb += CLOSE_MERGE;
        let dim = self.dim();
        let glyph = self.glyph();
        let len = glyph.subpaths.get(self.subpath_of(pe1)).map_or(0, Vec::len);
        let mut n = Some(pe1);
        let mut p = glyph.prev_in_subpath(pe1, false, true, false);
        let (mut ngood, mut pgood) = (true, true);
        let end_o = |r: Option<crate::glyph::PeRef>| r.and_then(|r| glyph.element(r)).map(|e| e.e.o(dim));
        for _ in 0..len {
            if !ngood && !pgood {
                return false;
            }
            if (ngood && n == Some(pe2)) || (pgood && p == Some(pe2)) {
                return true;
            }
            match end_o(n) {
                Some(o) if o <= locb && o >= loca => {}
                _ => ngood = false,
            }
            match end_o(p) {
                Some(o) if o <= locb && o >= loca => {}
                _ => pgood = false,
            }
            n = n.and_then(|c| glyph.next_in_subpath(c, false, true, false));
            p = p.and_then(|c| glyph.prev_in_subpath(c, false, true, false));
        }
        false
    }

    /// `prune`.
    pub fn prune(&mut self, sv: StemId, other: StemId, desc: &str) {
        let _ = (other, desc);
        self.set_pruned(sv, true);
    }

    /// `highestStemVals`. *Shared* (M10.9).
    pub fn highest_stem_vals(&mut self) {
        for id in self.st.hs.stem_values.clone() {
            self.set_pruned(id, true);
        }
        let (ll, ul) = self.segment_lists();
        self.find_highest_val_for_segs(&ul, true);
        self.find_highest_val_for_segs(&ll, false);
        self.retain_unpruned();
    }

    /// `findHighestValForSegs`: ties each segment to the highest related stem value.
    fn find_highest_val_for_segs(&mut self, segl: &[SegId], is_u: bool) {
        for &seg in segl {
            let mut ghst = None;
            let mut highest = self.find_highest_val(seg, is_u, false);
            if let Some(h) = highest
                && self.sv(h).is_some_and(|v| v.is_ghost)
                && let Some(ng) = self.find_highest_val(seg, is_u, true)
                && self.sv(ng).is_some_and(|v| v.val >= 2.0)
            {
                ghst = Some(h);
                highest = Some(ng);
            }
            let Some(h) = highest else { continue };
            let hval = self.sv(h).map_or(0.0, |v| v.val);
            let gval = ghst.and_then(|g| self.sv(g)).map(|v| v.val);
            if !(hval < MIN_HIGH_SEG_VAL && gval.is_none_or(|g| g < MIN_HIGH_SEG_VAL)) {
                self.set_pruned(h, false);
                if let Some(s) = self.st.hs.seg_mut(seg) {
                    s.hint_val = Some(h);
                }
            }
        }
    }

    /// `findHighestVal`.
    pub fn find_highest_val(&self, seg: SegId, is_u: bool, loc_flag: bool) -> Option<StemId> {
        let seg_loc = self.sg(seg)?.loc;
        let cands = self.st.hs.stem_values.iter().filter_map(|&id| {
            let sv = self.sv(id)?;
            let (vs, vl) = if is_u { (sv.useg, sv.uloc) } else { (sv.lseg, sv.lloc) };
            if (vl - seg_loc).abs() > MAX_MERGE {
                return None;
            }
            let loctest = if loc_flag { !sv.is_ghost } else { vs == seg || self.close_segs(vs, seg) };
            if !loctest || !self.consider_val_for_seg(id, seg, is_u) {
                return None;
            }
            Some((sv.comp_val(SPC_BONUS, GHOST_FACTOR), id))
        });
        self.py_max(cands)
    }

    /// `considerValForSeg`.
    pub fn consider_val_for_seg(&self, sv: StemId, seg: SegId, is_u: bool) -> bool {
        let Some(v) = self.sv(sv) else { return false };
        if v.spc > 0.0 || self.in_band(self.seg_loc(seg), !is_u) {
            return true;
        }
        !(self.st.pruning && v.val < PRUNE_B)
    }

    /// `findBestValues`.
    pub fn find_best_values(&mut self) {
        let svl = self.st.hs.stem_values.clone();
        for (i, &si) in svl.iter().enumerate() {
            if self.sv(si).is_none_or(|v| v.best.is_some()) {
                continue;
            }
            let mut blst = vec![si];
            let mut b = si;
            for &sj in svl.iter().skip(i + 1) {
                let (Some(vj), Some(vb)) = (self.sv(sj), self.sv(b)) else { continue };
                // The original compares the upper location with the best value's lower one, so
                // in practice each value is its own best. Kept as is: otfautohint is the oracle.
                if vj.best.is_some() || fne(vj.lloc, vb.lloc) || fne(vj.uloc, vb.lloc) {
                    continue;
                }
                let better = (vj.spc == vb.spc && vj.val > vb.val) || vj.spc > vb.spc;
                blst.push(sj);
                if better {
                    b = sj;
                }
            }
            for id in blst {
                if let Some(v) = self.st.hs.value_mut(id) {
                    v.best = Some(b);
                }
            }
        }
    }

    /// `replaceVals`.
    pub fn replace_vals(&mut self, oldl: f64, oldu: f64, newl: f64, newu: f64, newbest: StemId) {
        let Some((val, spc)) = self.sv(newbest).map(|b| (b.val, b.spc)) else { return };
        for id in self.st.hs.stem_values.clone() {
            let Some(v) = self.st.hs.value_mut(id) else { continue };
            if fne(v.lloc, oldl) || fne(v.uloc, oldu) || v.merge {
                continue;
            }
            v.lloc = newl;
            v.uloc = newu;
            v.val = val;
            v.spc = spc;
            v.best = Some(newbest);
            v.merge = true;
        }
    }

    /// `mergeVals`. *Shared* (M10.9).
    pub fn merge_vals(&mut self) {
        self.find_best_values();
        // The original merges only while reporting zones; hinting stops here.
        if !self.st.options.report_zones {
            return;
        }
        let is_v = self.is_v();
        let svl = self.st.hs.stem_values.clone();
        for &id in &svl {
            if let Some(v) = self.st.hs.value_mut(id) {
                v.merge = false;
            }
        }
        loop {
            let cands = svl.iter().filter_map(|&id| {
                let v = self.sv(id)?;
                if v.merge {
                    return None;
                }
                Some((self.comp_val_of(v.best.unwrap_or(id), S_FACTOR, 1.0), id))
            });
            let Some(bst) = self.py_max(cands) else { break };
            if let Some(v) = self.st.hs.value_mut(bst) {
                v.merge = true;
            }
            let Some(bv) = self.sv(bst).cloned() else { break };
            let best = bv.best.unwrap_or(bst);
            for &sid in &svl {
                let Some(sv) = self.sv(sid).cloned() else { continue };
                let Some(bb) = self.sv(best).cloned() else { break };
                let mut replace = false;
                if sv.merge || bv.is_ghost != sv.is_ghost {
                    continue;
                }
                if feq(bv.lloc, sv.lloc) && feq(bv.uloc, sv.uloc) {
                    continue;
                }
                let svu_ib = self.in_band(sv.uloc, false);
                let svl_ib = self.in_band(sv.lloc, true);
                let btu_ib = self.in_band(bv.uloc, false);
                let btl_ib = self.in_band(bv.lloc, true);
                if (feq(bv.lloc, sv.lloc) && self.close_segs(bv.lseg, sv.lseg) && !btl_ib && !svu_ib && !btu_ib)
                    || (feq(bv.uloc, sv.uloc) && self.close_segs(bv.useg, sv.useg) && !btu_ib && !svl_ib && !btl_ib)
                    || ((bv.lloc - sv.lloc).abs() <= MAX_MERGE
                        && (bv.uloc - sv.uloc).abs() <= MAX_MERGE
                        && (is_v || feq(bv.lloc, sv.lloc) || !svl_ib)
                        && (is_v || feq(bv.uloc, sv.uloc) || !svu_ib))
                {
                    if feq(bb.spc, sv.spc) && feq(bb.val, sv.val) && !is_v {
                        if svl_ib {
                            replace = bv.lloc > sv.lloc;
                        } else if svu_ib {
                            replace = bv.uloc < sv.uloc;
                        }
                    } else {
                        replace = true;
                    }
                } else if feq(bb.spc, sv.spc) {
                    if (bv.lloc - sv.lloc).abs() <= 1.0 && (bv.uloc - sv.uloc).abs() <= MAX_BEND_MERGE {
                        replace = self.seg_has(sv.useg, HintSegment::is_bend) && !svu_ib;
                    } else if (bv.uloc - sv.uloc).abs() <= 1.0 && (bv.lloc - sv.lloc).abs() <= MAX_BEND_MERGE {
                        replace = self.seg_has(sv.lseg, HintSegment::is_bend) && !svl_ib;
                    }
                }
                if replace {
                    self.replace_vals(sv.lloc, sv.uloc, bv.lloc, bv.uloc, best);
                }
            }
        }
    }

    /// `limitVals`: at most [`STEM_LIMIT`] distinct locations, the highest scoring kept with all
    /// their duplicates. *Shared* (M10.9).
    pub fn limit_vals(&mut self) {
        let svl = self.st.hs.stem_values.clone();
        if svl.len() <= STEM_LIMIT {
            return;
        }
        let mut seen: HashSet<(u64, u64)> = HashSet::new();
        let mut reps = Vec::new();
        for &id in &svl {
            let Some(v) = self.sv(id) else { continue };
            if seen.insert(loc_key(v.lloc, v.uloc)) {
                reps.push(id);
            }
        }
        if reps.len() <= STEM_LIMIT {
            return;
        }
        let mut ol: Vec<Scored> = reps.iter().map(|&id| (self.comp_val_of(id, SPC_BONUS, 1.0), id)).collect();
        ol.sort_by(|a, b| self.scored_order(*b, *a));
        let kept: HashSet<(u64, u64)> = ol.iter().take(STEM_LIMIT).filter_map(|&(_, id)| self.sv(id).map(|v| loc_key(v.lloc, v.uloc))).collect();
        let mut kept_stems: Vec<StemId> =
            svl.into_iter().filter(|&id| self.sv(id).is_some_and(|v| kept.contains(&loc_key(v.lloc, v.uloc)))).collect();
        kept_stems.sort_by(|&a, &b| self.sv_order(a, b));
        self.st.hs.stem_values = kept_stems;
    }

    /// `checkVals`: the original logs stems within 2 units of a dominant width, at info level
    /// only, so nothing is kept. *Shared* (M10.9).
    pub fn check_vals(&mut self) {}

    /// `findLineSeg`: a line segment lies at `loc` in the lower (`is_bottom`) or upper list.
    pub fn find_line_seg(&self, loc: f64, is_bottom: bool) -> bool {
        let (ll, ul) = self.segment_lists();
        let list = if is_bottom { ll } else { ul };
        list.iter().any(|&s| self.sg(s).is_some_and(|s| feq(s.loc, loc) && s.is_line()))
    }

    /// `reportStems`: feeds the glyph report. *Shared* (M10.9).
    pub fn report_stems(&mut self) {
        if self.st.report.is_none() {
            return;
        }
        let is_v = self.is_v();
        let mut top = -1e40_f64;
        let mut bot = 1e40_f64;
        let mut stems = Vec::new();
        for &id in &self.st.hs.stem_values {
            let Some(v) = self.sv(id) else { continue };
            let (l, u) = (v.lloc, v.uloc);
            bot = bot.min(l);
            top = top.max(u);
            if !v.is_ghost {
                stems.push((l, u, self.find_line_seg(l, true) && self.find_line_seg(u, false)));
            }
        }
        let Some(report) = self.st.report.as_mut() else { return };
        for (l, u, line) in stems {
            report.stem(l, u, line, is_v);
            if !is_v {
                report.stem_zone(l, u);
            }
        }
        if !is_v && top > bot {
            report.char_zone(bot, top);
        }
    }

    /// `mainVals`: the highest-valued stems that do not overlap, widened by the band margin.
    /// *Shared* (M10.9).
    pub fn main_vals(&mut self) {
        let mut main: Vec<StemId> = Vec::new();
        let mut reject: Vec<StemId> = Vec::new();
        let mut svl = self.st.hs.stem_values.clone();
        let mut prev_bv = 0.0;
        loop {
            let has_hints = !main.is_empty();
            let cands = svl.iter().filter_map(|&id| {
                let v = self.sv(id)?;
                self.main_ok(v.spc, v.val, has_hints, prev_bv).then(|| (v.comp_val(SPC_BONUS, 1.0), id))
            });
            let Some(mut best) = self.py_max(cands) else { break };
            let Some(bv) = self.sv(best).cloned() else { break };
            if bv.is_ghost {
                let (mut lseg, mut useg) = (bv.lseg, bv.useg);
                if let Some(real) = svl.iter().filter_map(|&id| self.sv(id)).find(|v| feq(bv.lloc, v.lloc) && feq(bv.uloc, v.uloc) && !v.is_ghost) {
                    (lseg, useg) = (real.lseg, real.useg);
                }
                let other = if self.seg_has(lseg, HintSegment::is_ghost) {
                    Some(useg)
                } else if self.seg_has(useg, HintSegment::is_ghost) {
                    Some(lseg)
                } else {
                    None
                };
                if let Some(nb) = other.and_then(|s| self.sg(s)).and_then(|s| s.hint_val)
                    && nb != best
                    && svl.iter().any(|&x| self.sv_eq(x, nb))
                {
                    best = nb;
                }
            }
            // `list.remove` takes out the first equal value, not necessarily `best` itself.
            let Some(pos) = svl.iter().position(|&x| self.sv_eq(x, best)) else { break };
            svl.remove(pos);
            prev_bv = self.sv(best).map_or(0.0, |v| v.val);
            main.push(best);
            let (l, u) = self.collapsed(best);
            let (lb, ub) = (l - BAND_MARGIN, u + BAND_MARGIN);
            let mut i = 0;
            while let Some(&id) = svl.get(i) {
                let (l, u) = self.collapsed(id);
                if l <= ub && u >= lb {
                    reject.push(id);
                    svl.remove(i);
                } else {
                    i += 1;
                }
            }
        }
        reject.extend(svl);
        self.st.hs.main_values = main;
        self.st.hs.reject_values = reject;
    }

    /// `mainOK`.
    pub fn main_ok(&self, spc: f64, val: f64, has_hints: bool, prev_bv: f64) -> bool {
        let pruning = self.st.pruning;
        if spc > 0.0 {
            return true;
        }
        if !has_hints {
            return !pruning || val >= PRUNE_D;
        }
        if !pruning || val > PRUNE_A {
            return true;
        }
        if pruning && val < PRUNE_B {
            return false;
        }
        !pruning || prev_bv <= val * PRUNE_C
    }

    /// `tryCounterHinting`: the three highest main values, when equally wide and equally
    /// spaced, become the only ones. *Shared* (M10.9).
    pub fn try_counter_hinting(&mut self) -> bool {
        let (mut minloc, mut midloc, mut maxloc) = (1e40_f64, 1e40_f64, 1e40_f64);
        let (mut mindelta, mut middelta, mut maxdelta) = (0.0_f64, 0.0_f64, 0.0_f64);
        let hvl = self.st.hs.main_values.clone();
        if hvl.len() < 3 {
            return false;
        }
        let val = |i: usize| hvl.get(i).and_then(|&id| self.sv(id)).map_or(0.0, |v| v.val);
        let bestv = val(0);
        let pbestv = if hvl.len() > 3 { val(3) } else { 0.0 };
        if pbestv > 1000.0 || bestv < pbestv * 10.0 {
            return false;
        }
        for &id in hvl.iter().take(3) {
            let Some(hv) = self.sv(id) else { continue };
            let delta = hv.uloc - hv.lloc;
            let loc = hv.lloc + delta / 2.0;
            if loc < minloc {
                (maxloc, midloc, minloc) = (midloc, minloc, loc);
                (maxdelta, middelta, mindelta) = (middelta, mindelta, delta);
            } else if loc < midloc {
                (maxloc, midloc) = (midloc, loc);
                (maxdelta, middelta) = (middelta, delta);
            } else {
                maxloc = loc;
                maxdelta = delta;
            }
        }
        let _ = middelta;
        let spacing = ((maxloc - midloc) - (midloc - minloc)).abs();
        if (mindelta - maxdelta).abs() < DELTA_DIFF_MIN && spacing < DELTA_DIFF_MIN {
            self.st.hs.reject_values.extend(hvl.iter().skip(3));
            self.st.hs.main_values = hvl.into_iter().take(3).collect();
            return true;
        }
        // Within DELTA_DIFF_REPORT the original logs a near miss, at info level.
        false
    }

    /// `addBBox`: bounding-box stems when nothing better exists. *Shared* (M10.9).
    pub fn add_bbox(&mut self, do_subpaths: bool) {
        let is_v = self.is_v();
        let dim = self.dim();
        let params: Vec<Option<usize>> = if do_subpaths { (0..self.glyph().subpaths.len()).map(Some).collect() } else { vec![None] };
        let (ltype, utype) = if do_subpaths { (SegType::LsBbox, SegType::UsBbox) } else { (SegType::LgBbox, SegType::UgBbox) };
        let peidx = if is_v { 0 } else { 1 };
        for param in params {
            let Some(pbs) = self.glyph_mut().get_bounds(param) else { continue };
            let [[x0, y0], [x1, y1]] = pbs.bounds;
            let (mn, mx) = (Pt::new(x0, y0), Pt::new(x1, y1));
            let (mn_o, mx_o, mn_a, mx_a) = (mn.o(dim), mx.o(dim), mn.a(dim), mx.a(dim));
            if self.st.hs.main_values.iter().filter_map(|&id| self.sv(id)).any(|hv| hv.lloc <= mx_o && mn_o <= hv.uloc) {
                continue;
            }
            let (Some(lpe), Some(upe)) =
                (pbs.extpes.first().and_then(|r| r.get(peidx)).copied(), pbs.extpes.get(1).and_then(|r| r.get(peidx)).copied())
            else {
                continue;
            };
            let make = |loc: f64, pe, typ, is_inc, desc: &str| HintSegment {
                loc,
                min: mn_a,
                max: mx_a,
                bonus: 0.0,
                typ,
                is_v,
                is_inc,
                desc: desc.to_string(),
                pe: Some(pe),
                hint_val: None,
                replaced_by: None,
                deleted: false,
                suppressed: false,
            };
            let lseg = self.st.hs.push_segment(make(mn_o, lpe, ltype, !is_v, "l bbox"));
            if let Some(ps) = self.st.hs.pe_state(lpe, true) {
                ps.m_segs.push(lseg);
            }
            let useg = self.st.hs.push_segment(make(mx_o, upe, utype, is_v, "u bbox"));
            if let Some(ps) = self.st.hs.pe_state(upe, true) {
                ps.m_segs.push(useg);
            }
            let round = self.st.options.round_coords;
            let (lloc, uloc) = if round { (mn_o.round_ties_even(), mx_o.round_ties_even()) } else { (mn_o, mx_o) };
            let hv = self.st.hs.push_value(StemValue {
                val: 100.0,
                spc: 0.0,
                lloc,
                uloc,
                is_ghost: false,
                pruned: false,
                merge: false,
                lseg,
                useg,
                best: None,
                initial_val: 100.0,
                idx: None,
            });
            self.insert_stem_value(hv, "bboxadd");
            self.st.hs.main_values.push(hv);
            let mut mv = std::mem::take(&mut self.st.hs.main_values);
            mv.sort_by(|&a, &b| {
                let (ka, kb) = (self.comp_val_of(a, SPC_BONUS, 1.0), self.comp_val_of(b, SPC_BONUS, 1.0));
                kb.partial_cmp(&ka).unwrap_or(Ordering::Equal)
            });
            self.st.hs.main_values = mv;
        }
    }

    /// `markStraySegs`: a segment of a chosen stem that no value claimed takes that stem.
    /// *Shared* (M10.9).
    pub fn mark_stray_segs(&mut self) {
        for id in self.st.hs.stem_values.clone() {
            let Some(v) = self.sv(id) else { continue };
            for seg in [v.lseg, v.useg] {
                if let Some(s) = self.st.hs.seg_mut(seg)
                    && !s.is_ghost()
                    && s.hint_val.is_none()
                {
                    s.hint_val = Some(id);
                }
            }
        }
    }

    /// `convertToStemLists`: the main values as stems, ghosts and weights; false when there is
    /// nothing to convert. *Shared* (M10.12).
    pub fn convert_to_stem_lists(&mut self) -> bool {
        if self.st.keep_hints {
            // Unreachable in the original, which always clears existing hints first; there the
            // vertical branch falls through and the horizontal one compares a count with a list.
            // This follows the evident intent: keep each master's stems and report whether
            // every master has as many as the main one.
            let is_v = self.is_v();
            let stems: Vec<Vec<Stem>> = std::iter::once(&self.glyphs.main)
                .chain(&self.glyphs.others)
                .map(|g| if is_v { g.vstems.clone() } else { g.hstems.clone() })
                .collect();
            let n = stems.first().map_or(0, Vec::len);
            let same = stems.iter().all(|s| s.len() == n);
            self.st.hs.stems = Some(stems);
            return same;
        }
        let remove_conflicts = self.st.options.remove_conflicts;
        let mut dsl: Vec<Stem> = Vec::new();
        let mut pairs: Vec<((f64, f64), usize)> = Vec::new();
        if remove_conflicts {
            self.st.hs.weights = Vec::new();
        }
        if self.st.hs.counter_hinted {
            let mut sorted = self.st.hs.main_values.clone();
            sorted.sort_by(|&a, &b| self.sv_order(a, b));
            self.st.hs.stem_values = sorted;
        }
        for id in self.st.hs.stem_values.clone() {
            let p = self.ghosted_of(id);
            let weight = self.comp_val_of(id, SPC_BONUS, 1.0).0.max(0.0).powf(0.25);
            let sidx = match pairs.iter().find(|(q, _)| *q == p) {
                Some(&(_, sidx)) => {
                    if remove_conflicts && let Some(w) = self.st.hs.weights.get_mut(sidx) {
                        *w += weight;
                    }
                    sidx
                }
                None => {
                    let sidx = dsl.len();
                    pairs.push((p, sidx));
                    dsl.push(Stem::new(p.0, p.1));
                    if remove_conflicts {
                        self.st.hs.weights.push(weight);
                    }
                    sidx
                }
            };
            if let Some(v) = self.st.hs.value_mut(id) {
                v.idx = Some(sidx);
            }
        }
        let mut stems = vec![Vec::new(); self.glyphs.len()];
        if let Some(first) = stems.first_mut() {
            *first = dsl;
        }
        self.st.hs.stems = Some(stems);
        for glidx in 1..self.glyphs.len() {
            self.calc_instance_stems(glidx);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::Options;
    use crate::fddict::{BlueValue, FdDict};
    use crate::glyph::Dim;
    use crate::hinter::{BIG_DIST_FACTOR, DimState, Glyphs, MAX_STEM_DIST};

    // Expected values come from running otfautohint 5.0.1's own `hhinter` and `vhinter` methods
    // on the same segments (segments without elements, so every pair counts as close).

    fn zone(top: f64, bottom: f64, is_bottom: bool) -> BlueValue {
        BlueValue { top, bottom, name: String::new(), dict_name: String::new(), is_bottom }
    }

    fn state(dim: Dim, name: &str, blues: Vec<BlueValue>, dominant: f64) -> DimState {
        let dict = FdDict { blue_pairs: blues, blue_fuzz: 1.0, dominant_h: vec![dominant], dominant_v: vec![dominant], ..FdDict::default() };
        let mut st = DimState::new(dim, Arc::new(Options::default()));
        st.set_glyph(vec![Arc::new(dict)], None, 1, name);
        st.big_dist = dominant.max(MAX_STEM_DIST) * BIG_DIST_FACTOR;
        st
    }

    fn seg(st: &mut DimState, loc: f64, min: f64, max: f64, typ: SegType, is_inc: bool) -> SegId {
        let is_v = st.dim.is_v();
        let id = st.hs.push_segment(HintSegment {
            loc,
            min,
            max,
            bonus: 0.0,
            typ,
            is_v,
            is_inc,
            desc: String::new(),
            pe: None,
            hint_val: None,
            replaced_by: None,
            deleted: false,
            suppressed: false,
        });
        if is_inc {
            st.hs.increasing_segs.push(id)
        } else {
            st.hs.decreasing_segs.push(id)
        }
        id
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * a.abs().max(1.0)
    }

    #[track_caller]
    fn assert_values(st: &DimState, ids: &[StemId], want: &[(f64, f64, f64, f64, bool)]) {
        let got: Vec<_> = ids.iter().filter_map(|&id| st.hs.value(id)).map(|v| (v.lloc, v.uloc, v.val, v.spc, v.is_ghost)).collect();
        assert_eq!(got.len(), want.len(), "{got:?}");
        for (g, w) in got.iter().zip(want) {
            assert!(g.0 == w.0 && g.1 == w.1 && close(g.2, w.2) && g.3 == w.3 && g.4 == w.4, "got {g:?}, want {w:?}");
        }
    }

    fn hint_of(st: &DimState, s: SegId) -> Option<(f64, f64)> {
        st.hs.seg(s)?.hint_val.and_then(|id| st.hs.value(id)).map(|v| (v.lloc, v.uloc))
    }

    #[test]
    fn horizontal_stems_are_scored_pruned_and_chosen_as_otfautohint_does() {
        let blues = vec![zone(0.0, -12.0, true), zone(712.0, 700.0, false)];
        let mut st = state(Dim::H, "E", blues, 80.0);
        let lows = [0.0, 330.0, 620.0].map(|y| seg(&mut st, y, 100.0, if y == 330.0 { 450.0 } else { 500.0 }, SegType::Line, true));
        let up80 = seg(&mut st, 80.0, 100.0, 500.0, SegType::Line, false);
        let up85 = seg(&mut st, 85.0, 200.0, 230.0, SegType::Curve, false);
        let up400 = seg(&mut st, 400.0, 100.0, 450.0, SegType::Line, false);
        let up700 = seg(&mut st, 700.0, 100.0, 500.0, SegType::Line, false);
        let mut glyphs = Glyphs::default();
        let mut h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        h.start_hint();

        let (v, spc) = h.eval_pair(lows[0], up85);
        assert!(close(v, 2758.587660588355) && spc == 2.0, "{v} {spc}");
        assert_eq!(h.eval_pair(lows[0], up400), (0.0, 2.0));

        h.gen_stem_vals();
        let svl = h.st.hs.stem_values.clone();
        assert_values(
            h.st,
            &svl,
            &[
                (0.0, 80.0, 625000.0, 3.0, false),
                (0.0, 85.0, 2758.587660588355, 2.0, false),
                (0.0, 20.0, 20.0, 2.0, true),
                (330.0, 400.0, 625000.0, 0.0, false),
                (620.0, 700.0, 625000.0, 3.0, false),
                (680.0, 700.0, 20.0, 2.0, true),
            ],
        );

        h.prune_stem_vals();
        assert_eq!(h.st.hs.stem_values.len(), 5, "the short curve pair is pruned");
        h.highest_stem_vals();
        let svl = h.st.hs.stem_values.clone();
        assert_values(h.st, &svl, &[(0.0, 80.0, 625000.0, 3.0, false), (330.0, 400.0, 625000.0, 0.0, false), (620.0, 700.0, 625000.0, 3.0, false)]);
        assert_eq!(hint_of(h.st, lows[0]), Some((0.0, 80.0)));
        assert_eq!(hint_of(h.st, up80), Some((0.0, 80.0)));
        assert_eq!(hint_of(h.st, up85), None);
        assert_eq!(hint_of(h.st, up400), Some((330.0, 400.0)));
        assert_eq!(hint_of(h.st, up700), Some((620.0, 700.0)));

        h.merge_vals();
        h.limit_vals();
        h.main_vals();
        let main = h.st.hs.main_values.clone();
        // Equal scores: the tie goes to the value with the higher location.
        assert_values(h.st, &main, &[(620.0, 700.0, 625000.0, 3.0, false), (0.0, 80.0, 625000.0, 3.0, false), (330.0, 400.0, 625000.0, 0.0, false)]);
        assert!(h.st.hs.reject_values.is_empty());

        h.mark_stray_segs();
        assert!(h.convert_to_stem_lists());
        let stems = h.st.hs.stems.clone().unwrap_or_default();
        assert_eq!(stems, vec![vec![Stem::new(0.0, 80.0), Stem::new(330.0, 400.0), Stem::new(620.0, 700.0)]]);
        let want = [158.11388300841898, 28.117066259517454, 158.11388300841898];
        assert!(h.st.hs.weights.iter().zip(want).all(|(g, w)| close(*g, w)), "{:?}", h.st.hs.weights);
    }

    fn counter_glyph(last: f64) -> (DimState, bool) {
        let mut st = state(Dim::V, "m", Vec::new(), 80.0);
        for (loc, inc) in [(50.0, false), (130.0, true), (300.0, false), (380.0, true), (550.0, false), (last, true)] {
            seg(&mut st, loc, 0.0, 500.0, SegType::Line, inc);
        }
        let mut glyphs = Glyphs::default();
        let mut h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        h.start_hint();
        assert!(h.is_counter_glyph());
        h.st.pruning = false;
        h.gen_stem_vals();
        h.prune_stem_vals();
        h.highest_stem_vals();
        h.merge_vals();
        h.limit_vals();
        h.main_vals();
        h.st.pruning = true;
        let counter = h.try_counter_hinting();
        (st, counter)
    }

    #[test]
    fn three_equal_vertical_stems_get_counter_hints_and_unequal_ones_do_not() {
        let (st, counter) = counter_glyph(630.0);
        assert!(counter);
        let w = 1525878.90625;
        assert_values(&st, &st.hs.main_values, &[(550.0, 630.0, w, 1.0, false), (300.0, 380.0, w, 1.0, false), (50.0, 130.0, w, 1.0, false)]);

        let (st, counter) = counter_glyph(632.0);
        assert!(!counter);
        assert_values(
            &st,
            &st.hs.main_values,
            &[(300.0, 380.0, w, 1.0, false), (50.0, 130.0, w, 1.0, false), (550.0, 632.0, 1382370.979003532, 0.0, false)],
        );
    }

    #[test]
    fn zones_count_with_blue_fuzz_and_not_for_symbol_glyphs() {
        let blues = vec![zone(0.0, -12.0, true), zone(712.0, 700.0, false)];
        let mut st = state(Dim::H, "H", blues.clone(), 80.0);
        let mut glyphs = Glyphs::default();
        let mut h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        h.start_hint();
        assert!(h.has_bands());
        assert!(h.in_band(1.0, true));
        assert!(!h.in_band(1.5, true));
        assert!(!h.in_band(0.0, false));
        assert!(h.in_band(699.0, false));

        let mut st = state(Dim::H, "at", blues, 80.0);
        let mut h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        h.start_hint();
        assert!(!h.in_band(0.0, true));

        let mut st = state(Dim::V, "H", Vec::new(), 80.0);
        let h = DimHinter { st: &mut st, glyphs: &mut glyphs };
        assert!(!h.has_bands() && !h.is_counter_glyph());
        let dict = FdDict { v_counter_chars: vec!["H".to_string()], ..FdDict::default() };
        h.st.fddicts = vec![Arc::new(dict)];
        assert!(h.is_counter_glyph());
    }
}
