//! Variable fonts and static instances compared with a reference compiler's (M9.10).
//!
//! A variable font is compared by what its variation tables give at a set of locations
//! ([`Space::locations`]): the masters, the virtual masters, the static instances inside the
//! axis ranges, and the midpoint between each two neighbouring masters on each axis (the other
//! axes at their defaults). At each location both fonts are read through skrifa:
//!
//! | Check | What is compared |
//! |---|---|
//! | `outlines` | every glyph both fonts have, drawn at the location, within half a unit, start points free |
//! | `advances` | the advance width (`HVAR`, or the `gvar` phantom points without it) |
//! | `phantom` | the advance width the outline scaler gives from the phantom points |
//! | `MVAR` | each value tag's delta; a tag one font lacks counts as 0 |
//! | `GSUB variations` | the single substitutions the feature variations put in place |
//! | `GPOS kerning` | pair adjustments with their device deltas |
//! | `GPOS marks` | mark attachment offsets with their device deltas |
//!
//! `fvar`, `avar` and `STAT` are compared as structures, record by record, names resolved.
//! Difference keys end in `at <location>`, so an allow-list entry can name a location or use
//! `*` for every one.
//!
//! Static instances are separate static fonts: each is compared with the reference's instance
//! by the static comparison ([`crate::compare`]), the instance's name appended to the source's
//! label.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use skrifa::instance::{Location, Size};
use skrifa::metrics::GlyphMetrics;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::stat::AxisValue;
use skrifa::raw::types::{GlyphId, NameId};
use skrifa::{FontRef, MetadataProvider};

use crate::evaluate::{self, Names};
use crate::outline::{self, Match};
use crate::{Difference, GlyphCounts, Ours, OursError, Outcome, Settings, SourceReport, Status, TableResult, compare, tools};

/// Locations evaluated per source; the rest are left out with a note.
pub const MAX_LOCATIONS: usize = 64;
/// Glyphs drawn per location; the rest are left out with a note.
pub const MAX_GLYPHS: usize = 6_000;
/// Static instances compared per source.
pub const MAX_INSTANCES: usize = 128;
/// Axes read from a font or a source.
pub const MAX_AXES: usize = 64;
/// Records read from `fvar`, `avar` and `STAT`.
const MAX_RECORDS: usize = 10_000;
/// Two coordinates closer than this are one location.
const SAME: f64 = 1e-6;
/// Values closer than this are equal (half a font unit).
const TOLERANCE: f64 = 0.5;

/// One axis in user coordinates.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Axis {
    pub tag: String,
    pub minimum: f64,
    pub default: f64,
    pub maximum: f64,
}

/// Why a location is evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Master,
    VirtualMaster,
    Instance,
    Midpoint,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Master => "master",
            Kind::VirtualMaster => "virtual master",
            Kind::Instance => "instance",
            Kind::Midpoint => "midpoint",
        }
    }
}

/// A location to evaluate, in user coordinates, one value per axis of its [`Space`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Place {
    pub kind: Kind,
    pub name: String,
    pub coordinates: Vec<(String, f64)>,
}

impl Place {
    /// `master Bold (wght=700)`.
    pub fn label(&self) -> String {
        let coords: Vec<String> = self.coordinates.iter().map(|(t, v)| format!("{}={}", t.trim_end(), number(*v))).collect();
        let name = if self.name.is_empty() { String::new() } else { format!(" {}", self.name) };
        format!("{}{name} ({})", self.kind.name(), coords.join(","))
    }
}

fn number(v: f64) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if r == r.trunc() && r.abs() < 1e15 { format!("{}", r as i64) } else { format!("{r}") }
}

/// A source's designspace as FontCraft reads it, in user coordinates: what the locations are
/// made from. Each location has one value per axis.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Space {
    pub axes: Vec<Axis>,
    pub masters: Vec<(String, Vec<f64>)>,
    pub virtual_masters: Vec<(String, Vec<f64>)>,
    pub instances: Vec<(String, Vec<f64>)>,
}

