//! Hinting one glyph in several compatible masters at once, so that every master gets the same
//! stems in the same order and the same masks (the input of a CFF2 font, written in M9).
//!
//! Ported from otfautohint's `hinter.py` (`compatiblePaths`, `otherInstanceStems`,
//! `otherInstanceMasks`) and the per-glyph part of `autohint.py` (AFDKO 5.0.1, Apache-2.0,
//! Copyright 2021 Adobe; see `NOTICE`). Rows: `autohint.ps.variable.*` (data half).

use std::sync::Arc;

use crate::driver::overlap_state;
use crate::glyph::{Dim, GlyphData, PeRef};
use crate::hinter::{GlyphHinter, Glyphs};
use crate::{AutohintError, FdDict, GlyphHints, HintInput, MAX_MASTERS};

/// Hints the same glyph in every master, main master first. Every input must draw the same path
/// structure (same subpaths, same element kinds); the result has one entry per input, in order.
/// A master without contours is left out of the hinting and gets empty hints, as a sparse
/// master does in the original. More than [`MAX_MASTERS`] inputs is an error.
pub fn hint_masters(inputs: &[HintInput]) -> Result<Vec<GlyphHints>, AutohintError> {
    if inputs.len() > MAX_MASTERS {
        return Err(AutohintError::LimitExceeded { what: "masters hinted together", limit: MAX_MASTERS });
    }
    let mut present: Vec<(usize, GlyphData)> = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        if input.contours.is_empty() {
            continue;
        }
        let glyph = GlyphData::from_contours(input.name, input.contours, input.options.round_coords)?;
        if !glyph.is_empty() {
            present.push((i, glyph));
        }
    }
    let mut results = vec![GlyphHints::default(); inputs.len()];
    let Some(&(first, _)) = present.first() else { return Ok(results) };
    let Some(main_input) = inputs.get(first) else { return Ok(results) };
    let multi = present.len() > 1;

    let fddicts: Vec<Arc<FdDict>> = present.iter().filter_map(|(i, _)| inputs.get(*i)).map(|input| Arc::new(input.dict.clone())).collect();
    let mut glyphs = present.iter().map(|(_, g)| g.clone());
    let Some(main) = glyphs.next() else { return Ok(results) };
    // Only the main master's overlap-free outline reaches segment generation: the other masters'
    // segments are made while their stems are placed, from a fresh state.
    let mut warnings = Vec::new();
    let overlap = overlap_state(&main, main_input, multi, &mut warnings);
    let mut hinter = GlyphHinter::new(Arc::new(main_input.options.clone()));
    hinter.glyphs = Glyphs { main, others: glyphs.collect() };
    let hinted = hinter.hint_with_overlap(main_input.name, fddicts, overlap)?;
    if !hinted && multi {
        let reason = hinter.h.warnings.last().cloned().unwrap_or_else(|| "the paths differ".to_string());
        return Err(AutohintError::Incompatible { glyph: main_input.name.to_string(), reason });
    }
    for (k, (i, _)) in present.iter().enumerate() {
        if let (Some(slot), Some(mut hints)) = (results.get_mut(*i), hinter.result(k)) {
            let mut w = warnings.clone();
            w.append(&mut hints.warnings);
            hints.warnings = w;
            *slot = hints;
        }
    }
    Ok(results)
}

