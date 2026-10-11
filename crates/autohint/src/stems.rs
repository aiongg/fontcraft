//! Stem values: pairs of segments scored as stems, pruned, merged and narrowed to the main
//! values, then turned into stem lists.
//!
//! Ported from otfautohint's `hinter.py` lines 1073–1904 (`dimensionHinter`; AFDKO 5.0.1,
//! Apache-2.0, Copyright 2021 Adobe; see `NOTICE`). Package M10.10 fills the bodies marked
//! `// M10.10:`. Functions marked *shared* are called from other packages and keep their
//! signature.

use crate::hinter::DimHinter;
use crate::state::{SegId, StemId};

impl DimHinter<'_> {
    /// `genStemVals`. *Shared* (M10.9).
    pub fn gen_stem_vals(&mut self) {
        // M10.10: genStemVals.
    }

    /// `evalPair`: (value, spacing bonus) of the segments as a stem.
    pub fn eval_pair(&self, ls: SegId, us: SegId) -> (f64, f64) {
        // M10.10: evalPair.
        let _ = (ls, us);
        (0.0, 0.0)
    }

    /// `stemMiss`.
    pub fn stem_miss(&self, ls: SegId, us: SegId) -> Option<i32> {
        // M10.10: stemMiss.
        let _ = (ls, us);
        None
    }

    /// `addStemValue`.
    pub fn add_stem_value(&mut self, lloc: f64, uloc: f64, val: f64, spc: f64, lseg: SegId, useg: SegId) {
        // M10.10: addStemValue.
        let _ = (lloc, uloc, val, spc, lseg, useg);
    }

    /// `insertStemValue`: keeps `stem_values` ordered.
    pub fn insert_stem_value(&mut self, sv: StemId, note: &str) {
        // M10.10: insertStemValue.
        let _ = (sv, note);
    }

    /// `combineStemValues`.
    pub fn combine_stem_values(&mut self) {
        // M10.10: combineStemValues.
    }

    /// `pruneStemVals`. *Shared* (M10.9).
    pub fn prune_stem_vals(&mut self) {
        // M10.10: pruneStemVals.
    }

    /// `closeSegs`.
    pub fn close_segs(&self, s1: SegId, s2: SegId) -> bool {
        // M10.10: closeSegs.
        let _ = (s1, s2);
        false
    }

    /// `prune`.
    pub fn prune(&mut self, sv: StemId, other: StemId, desc: &str) {
        // M10.10: prune.
        let _ = (sv, other, desc);
    }

    /// `highestStemVals`. *Shared* (M10.9).
    pub fn highest_stem_vals(&mut self) {
        // M10.10: highestStemVals, findHighestValForSegs.
    }

    /// `findHighestVal`.
    pub fn find_highest_val(&self, seg: SegId, is_u: bool, loc_flag: bool) -> Option<StemId> {
        // M10.10: findHighestVal.
        let _ = (seg, is_u, loc_flag);
        None
    }

    /// `considerValForSeg`.
    pub fn consider_val_for_seg(&self, sv: StemId, seg: SegId, is_u: bool) -> bool {
        // M10.10: considerValForSeg.
        let _ = (sv, seg, is_u);
        false
    }

    /// `findBestValues`.
    pub fn find_best_values(&mut self) {
        // M10.10: findBestValues.
    }

    /// `replaceVals`.
    pub fn replace_vals(&mut self, oldl: f64, oldu: f64, newl: f64, newu: f64, newbest: StemId) {
        // M10.10: replaceVals.
        let _ = (oldl, oldu, newl, newu, newbest);
    }

    /// `mergeVals`. *Shared* (M10.9).
    pub fn merge_vals(&mut self) {
        // M10.10: mergeVals.
    }

    /// `limitVals`. *Shared* (M10.9).
    pub fn limit_vals(&mut self) {
        // M10.10: limitVals.
    }

    /// `checkVals`. *Shared* (M10.9).
    pub fn check_vals(&mut self) {
        // M10.10: checkVals.
    }

    /// `findLineSeg`.
    pub fn find_line_seg(&self, loc: f64, is_bottom: bool) -> bool {
        // M10.10: findLineSeg.
        let _ = (loc, is_bottom);
        false
    }

    /// `reportStems`: feeds the glyph report. *Shared* (M10.9).
    pub fn report_stems(&mut self) {
        // M10.10: reportStems.
    }

    /// `mainVals`. *Shared* (M10.9).
    pub fn main_vals(&mut self) {
        // M10.10: mainVals.
    }

    /// `mainOK`.
    pub fn main_ok(&self, spc: f64, val: f64, has_hints: bool, prev_bv: f64) -> bool {
        // M10.10: mainOK.
        let _ = (spc, val, has_hints, prev_bv);
        false
    }

    /// `tryCounterHinting`. *Shared* (M10.9).
    pub fn try_counter_hinting(&mut self) -> bool {
        // M10.10: tryCounterHinting.
        false
    }

    /// `addBBox`: bounding-box stems when nothing better exists. *Shared* (M10.9).
    pub fn add_bbox(&mut self, do_subpaths: bool) {
        // M10.10: addBBox.
        let _ = do_subpaths;
    }

    /// `markStraySegs`. *Shared* (M10.9).
    pub fn mark_stray_segs(&mut self) {
        // M10.10: markStraySegs.
    }

    /// `convertToStemLists`: the main values as stems, ghosts and weights; false when there is
    /// nothing to convert. *Shared* (M10.12).
    pub fn convert_to_stem_lists(&mut self) -> bool {
        // M10.10: convertToStemLists.
        false
    }
}