impl Space {
    /// The axes and named instances of a variable font, with its default and axis ends as
    /// masters: the locations when FontCraft could not read the source.
    pub fn of_font(font: &FontRef) -> Space {
        let axes: Vec<Axis> = font
            .axes()
            .iter()
            .take(MAX_AXES)
            .map(|a| Axis {
                tag: a.tag().to_string(),
                minimum: f64::from(a.min_value()),
                default: f64::from(a.default_value()),
                maximum: f64::from(a.max_value()),
            })
            .collect();
        let instances = font
            .named_instances()
            .iter()
            .take(MAX_INSTANCES)
            .map(|i| (english(font, i.subfamily_name_id()), i.user_coords().take(MAX_AXES).map(f64::from).collect()))
            .collect();
        // Without the masters, the default and each axis's ends stand in for them.
        let defaults: Vec<f64> = axes.iter().map(|a| a.default).collect();
        let mut masters = vec![("default".to_string(), defaults.clone())];
        for (k, a) in axes.iter().enumerate() {
            for (end, v) in [("minimum", a.minimum), ("maximum", a.maximum)] {
                let mut c = defaults.clone();
                if let Some(slot) = c.get_mut(k) {
                    *slot = v;
                }
                masters.push((format!("{} {end}", a.tag.trim_end()), c));
            }
        }
        Space { axes, masters, instances, ..Space::default() }
    }

    fn inside(&self, coords: &[f64]) -> bool {
        coords.len() == self.axes.len()
            && coords
                .iter()
                .zip(&self.axes)
                .all(|(v, a)| v.is_finite() && *v >= a.minimum.min(a.maximum) - SAME && *v <= a.maximum.max(a.minimum) + SAME)
    }

    /// The locations to evaluate, in order: masters, virtual masters, instances inside the axis
    /// ranges, midpoints. A location equal to an earlier one is left out. Returns the locations
    /// and how many were left out for [`MAX_LOCATIONS`] or for lying outside the axes.
    pub fn locations(&self) -> (Vec<Place>, usize) {
        let mut out: Vec<Place> = Vec::new();
        let mut dropped = 0usize;
        let mut push = |out: &mut Vec<Place>, kind: Kind, name: &str, coords: &[f64]| {
            if !self.inside(coords) {
                dropped += 1;
                return;
            }
            if out.iter().any(|p| p.coordinates.iter().zip(coords).all(|((_, a), b)| (a - b).abs() <= SAME)) {
                return;
            }
            if out.len() >= MAX_LOCATIONS {
                dropped += 1;
                return;
            }
            let coordinates = self.axes.iter().zip(coords).map(|(a, v)| (a.tag.clone(), *v)).collect();
            out.push(Place { kind, name: name.to_string(), coordinates });
        };
        for (name, c) in &self.masters {
            push(&mut out, Kind::Master, name, c);
        }
        for (name, c) in &self.virtual_masters {
            push(&mut out, Kind::VirtualMaster, name, c);
        }
        for (name, c) in &self.instances {
            push(&mut out, Kind::Instance, name, c);
        }
        let defaults: Vec<f64> = self.axes.iter().map(|a| a.default).collect();
        for k in 0..self.axes.len() {
            let mut values: Vec<f64> =
                self.masters.iter().chain(&self.virtual_masters).filter_map(|(_, c)| c.get(k).copied()).filter(|v| v.is_finite()).collect();
            values.sort_by(f64::total_cmp);
            values.dedup_by(|a, b| (*a - *b).abs() <= SAME);
            for pair in values.windows(2) {
                if let [a, b] = pair {
                    let mut c = defaults.clone();
                    if let Some(slot) = c.get_mut(k) {
                        *slot = (a + b) / 2.0;
                    }
                    push(&mut out, Kind::Midpoint, "", &c);
                }
            }
        }
        (out, dropped)
    }
}

fn english(font: &FontRef, id: impl Into<NameId>) -> String {
    let id: NameId = id.into();
    font.localized_strings(id).english_or_first().map(|s| s.to_string()).unwrap_or_else(|| format!("name {}", id.to_u16()))
}

/// One font's side of a variable comparison.
struct Side<'a> {
    font: FontRef<'a>,
    names: Names,
    by_name: HashMap<String, GlyphId>,
}