impl GlyphHinter {
    /// The masters' paths match element for element (`compatiblePaths`). A subpath that one
    /// master closes with a line and another does not gets a zero-length closing line where it
    /// lacks one; an element that is a line in some masters and a curve in others is made a
    /// curve everywhere when [`crate::Options::allow_changes`] is set, and is an incompatibility
    /// otherwise. The reason for a failure is the last warning of the horizontal state.
    pub fn compatible_paths(&mut self, fddicts: &[Arc<FdDict>]) -> bool {
        if self.glyphs.others.is_empty() {
            return true;
        }
        let master_name = |i: usize| fddicts.get(i).and_then(|d| d.font_name.clone()).unwrap_or_else(|| format!("master {i}"));

        for i in 0..self.glyphs.others.len() {
            let other = master_name(i + 1);
            let main_count = self.glyphs.main.subpaths.len();
            let other_count = self.glyphs.others.get(i).map_or(0, |g| g.subpaths.len());
            if main_count != other_count {
                self.h.warnings.push(format!(
                    "{}: not hinted, because it has {main_count} contours in the main master and {other_count} in {other}",
                    self.h.name
                ));
                return false;
            }
            for si in 0..main_count {
                let shape = |g: Option<&GlyphData>| {
                    g.and_then(|g| g.subpaths.get(si)).map_or((0, false), |sp| (sp.len(), sp.last().is_some_and(|pe| pe.is_close)))
                };
                let (dpl, dp_close) = shape(Some(&self.glyphs.main));
                let (gpl, gp_close) = shape(self.glyphs.others.get(i));
                if gpl == dpl {
                    continue;
                }
                if gpl == dpl + 1 && gp_close && !dp_close {
                    for gi in 0..=i {
                        if let Some(g) = self.glyphs.get_mut(gi) {
                            g.add_null_close(si);
                        }
                    }
                    continue;
                }
                if dpl == gpl + 1 && dp_close && !gp_close {
                    if let Some(g) = self.glyphs.others.get_mut(i) {
                        g.add_null_close(si);
                    }
                    continue;
                }
                self.h
                    .warnings
                    .push(format!("{}: not hinted, because contour {si} has {dpl} segments in the main master and {gpl} in {other}", self.h.name));
                return false;
            }
        }

        let elements: Vec<PeRef> = self.glyphs.main.elements().collect();
        for c in elements {
            let is_line: Vec<bool> =
                (0..self.glyphs.len()).filter_map(|gi| self.glyphs.get(gi).and_then(|g| g.element(c)).map(|pe| pe.is_line)).collect();
            if is_line.contains(&true) && is_line.contains(&false) {
                if self.options.allow_changes {
                    for gi in 0..self.glyphs.len() {
                        if let Some(pe) = self.glyphs.get_mut(gi).and_then(|g| g.element_mut(c))
                            && pe.is_line
                        {
                            pe.convert_to_curve(None, None);
                        }
                    }
                } else {
                    self.h.warnings.push(format!(
                        "{}: not hinted, because a segment is a line in some masters and a curve in others, and outlines may not be changed",
                        self.h.name
                    ));
                    return false;
                }
            }
        }
        true
    }

    /// Stems of the other masters from the main master's (`otherInstanceStems`). Stem
    /// conversion places each main-master stem in the other masters
    /// ([`crate::hinter::DimHinter::calc_instance_stems`]); a master whose stems it has not
    /// placed is placed here, then each master takes its stems.
    pub fn other_instance_stems(&mut self) {
        let masters = self.glyphs.len();
        if masters < 2 {
            return;
        }
        for dim in [Dim::H, Dim::V] {
            let mut d = self.dim(dim);
            let Some(stems) = d.st.hs.stems.as_mut() else { continue };
            if stems.len() < masters {
                stems.resize_with(masters, Vec::new);
            }
            let main_count = stems.first().map_or(0, Vec::len);
            let missing: Vec<usize> = (1..masters).filter(|&gi| main_count > 0 && stems.get(gi).is_some_and(Vec::is_empty)).collect();
            for gi in missing {
                d.calc_instance_stems(gi);
            }
        }
        for (i, g) in self.glyphs.others.iter_mut().enumerate() {
            let pick = |hs: &crate::state::GlyphHintState| hs.stems.as_ref().and_then(|s| s.get(i + 1)).cloned().unwrap_or_default();
            g.hstems = pick(&self.h.hs);
            g.vstems = pick(&self.v.hs);
        }
    }

