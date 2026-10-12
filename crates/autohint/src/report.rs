//! Stem and zone reports: which stem widths and zone positions the glyphs suggest.
//!
//! Ported from otfautohint's `report.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`). Rows: `autohint.ps.reportStems`, `autohint.ps.reportZones` (the engine side is
//! `engine/src/cmd/autohint_report.rs`).
//!
//! The analysis is the hinter's own, run up to its stem values: the hinter records into the
//! [`GlyphReport`] of each dimension ([`crate::hinter::DimState::report`]) when
//! [`crate::Options::just_reporting`] is set, and this module adds up the glyphs. The
//! overlap-free outline of [`HintInput::without_overlap`] is not used here.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::glyph::GlyphData;
use crate::hinter::{GlyphHinter, Glyphs};
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

/// The glyph otfautohint leaves out of its reports.
const NOTDEF: &str = ".notdef";

/// Python's `round` in `Report.round_value`: halves away from zero.
fn round_value(v: f64) -> i64 {
    if v >= 0.0 { (v + 0.5) as i64 } else { (v - 0.5) as i64 }
}

/// A thousandths key back in font units, rounded as the report rounds.
fn units(thousandths: i64) -> i64 {
    round_value(thousandths as f64 / 1000.0)
}

impl GlyphReport {
    /// Adds what another dimension's hinter recorded for the same glyph (otfautohint shares one
    /// report between its two hinters).
    fn absorb(&mut self, other: GlyphReport) {
        for (pos, stems, from) in
            [(&mut self.hstems_pos, &mut self.hstems, other.hstems_pos), (&mut self.vstems_pos, &mut self.vstems, other.vstems_pos)]
        {
            for pair in from {
                if pos.insert(pair) {
                    *stems.entry(pair.1 - pair.0).or_insert(0) += 1;
                }
            }
        }
        self.char_zones.extend(other.char_zones);
        self.stem_zone_stems.extend(other.stem_zone_stems);
    }
}

/// Runs the analysis of one glyph as a report run of otfautohint does (`ReportOptions`): flex
/// off, the glyph's own hints replaced, zones and stems not required, nothing hinted. `None`
/// for `.notdef` and for a glyph without contours.
fn analyse(input: &HintInput, zones: bool) -> Result<Option<GlyphReport>, AutohintError> {
    if input.name == NOTDEF || input.contours.is_empty() {
        return Ok(None);
    }
    let mut options = input.options.clone();
    options.report_zones = zones;
    options.report_stems = !zones;
    options.hint_all = true;
    options.no_flex = true;
    options.allow_no_blues = true;
    let all_stems = options.report_all_stems;
    let main = GlyphData::from_contours(input.name, input.contours, options.round_coords)?;
    let mut hinter = GlyphHinter::new(Arc::new(options));
    hinter.glyphs = Glyphs { main, ..Default::default() };
    hinter.hint(input.name, vec![Arc::new(input.dict.clone())])?;
    let mut report = GlyphReport::new(input.name, all_stems);
    for part in [hinter.h.report.take(), hinter.v.report.take()].into_iter().flatten() {
        report.absorb(part);
    }
    Ok(Some(report))
}

/// The reports of every glyph in input order; a name given twice keeps its first place and its
/// last report, as the original's dictionary of glyphs does.
fn analyse_all(glyphs: &[HintInput], zones: bool) -> Result<Vec<GlyphReport>, AutohintError> {
    let mut out: Vec<GlyphReport> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for input in glyphs {
        let Some(report) = analyse(input, zones)? else { continue };
        match index.get(input.name).and_then(|&i| out.get_mut(i)) {
            Some(slot) => *slot = report,
            None => {
                index.insert(input.name.to_string(), out.len());
                out.push(report);
            }
        }
    }
    Ok(out)
}

/// Value (whole units) → (total count, the glyphs, by input index).
type Tally = BTreeMap<i64, (usize, BTreeSet<usize>)>;

fn add(tally: &mut Tally, value: i64, count: usize, glyph: usize) {
    let e = tally.entry(value).or_default();
    e.0 += count;
    e.1.insert(glyph);
}

/// `assemble_rep_list`: (count, value, glyph names in input order).
fn rows(tally: Tally, reports: &[GlyphReport]) -> Vec<(usize, i64, Vec<String>)> {
    tally
        .into_iter()
        .map(|(value, (count, glyphs))| (count, value, glyphs.into_iter().filter_map(|i| reports.get(i).map(|r| r.name.clone())).collect()))
        .collect()
}