impl<'a> Side<'a> {
    fn new(bytes: &'a [u8]) -> Result<Side<'a>, String> {
        let font = FontRef::new(bytes).map_err(|e| e.to_string())?;
        let mut names = Names::new();
        let mut by_name = HashMap::new();
        for (gid, name) in skrifa::GlyphNames::new(&font).iter() {
            let name = name.to_string();
            names.insert(gid.to_u32(), name.clone());
            by_name.entry(name).or_insert(gid);
        }
        Ok(Side { font, names, by_name })
    }

    fn location(&self, place: &Place) -> Location {
        self.font.axes().location(place.coordinates.iter().map(|(t, v)| (t.as_str(), *v as f32)))
    }
}

/// One check's differences, collected over the locations.
#[derive(Default)]
struct Check {
    differences: Vec<Difference>,
    count: usize,
    glyphs: Option<GlyphCounts>,
}

impl Check {
    fn push(&mut self, key: String, ours: String, theirs: String) {
        self.count += 1;
        if self.differences.len() < compare::MAX_KEPT {
            self.differences.push(Difference { key, ours, theirs });
        }
    }

    fn into_result(self, tag: &str) -> TableResult {
        let status = if self.count == 0 { Status::Identical } else { Status::Different };
        TableResult {
            tag: tag.into(),
            status,
            differences: self.differences,
            count: self.count,
            note: None,
            allowed: Vec::new(),
            glyphs: self.glyphs,
        }
    }
}

/// The location checks in report order.
pub const CHECKS: &[&str] = &["outlines", "advances", "phantom", "MVAR", "GSUB variations", "GPOS kerning", "GPOS marks"];

fn values(v: &[f64]) -> String {
    match v {
        // A kerning value with only an x advance shows as one number.
        [0.0, 0.0, x, 0.0] => number(*x),
        _ => format!("[{}]", v.iter().map(|x| number(*x)).collect::<Vec<_>>().join(", ")),
    }
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= TOLERANCE)
}

/// Compares two numeric maps; an entry one map lacks is `missing` (`None`: reported absent).
fn compare_maps<const N: usize>(
    check: &mut Check,
    what: &str,
    at: &str,
    a: &BTreeMap<String, [f64; N]>,
    b: &BTreeMap<String, [f64; N]>,
    missing: Option<[f64; N]>,
) {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    for k in keys {
        let (x, y) = (a.get(k).copied().or(missing), b.get(k).copied().or(missing));
        match (x, y) {
            (Some(x), Some(y)) if close(&x, &y) => {}
            (x, y) => {
                let show = |v: Option<[f64; N]>| v.map(|v| values(&v)).unwrap_or_else(|| "absent".into());
                check.push(format!("{k} {what} at {at}"), show(x), show(y));
            }
        }
    }
}

/// The result of comparing two variable fonts.
pub struct VarComparison {
    pub tables: Vec<TableResult>,
    pub notes: Vec<String>,
    pub locations: usize,
}