    /// Masks of the other masters from the main master's (`otherInstanceMasks`): the same start
    /// mask, counter masks and element masks.
    pub fn other_instance_masks(&mut self) {
        if self.glyphs.others.is_empty() {
            return;
        }
        let main = &self.glyphs.main;
        let (cntr, start, is_hm) = (main.cntr.clone(), main.start_masks.clone(), main.is_hm);
        let masks: Vec<(PeRef, Option<crate::HintMask>)> = main.elements().map(|c| (c, main.element(c).and_then(|pe| pe.masks.clone()))).collect();
        for g in &mut self.glyphs.others {
            g.cntr = cntr.clone();
            g.start_masks = start.clone();
            g.is_hm = is_hm;
            for (c, m) in &masks {
                if let Some(pe) = g.element_mut(*c) {
                    pe.masks = m.clone();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::BezPath;

    use super::*;
    use crate::driver::tests::{rect, test_dict};
    use crate::{ElementPos, HintMask, Options, Stem};

    fn mask(h: &[bool], v: &[bool]) -> HintMask {
        HintMask { h: h.to_vec(), v: v.to_vec() }
    }

    fn staggered(dx: f64) -> [BezPath; 2] {
        [rect(100.0 + dx, 0.0, 180.0 + dx, 300.0), rect(140.0 + dx, 400.0, 220.0 + dx, 700.0)]
    }

    #[test]
    fn two_masters_get_the_same_stem_order_and_masks() {
        // otfautohint 5.0.1 (tests/oracle/driver.py, "staggered two masters").
        let (a, b) = (staggered(0.0), staggered(10.0));
        let dict = test_dict();
        let options = Options::default();
        let inputs = [
            HintInput { name: "staggered", contours: &a, without_overlap: None, dict: &dict, options: &options },
            HintInput { name: "staggered", contours: &b, without_overlap: None, dict: &dict, options: &options },
        ];
        let hints = hint_masters(&inputs).unwrap();
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0].h, vec![Stem::new(21.0, 0.0), Stem::new(700.0, 680.0)]);
        assert_eq!(hints[0].v, vec![Stem::new(100.0, 180.0), Stem::new(140.0, 220.0)]);
        assert_eq!(hints[1].h, vec![Stem::new(21.0, 0.0), Stem::new(700.0, 680.0)]);
        assert_eq!(hints[1].v, vec![Stem::new(110.0, 190.0), Stem::new(150.0, 230.0)]);
        for h in &hints {
            assert_eq!(h.start_mask, Some(mask(&[true, true], &[true, false])));
            assert_eq!(h.masks, vec![(ElementPos { subpath: 1, element: 3 }, mask(&[true, true], &[false, true]))]);
        }
    }

    #[test]
    fn masters_with_different_contour_counts_are_incompatible() {
        let a = staggered(0.0);
        let b = [rect(100.0, 0.0, 180.0, 300.0)];
        let dict = test_dict();
        let options = Options::default();
        let inputs = [
            HintInput { name: "staggered", contours: &a, without_overlap: None, dict: &dict, options: &options },
            HintInput { name: "staggered", contours: &b, without_overlap: None, dict: &dict, options: &options },
        ];
        assert!(matches!(hint_masters(&inputs), Err(AutohintError::Incompatible { .. })));
    }

    #[test]
    fn a_line_against_a_curve_needs_allow_changes() {
        let a = [rect(100.0, 0.0, 180.0, 300.0)];
        let mut b = BezPath::new();
        b.move_to((100.0, 0.0));
        b.curve_to((127.0, 0.0), (153.0, 0.0), (180.0, 0.0));
        b.line_to((180.0, 300.0));
        b.line_to((100.0, 300.0));
        b.close_path();
        let b = [b];
        let dict = test_dict();
        let strict = Options::default();
        let inputs = |options| {
            [
                HintInput { name: "bar", contours: &a, without_overlap: None, dict: &dict, options },
                HintInput { name: "bar", contours: &b, without_overlap: None, dict: &dict, options },
            ]
        };
        assert!(matches!(hint_masters(&inputs(&strict)), Err(AutohintError::Incompatible { .. })));
        let lenient = Options { allow_changes: true, ..Options::default() };
        let hints = hint_masters(&inputs(&lenient)).unwrap();
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0].v.len(), hints[1].v.len());
        assert_eq!(hints[0].h.len(), hints[1].h.len());
    }

    #[test]
    fn a_master_without_contours_gets_empty_hints() {
        let a = staggered(0.0);
        let dict = test_dict();
        let options = Options::default();
        let inputs = [
            HintInput { name: "staggered", contours: &a, without_overlap: None, dict: &dict, options: &options },
            HintInput { name: "staggered", contours: &[], without_overlap: None, dict: &dict, options: &options },
        ];
        let hints = hint_masters(&inputs).unwrap();
        assert_eq!(hints[0].v, vec![Stem::new(100.0, 180.0), Stem::new(140.0, 220.0)]);
        assert!(hints[1].is_empty());
    }
}
