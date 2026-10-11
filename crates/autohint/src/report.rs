//! Stem and zone reports: which stem widths and zone positions the glyphs suggest.
//!
//! Ported from otfautohint's `report.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`). Package M10.13 fills the bodies marked `// M10.13:`. Rows: `autohint.ps.report*`
//! (the engine side is `engine/src/cmd/autohint_report.rs`).

use std::collections::{BTreeMap, BTreeSet};

use crate::{AutohintError, HintInput};

/// A location pair as an ordered key: floats are kept as their bits after rounding to 1/1000.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pair(pub i64, pub i64);

impl Pair {
    pub fn new(l: f64, u: f64) -> Self {
        let k = |v: f64| if v.is_finite() { (v * 1000.0).round() as i64 } else { 0 };
        Self(k(l), k(u))
    }
}

/// What one glyph contributes to the reports (`GlyphReport`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphReport {
    pub name: String,
    /// Stem width (thousandths) → count, per direction.
    pub hstems: BTreeMap<i64, usize>,
    pub vstems: BTreeMap<i64, usize>,
    pub hstems_pos: BTreeSet<Pair>,
    pub vstems_pos: BTreeSet<Pair>,
    pub char_zones: BTreeSet<Pair>,
    pub stem_zone_stems: BTreeSet<Pair>,
    /// Count curved stems too (`all_stems`).
    pub all_stems: bool,
}

impl GlyphReport {
    pub fn new(name: &str, all_stems: bool) -> Self {
        Self { name: name.to_string(), all_stems, ..Self::default() }
    }

    /// `charZone`.
    pub fn char_zone(&mut self, l: f64, u: f64) {
        self.char_zones.insert(Pair::new(l, u));
    }

    /// `stemZone`.
    pub fn stem_zone(&mut self, l: f64, u: f64) {
        self.stem_zone_stems.insert(Pair::new(l, u));
    }

    /// `stem`: counts a stem of each position once.
    pub fn stem(&mut self, l: f64, u: f64, is_line: bool, is_v: bool) {
        if !is_line && !self.all_stems {
            return;
        }
        let (stems, pos) = if is_v { (&mut self.vstems, &mut self.vstems_pos) } else { (&mut self.hstems, &mut self.hstems_pos) };
        let pair = Pair::new(l, u);
        if pos.insert(pair) {
            *stems.entry(pair.1 - pair.0).or_insert(0) += 1;
        }
    }
}

/// One row of a stem report: width, number of glyphs, and the glyphs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StemRow {
    pub width: f64,
    pub count: usize,
    pub glyphs: Vec<String>,
}

/// A stem report (`report_stems`): horizontal and vertical stems, most frequent first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StemReport {
    pub h: Vec<StemRow>,
    pub v: Vec<StemRow>,
}

/// One row of a zone report: top or bottom position, number of glyphs, and the glyphs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneRow {
    pub position: f64,
    pub count: usize,
    pub glyphs: Vec<String>,
}

/// A zone report (`report_zones`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneReport {
    pub top: Vec<ZoneRow>,
    pub bottom: Vec<ZoneRow>,
}

/// The stem report of `glyphs` (`Report._get_lists` with stems).
pub fn report_stems(glyphs: &[HintInput]) -> Result<StemReport, AutohintError> {
    // M10.13: run the analysis per glyph, then Report.parse_stem_dict / assemble_rep_list.
    let _ = glyphs;
    Ok(StemReport::default())
}

/// The zone report of `glyphs` (`Report._get_lists` with zones).
pub fn report_zones(glyphs: &[HintInput]) -> Result<ZoneReport, AutohintError> {
    // M10.13: Report.parse_zone_dicts.
    let _ = glyphs;
    Ok(ZoneReport::default())
}

/// The report as otfautohint writes it to a file (`Report.save`).
pub fn write_stem_report(report: &StemReport) -> String {
    // M10.13: Report.save.
    let _ = report;
    String::new()
}

/// The zone report as otfautohint writes it (`Report.save`).
pub fn write_zone_report(report: &ZoneReport) -> String {
    // M10.13: Report.save.
    let _ = report;
    String::new()
}