/// Compares two variable fonts at the locations of `space` (the reference's axes and named
/// instances when it is `None`) and their `fvar`, `avar` and `STAT` structures.
pub fn compare_variable(ours: &[u8], theirs: &[u8], space: Option<&Space>) -> Result<VarComparison, String> {
    let a = Side::new(ours).map_err(|e| format!("FontCraft's font cannot be read: {e}"))?;
    let b = Side::new(theirs).map_err(|e| format!("the reference font cannot be read: {e}"))?;
    let fallback;
    let space = match space {
        Some(s) => s,
        None => {
            fallback = Space::of_font(&b.font);
            &fallback
        }
    };
    let (places, dropped) = space.locations();
    let mut notes = Vec::new();
    if dropped > 0 {
        notes.push(format!("{dropped} locations left out (outside the axes, or beyond {MAX_LOCATIONS})"));
    }
    // Glyphs both fonts have, in FontCraft's order.
    let mut common: Vec<(String, GlyphId, GlyphId)> = Vec::new();
    let mut ours_only = 0usize;
    let mut gids: Vec<(&u32, &String)> = a.names.iter().collect();
    gids.sort();
    for (gid, name) in gids {
        match b.by_name.get(name) {
            Some(g) => common.push((name.clone(), GlyphId::new(*gid), *g)),
            None => ours_only += 1,
        }
    }
    let theirs_only = b.names.len().saturating_sub(common.len());
    if ours_only + theirs_only > 0 {
        notes.push(format!("glyphs only FontCraft's font has: {ours_only}; only the reference has: {theirs_only} (not compared)"));
    }
    if common.len() > MAX_GLYPHS {
        notes.push(format!("{} glyphs compared per location, of {}", MAX_GLYPHS, common.len()));
        common.truncate(MAX_GLYPHS);
    }
    let mut checks: Vec<Check> = CHECKS.iter().map(|_| Check::default()).collect();
    let mut cut = BTreeSet::new();
    for place in &places {
        let at = place.label();
        let (la, lb) = (a.location(place), b.location(place));
        let (ca, cb) = (la.coords(), lb.coords());
        let (ma, mb) = (GlyphMetrics::new(&a.font, Size::unscaled(), &la), GlyphMetrics::new(&b.font, Size::unscaled(), &lb));
        let mut glyphs = GlyphCounts::default();
        for (name, ga, gb) in &common {
            let (da, db) = (outline::draw_at(&a.font, *ga, (&la).into()), outline::draw_at(&b.font, *gb, (&lb).into()));
            if let Some(c) = checks.get_mut(0) {
                glyphs.compared += 1;
                match (&da, &db) {
                    (Ok((x, _)), Ok((y, _))) => match outline::compare(&x.clone().unwrap_or_default(), &y.clone().unwrap_or_default()) {
                        Match::Same | Match::Rotated => glyphs.same += 1,
                        Match::Different { ours, theirs } => {
                            glyphs.different += 1;
                            c.push(format!("{name} outline at {at}"), ours, theirs);
                        }
                    },
                    (x, y) => {
                        glyphs.different += 1;
                        let err = |r: &Result<_, String>| r.as_ref().err().cloned().unwrap_or_else(|| "drawn".into());
                        c.push(format!("{name} outline at {at}"), err(x), err(y));
                    }
                }
            }
            if let (Some(x), Some(y), Some(c)) = (ma.advance_width(*ga), mb.advance_width(*gb), checks.get_mut(1))
                && (f64::from(x) - f64::from(y)).abs() > TOLERANCE
            {
                c.push(format!("{name} advance at {at}"), number(f64::from(x)), number(f64::from(y)));
            }
            if let (Ok((_, Some(x))), Ok((_, Some(y))), Some(c)) = (&da, &db, checks.get_mut(2))
                && (f64::from(*x) - f64::from(*y)).abs() > TOLERANCE
            {
                c.push(format!("{name} phantom advance at {at}"), number(f64::from(*x)), number(f64::from(*y)));
            }
        }
        if let Some(c) = checks.get_mut(0) {
            c.glyphs.get_or_insert_with(GlyphCounts::default).add(&glyphs);
        }
        let to1 = |m: BTreeMap<String, f64>| m.into_iter().map(|(k, v)| (k, [v])).collect::<BTreeMap<_, _>>();
        if let Some(c) = checks.get_mut(3) {
            compare_maps(c, "delta", &at, &to1(evaluate::mvar(&a.font, ca)), &to1(evaluate::mvar(&b.font, cb)), Some([0.0]));
        }
        if let Some(c) = checks.get_mut(4) {
            let (x, y) = (evaluate::substitutions(&a.font, ca, &a.names), evaluate::substitutions(&b.font, cb, &b.names));
            for line in x.symmetric_difference(&y) {
                let (o, t) = if x.contains(line) { ("present", "absent") } else { ("absent", "present") };
                c.push(format!("{line} at {at}"), o.into(), t.into());
            }
        }
        let ((ka, cut_a), (kb, cut_b)) = (evaluate::kerning(&a.font, ca, &a.names), evaluate::kerning(&b.font, cb, &b.names));
        if cut_a || cut_b {
            cut.insert("kerning");
        }
        if let Some(c) = checks.get_mut(5) {
            compare_maps(c, "kerning", &at, &ka, &kb, Some([0.0; 4]));
        }
        let ((xa, cut_a), (xb, cut_b)) = (evaluate::attachments(&a.font, ca, &a.names), evaluate::attachments(&b.font, cb, &b.names));
        if cut_a || cut_b {
            cut.insert("mark attachments");
        }
        if let Some(c) = checks.get_mut(6) {
            compare_maps(c, "attachment", &at, &xa, &xb, None);
        }
    }
    for what in cut {
        notes.push(format!("{what} cut at {} entries or {} steps per font and location", evaluate::MAX_ENTRIES, evaluate::MAX_WORK));
    }
    let mut tables: Vec<TableResult> = Vec::new();
    for (tag, s) in [("fvar", structure_fvar as fn(&FontRef) -> Option<BTreeMap<String, String>>), ("avar", structure_avar), ("STAT", structure_stat)]
    {
        tables.push(compare_structure(tag, s(&a.font), s(&b.font)));
    }
    tables.extend(CHECKS.iter().zip(checks).map(|(tag, c)| c.into_result(tag)));
    Ok(VarComparison { tables, notes, locations: places.len() })
}