/// The stem report of `glyphs` (`Report._get_lists` with stems): each glyph's stem widths
/// rounded to whole units, then per width the number of stems and the glyphs that have it, most
/// frequent first (then widest first). Curved stems count when the glyphs' options set
/// [`crate::Options::report_all_stems`]. `.notdef` is left out.
pub fn report_stems(glyphs: &[HintInput]) -> Result<StemReport, AutohintError> {
    let reports = analyse_all(glyphs, false)?;
    let (mut h, mut v) = (Tally::new(), Tally::new());
    for (i, r) in reports.iter().enumerate() {
        for (tally, stems) in [(&mut h, &r.hstems), (&mut v, &r.vstems)] {
            let mut widths: BTreeMap<i64, usize> = BTreeMap::new();
            for (width, count) in stems {
                *widths.entry(units(*width)).or_insert(0) += count;
            }
            for (width, count) in widths {
                add(tally, width, count, i);
            }
        }
    }
    let sorted = |tally: Tally| {
        let mut list = rows(tally, &reports);
        list.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)));
        list.into_iter().map(|(count, width, glyphs)| StemRow { width: width as f64, count, glyphs }).collect()
    };
    Ok(StemReport { h: sorted(h), v: sorted(v) })
}

/// The zone report of `glyphs` (`Report._get_lists` with zones): the top and bottom edges of
/// each glyph's extent and of its horizontal stems, rounded to whole units, then per position
/// the number of times it occurs and the glyphs; top zones highest first, bottom zones lowest
/// first. `.notdef` is left out.
pub fn report_zones(glyphs: &[HintInput]) -> Result<ZoneReport, AutohintError> {
    let reports = analyse_all(glyphs, true)?;
    let (mut top, mut bottom) = (Tally::new(), Tally::new());
    for (i, r) in reports.iter().enumerate() {
        let (mut tops, mut bottoms): (BTreeMap<i64, usize>, BTreeMap<i64, usize>) = (BTreeMap::new(), BTreeMap::new());
        for pair in r.char_zones.union(&r.stem_zone_stems) {
            *tops.entry(units(pair.1)).or_insert(0) += 1;
            *bottoms.entry(units(pair.0)).or_insert(0) += 1;
        }
        for (pos, count) in tops {
            add(&mut top, pos, count, i);
        }
        for (pos, count) in bottoms {
            add(&mut bottom, pos, count, i);
        }
    }
    let to_rows =
        |list: Vec<(usize, i64, Vec<String>)>| list.into_iter().map(|(count, pos, glyphs)| ZoneRow { position: pos as f64, count, glyphs }).collect();
    let mut tops = rows(top, &reports);
    tops.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)).then_with(|| a.2.cmp(&b.2)));
    let mut bottoms = rows(bottom, &reports);
    bottoms.sort_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)).then_with(|| a.2.cmp(&b.2)));
    Ok(ZoneReport { top: to_rows(tops), bottom: to_rows(bottoms) })
}

/// One list as `Report.save` writes it to its file: a title, a header line, then one line per
/// row; nothing for an empty list.
fn write_section(out: &mut String, title: &str, header: &str, rows: impl Iterator<Item = (usize, f64, String)>) {
    let mut body = String::new();
    for (count, value, glyphs) in rows {
        body.push_str(&format!("{count:5}    {:5}    [{glyphs}]\n", value as i64));
    }
    if body.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(title);
    out.push('\n');
    out.push_str(header);
    out.push_str(&body);
}

/// The report as otfautohint writes it (`Report.save`): the horizontal list, then the vertical
/// one, in one text (the original writes each to its own file, with the font's path and the
/// time in the title).
pub fn write_stem_report(report: &StemReport) -> String {
    let mut out = String::new();
    for (title, rows) in [("Horizontal Stem List", &report.h), ("Vertical Stem List", &report.v)] {
        write_section(&mut out, title, "count    width    glyphs\n", rows.iter().map(|r| (r.count, r.width, r.glyphs.join(" "))));
    }
    out
}