fn compare_structure(tag: &str, a: Option<BTreeMap<String, String>>, b: Option<BTreeMap<String, String>>) -> TableResult {
    let mut check = Check::default();
    match (a, b) {
        (None, None) => {
            return TableResult {
                tag: tag.into(),
                status: Status::Skipped,
                differences: Vec::new(),
                count: 0,
                note: Some("neither font has it".into()),
                allowed: Vec::new(),
                glyphs: None,
            };
        }
        (Some(a), Some(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for k in keys {
                let (x, y) = (a.get(k), b.get(k));
                if x != y {
                    check.push(k.clone(), x.cloned().unwrap_or_else(|| "absent".into()), y.cloned().unwrap_or_else(|| "absent".into()));
                }
            }
        }
        (a, b) => {
            let show = |v: &Option<BTreeMap<String, String>>| if v.is_some() { "present" } else { "absent" }.to_string();
            check.push(format!("{tag} table"), show(&a), show(&b));
        }
    }
    check.into_result(tag)
}

fn structure_fvar(font: &FontRef) -> Option<BTreeMap<String, String>> {
    font.fvar().ok()?;
    let mut out = BTreeMap::new();
    let axes = font.axes();
    let tags: Vec<String> = axes.iter().take(MAX_AXES).map(|a| a.tag().to_string()).collect();
    for (k, a) in axes.iter().take(MAX_AXES).enumerate() {
        out.insert(
            format!("fvar axis {} {}", k, a.tag()),
            format!(
                "{} {} {}{} name {}",
                number(f64::from(a.min_value())),
                number(f64::from(a.default_value())),
                number(f64::from(a.max_value())),
                if a.is_hidden() { " hidden" } else { "" },
                english(font, a.name_id())
            ),
        );
    }
    for i in font.named_instances().iter().take(MAX_RECORDS) {
        let coords: Vec<String> = tags.iter().zip(i.user_coords()).map(|(t, v)| format!("{}={}", t.trim_end(), number(f64::from(v)))).collect();
        let ps = i.postscript_name_id().map(|id| format!(" postscript {}", english(font, id))).unwrap_or_default();
        out.insert(format!("fvar instance {}", english(font, i.subfamily_name_id())), format!("{}{ps}", coords.join(",")));
    }
    Some(out)
}

fn structure_avar(font: &FontRef) -> Option<BTreeMap<String, String>> {
    let avar = font.avar().ok()?;
    let tags: Vec<String> = font.axes().iter().take(MAX_AXES).map(|a| a.tag().to_string()).collect();
    let mut out = BTreeMap::new();
    for (k, map) in avar.axis_segment_maps().iter().take(MAX_AXES).enumerate() {
        let Ok(map) = map else { continue };
        let pairs: Vec<String> = map
            .axis_value_maps()
            .iter()
            .take(MAX_RECORDS)
            .map(|m| format!("{:.4}:{:.4}", m.from_coordinate().to_f32(), m.to_coordinate().to_f32()))
            .collect();
        // An identity map (only −1, 0 and 1 mapped to themselves) is no map.
        let identity = map.axis_value_maps().iter().all(|m| m.from_coordinate() == m.to_coordinate());
        if !identity {
            out.insert(format!("avar {}", tags.get(k).cloned().unwrap_or_else(|| format!("axis {k}"))), pairs.join(" "));
        }
    }
    Some(out)
}

fn structure_stat(font: &FontRef) -> Option<BTreeMap<String, String>> {
    let stat = font.stat().ok()?;
    let mut out = BTreeMap::new();
    let axes = stat.design_axes().ok().unwrap_or_default();
    let tag = |i: u16| axes.get(usize::from(i)).map(|a| a.axis_tag().to_string()).unwrap_or_else(|| format!("axis {i}"));
    for a in axes.iter().take(MAX_AXES) {
        out.insert(format!("STAT axis {}", a.axis_tag()), format!("{} ordering {}", english(font, a.axis_name_id()), a.axis_ordering()));
    }
    if let Some(id) = stat.elided_fallback_name_id() {
        out.insert("STAT elided fallback name".into(), english(font, id));
    }
    if let Some(Ok(values)) = stat.offset_to_axis_values() {
        for v in values.axis_values().iter().take(MAX_RECORDS).flatten() {
            let (key, extra) = match &v {
                AxisValue::Format1(f) => (format!("{}={}", tag(f.axis_index()), number(f.value().to_f64())), String::new()),
                AxisValue::Format2(f) => (
                    format!("{}={}", tag(f.axis_index()), number(f.nominal_value().to_f64())),
                    format!(" range {}..{}", number(f.range_min_value().to_f64()), number(f.range_max_value().to_f64())),
                ),
                AxisValue::Format3(f) => {
                    (format!("{}={}", tag(f.axis_index()), number(f.value().to_f64())), format!(" linked {}", number(f.linked_value().to_f64())))
                }
                AxisValue::Format4(f) => {
                    let parts: Vec<String> =
                        f.axis_values().iter().take(MAX_AXES).map(|r| format!("{}={}", tag(r.axis_index()), number(r.value().to_f64()))).collect();
                    (parts.join(","), String::new())
                }
            };
            out.insert(
                format!("STAT value {key}"),
                format!("format {} flags {} name {}{extra}", v.format(), v.flags().bits(), english(font, v.value_name_id())),
            );
        }
    }
    Some(out)
}

fn report(label: &str, settings: &Settings) -> SourceReport {
    SourceReport {
        source: label.to_string(),
        oracle: settings.oracle,
        flavour: settings.flavour,
        outcome: Outcome::Compared,
        messages: Vec::new(),
        reference: None,
        notes: Vec::new(),
        source_reading: Vec::new(),
        tables: Vec::new(),
    }
}

/// Compares `source`'s variable font built by FontCraft (`ours`) with the reference compiler's,
/// at the locations of `space` (FontCraft's reading of the source). When FontCraft does not
/// build the font yet, the outcome is "not compared" and the reference is still built and read
/// at every location, compared with itself, so the harness is known to work on it.
pub fn check_variable(
    source: &Path,
    label: &str,
    settings: &Settings,
    space: Option<&Space>,
    ours: impl FnOnce() -> Result<Ours, OursError>,
) -> SourceReport {
    let mut r = report(label, settings);
    let ours = ours();
    let hint = match &ours {
        Ok(o) => o.hint.clone(),
        Err(OursError::Unsupported { hint, .. }) => hint.clone(),
        Err(OursError::Failed(_)) => crate::Hint::default(),
    };
    let work = match crate::work_dir(settings, label) {
        Ok(w) => w,
        Err(e) => {
            r.outcome = Outcome::ReferenceFailed;
            r.messages.push(e);
            return r;
        }
    };
    let built = tools::build_variable(&request(settings, source, &work.path, &hint));
    r.outcome = match (&ours, &built) {
        (Err(OursError::Unsupported { .. }), _) => Outcome::NotCompared,
        (Ok(_), Ok(_)) => Outcome::Compared,
        (Ok(_), Err(_)) => Outcome::ReferenceFailed,
        (Err(_), Ok(_)) => Outcome::FontcraftFailed,
        (Err(_), Err(_)) => Outcome::BothFailed,
    };
    match &ours {
        Ok(o) => r.notes.extend(o.warnings.iter().map(|w| format!("FontCraft: {w}"))),
        Err(OursError::Unsupported { message: m, .. } | OursError::Failed(m)) => r.messages.push(format!("FontCraft: {m}")),
    }
    let reference = match built {
        Ok(b) => {
            r.notes.extend(b.notes.iter().cloned());
            let bytes = std::fs::read(&b.path).ok();
            r.reference = Some(b);
            bytes
        }
        Err(e) => {
            r.messages.push(e);
            None
        }
    };
    if let (Ok(o), Some(_)) = (&ours, &settings.keep) {
        let path = work.path.join(if settings.flavour == crate::Flavour::Cff { "fontcraft-variable.otf" } else { "fontcraft-variable.ttf" });
        if let Err(e) = std::fs::write(&path, &o.bytes) {
            r.notes.push(format!("cannot keep FontCraft's font at {}: {e}", path.display()));
        }
    }
    match (&ours, &reference) {
        (Ok(o), Some(theirs)) => match compare_variable(&o.bytes, theirs, space) {
            Ok(c) => {
                r.notes.push(format!("{} locations evaluated", c.locations));
                r.notes.extend(c.notes);
                r.tables = c.tables;
                crate::apply_allow(&mut r.tables, &settings.allow, settings.oracle.name(), label);
                for t in &mut r.tables {
                    t.differences.truncate(settings.shown);
                }
            }
            Err(e) => {
                r.outcome = Outcome::BothFailed;
                r.messages.push(e);
            }
        },
        (Err(OursError::Unsupported { .. }), Some(theirs)) => match compare_variable(theirs, theirs, space) {
            Ok(c) => {
                let differing: Vec<&str> = c.tables.iter().filter(|t| t.status == Status::Different).map(|t| t.tag.as_str()).collect();
                if differing.is_empty() {
                    r.notes.push(format!("the reference read at {} locations, equal to itself", c.locations));
                } else {
                    r.messages.push(format!("the reference differs from itself in {} (a harness error)", differing.join(", ")));
                }
            }
            Err(e) => r.messages.push(e),
        },
        (Ok(_), None) if r.outcome == Outcome::Compared => {
            r.outcome = Outcome::ReferenceFailed;
            r.messages.push("the reference font cannot be read back".into());
        }
        _ => {}
    }
    r
}

fn request<'a>(settings: &'a Settings, source: &'a Path, work: &'a Path, hint: &'a crate::Hint) -> tools::Request<'a> {
    tools::Request {
        oracle: settings.oracle,
        tool: &settings.tool,
        source,
        flavour: settings.flavour,
        work_dir: work,
        timestamp: settings.timestamp,
        timeout: settings.timeout,
        hint,
    }
}

/// A static instance to compare: its index in the source's instances and its name.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceJob {
    pub index: usize,
    pub name: String,
}

/// Compares each of `instances` built by FontCraft (`ours(index)`) with the reference's static
/// instance of the same name (fontmake `-i`), one report per instance labelled
/// `<label> › <instance>`. `family` names the reference's output files.
pub fn check_instances(
    source: &Path,
    label: &str,
    settings: &Settings,
    family: &str,
    instances: &[InstanceJob],
    mut ours: impl FnMut(usize) -> Result<Ours, OursError>,
) -> Vec<SourceReport> {
    let hint = crate::Hint { family: family.to_string(), style: String::new() };
    let work = crate::work_dir(settings, label);
    let built = match &work {
        Ok(w) => tools::build_instances(&request(settings, source, &w.path, &hint)),
        Err(e) => Err(e.clone()),
    };
    let mut out = Vec::new();
    for job in instances.iter().take(MAX_INSTANCES) {
        let name = format!("{label} › {}", job.name);
        let mut r = report(&name, settings);
        let ours = ours(job.index);
        let theirs = match &built {
            Ok(b) => match tools::instance_file(&b.files, family, &job.name) {
                Some(p) => {
                    r.reference = Some(tools::Built { path: p.clone(), command: b.command.clone(), notes: Vec::new(), alternatives: Vec::new() });
                    std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
                }
                None => {
                    let names: Vec<String> = b.files.iter().take(6).filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
                    Err(format!("fontmake wrote no file for instance `{}`; its {} files: {}", job.name, b.files.len(), names.join(", ")))
                }
            },
            Err(e) => Err(e.clone()),
        };
        r.outcome = match (&ours, &theirs) {
            (Err(OursError::Unsupported { .. }), _) => Outcome::NotCompared,
            (Ok(_), Ok(_)) => Outcome::Compared,
            (Ok(_), Err(_)) => Outcome::ReferenceFailed,
            (Err(_), Ok(_)) => Outcome::FontcraftFailed,
            (Err(_), Err(_)) => Outcome::BothFailed,
        };
        match &ours {
            Ok(o) => r.notes.extend(o.warnings.iter().map(|w| format!("FontCraft: {w}"))),
            Err(OursError::Unsupported { message: m, .. } | OursError::Failed(m)) => r.messages.push(format!("FontCraft: {m}")),
        }
        if let Err(e) = &theirs {
            r.messages.push(e.clone());
        }
        if let (Ok(o), Some(_), Ok(w)) = (&ours, &settings.keep, &work) {
            let path = w.path.join(format!("fontcraft-{}.ttf", crate::slug(&job.name)));
            if let Err(e) = std::fs::write(&path, &o.bytes) {
                r.notes.push(format!("cannot keep FontCraft's font at {}: {e}", path.display()));
            }
        }
        if let (Ok(o), Ok(t)) = (&ours, &theirs) {
            match compare::compare_with(&o.bytes, t, compare::Outlines::of(settings.oracle)) {
                Ok(c) => {
                    r.notes.extend(c.notes);
                    r.source_reading = c.source_reading;
                    r.tables = c.tables;
                    crate::apply_allow(&mut r.tables, &settings.allow, settings.oracle.name(), &name);
                    for t in &mut r.tables {
                        t.differences.truncate(settings.shown);
                    }
                }
                Err(e) => {
                    r.outcome = Outcome::BothFailed;
                    r.messages.push(e);
                }
            }
        }
        out.push(r);
    }
    if instances.len() > MAX_INSTANCES
        && let Some(last) = out.last_mut()
    {
        last.notes.push(format!("{} instances compared, of {}", MAX_INSTANCES, instances.len()));
    }
    out
}