/// The zone report as otfautohint writes it (`Report.save`): top zones, then bottom zones.
pub fn write_zone_report(report: &ZoneReport) -> String {
    let mut out = String::new();
    for (title, rows) in [("Top Zone List", &report.top), ("Bottom Zone List", &report.bottom)] {
        write_section(&mut out, title, "count   height    glyphs\n", rows.iter().map(|r| (r.count, r.position, r.glyphs.join(" "))));
    }
    out
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::BezPath;

    use super::*;
    use crate::{FdDict, Options, StemSet, Zone, ZoneSet};

    /// `l`: one stem; `H`: two stems and a bar; `o`: an oval with a counter (curved stems only);
    /// `.notdef`: left out of every report.
    fn glyphs() -> Vec<(&'static str, Vec<BezPath>)> {
        let poly = |pts: &[(f64, f64)]| {
            let mut p = BezPath::new();
            for (i, pt) in pts.iter().enumerate() {
                if i == 0 { p.move_to(*pt) } else { p.line_to(*pt) }
            }
            p.close_path();
            p
        };
        let curves = |start: (f64, f64), segs: &[[(f64, f64); 3]]| {
            let mut p = BezPath::new();
            p.move_to(start);
            for [a, b, c] in segs {
                p.curve_to(*a, *b, *c);
            }
            p.close_path();
            p
        };
        let outer = curves(
            (250.0, -10.0),
            &[
                [(360.0, -10.0), (450.0, 106.0), (450.0, 250.0)],
                [(450.0, 394.0), (360.0, 510.0), (250.0, 510.0)],
                [(140.0, 510.0), (50.0, 394.0), (50.0, 250.0)],
                [(50.0, 106.0), (140.0, -10.0), (250.0, -10.0)],
            ],
        );
        let inner = curves(
            (250.0, 60.0),
            &[
                [(184.0, 60.0), (130.0, 145.0), (130.0, 250.0)],
                [(130.0, 355.0), (184.0, 440.0), (250.0, 440.0)],
                [(316.0, 440.0), (370.0, 355.0), (370.0, 250.0)],
                [(370.0, 145.0), (316.0, 60.0), (250.0, 60.0)],
            ],
        );
        vec![
            (".notdef", vec![poly(&[(50.0, 0.0), (450.0, 0.0), (450.0, 700.0), (50.0, 700.0)])]),
            ("l", vec![poly(&[(100.0, 0.0), (180.0, 0.0), (180.0, 700.0), (100.0, 700.0)])]),
            (
                "H",
                vec![poly(&[
                    (100.0, 0.0),
                    (180.0, 0.0),
                    (180.0, 330.0),
                    (420.0, 330.0),
                    (420.0, 0.0),
                    (500.0, 0.0),
                    (500.0, 700.0),
                    (420.0, 700.0),
                    (420.0, 400.0),
                    (180.0, 400.0),
                    (180.0, 700.0),
                    (100.0, 700.0),
                ])],
            ),
            ("o", vec![outer, inner]),
        ]
    }

    fn dict() -> FdDict {
        let zone = |bottom, top| Zone { bottom, top, is_bottom: false, name: String::new() };
        let zones = ZoneSet { blue_values: vec![zone(-12.0, 0.0), zone(500.0, 512.0), zone(700.0, 712.0)], blue_fuzz: 1.0, ..ZoneSet::default() };
        FdDict::new("test", &zones, &StemSet { h: vec![70.0], v: vec![80.0] }, false).unwrap()
    }

    fn inputs<'a>(glyphs: &'a [(&'static str, Vec<BezPath>)], dict: &'a FdDict, options: &'a Options) -> Vec<HintInput<'a>> {
        glyphs.iter().map(|(name, contours)| HintInput { name, contours, without_overlap: None, dict, options }).collect()
    }

    fn stem(width: f64, count: usize, glyphs: &[&str]) -> StemRow {
        StemRow { width, count, glyphs: glyphs.iter().map(|g| g.to_string()).collect() }
    }

    fn zone(position: f64, count: usize, glyphs: &[&str]) -> ZoneRow {
        ZoneRow { position, count, glyphs: glyphs.iter().map(|g| g.to_string()).collect() }
    }

    // Expected values: otfautohint 5.0.1's report mode (`glyphHinter` with `ReportOptions`, then
    // `Report._get_lists`) on the same outlines and zones.

    #[test]
    fn report_stems_counts_straight_stems_and_curved_ones_on_request() {
        let (g, d) = (glyphs(), dict());
        let options = Options::default();
        let r = report_stems(&inputs(&g, &d, &options)).unwrap();
        assert_eq!(r, StemReport { h: vec![stem(70.0, 1, &["H"])], v: vec![stem(80.0, 3, &["l", "H"])] });
        assert_eq!(
            write_stem_report(&r),
            "Horizontal Stem List\ncount    width    glyphs\n    1       70    [H]\n\nVertical Stem List\ncount    width    glyphs\n    3       80    [l H]\n"
        );
        let curved = Options { report_all_stems: true, ..Options::default() };
        let r = report_stems(&inputs(&g, &d, &curved)).unwrap();
        assert_eq!(r, StemReport { h: vec![stem(70.0, 3, &["H", "o"])], v: vec![stem(80.0, 5, &["l", "H", "o"])] });
    }

    #[test]
    fn report_zones_lists_glyph_extents_and_bar_edges() {
        let (g, d) = (glyphs(), dict());
        let options = Options::default();
        let r = report_zones(&inputs(&g, &d, &options)).unwrap();
        assert_eq!(r.top, vec![zone(700.0, 2, &["l", "H"]), zone(510.0, 2, &["o"]), zone(400.0, 1, &["H"]), zone(60.0, 1, &["o"])]);
        assert_eq!(r.bottom, vec![zone(-10.0, 2, &["o"]), zone(0.0, 2, &["l", "H"]), zone(330.0, 1, &["H"]), zone(440.0, 1, &["o"])]);
        let text = write_zone_report(&r);
        assert!(text.starts_with("Top Zone List\ncount   height    glyphs\n    2      700    [l H]\n"), "{text}");
        assert_eq!(write_zone_report(&ZoneReport::default()), "", "empty lists are not written");
    }
}