/// How the sources or instances of a run came out, as the variable and instance measures count
/// them: compared and equal (every check identical, equivalent, coverage-equal or allowed),
/// compared with a difference, not built by FontCraft, and failed (a build or a read).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measure {
    pub equal: usize,
    pub different: usize,
    pub not_built: usize,
    pub failed: usize,
}

pub fn measure(reports: &[SourceReport]) -> Measure {
    let mut m = Measure::default();
    for r in reports {
        match r.outcome {
            Outcome::Compared if r.has_differences() => m.different += 1,
            Outcome::Compared => m.equal += 1,
            Outcome::NotCompared => m.not_built += 1,
            _ => m.failed += 1,
        }
    }
    m
}

pub fn render_measure(what: &str, m: &Measure) -> String {
    let mut out = String::new();
    let _ = write!(out, "{what}: {} equal, {} different, {} not built by FontCraft, {} failed", m.equal, m.different, m.not_built, m.failed);
    out
}

/// The files of a build, for [`tools::instance_file`].
pub(crate) fn ttf_files(dir: &Path, limit: usize) -> Vec<PathBuf> {
    font_files(dir, "ttf", limit)
}

/// The files of a build with `extension`, sorted.
pub(crate) fn font_files(dir: &Path, extension: &str, limit: usize) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| {
            it.take(limit).filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().and_then(|x| x.to_str()) == Some(extension)).collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}
