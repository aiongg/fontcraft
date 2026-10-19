//! The autohinter's read-only reports (`fontcraft_autohint::report`, `fontcraft_autohint::fontinfo`):
//! the stem widths and the zone positions the glyphs of a master suggest, and the zone-and-stem
//! sets the autohinter would use, with override sets from fontinfo text.
//!
//! Rows: `autohint.ps.reportStems`, `autohint.ps.reportZones`, `autohint.ps.listFdDicts`,
//! `autohint.ps.fontinfoOverrides*`.
//!
//! The reports run as otfautohint's report mode does: flex off, zones and stems not required,
//! `.notdef` left out. Each glyph is analysed as drawn in the master, components resolved, open
//! contours left out.

use std::collections::HashSet;

use fontcraft_autohint::fontinfo::{self, FontInfo};
use fontcraft_autohint::report::{self, StemReport, ZoneReport};
use fontcraft_autohint::{BlueValue, FdDict, HintInput, Options, StemSet, Zone, ZoneSet};
use fontcraft_geom::{BezPath, PathEl, Point};
use fontcraft_hinting::HintSettings;
use fontcraft_hinting::settings::DEFAULT_BLUE_FUZZ;
use fontcraft_model::Font;
use serde_json::{Value, json};

use super::*;
use crate::params::Params;

/// Standard stems read per direction, as the CFF Private dict holds them (StemSnapH/V).
const MAX_STEM_SNAP: usize = 12;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "autohint.ps.reportStems",
            "Report Stem Widths",
            [],
            None,
            "{glyphs?: [names] (default: every exporting glyph), excludeGlyphs?: [names], master?: id (default: the first master), includeCurved?: bool (count stems formed by curves too; default false)} the stem widths the autohinter finds, without hinting: per width (whole units) the number of stems and the glyphs that have it, most frequent first → {master, horizontal: [{width, count, glyphs}], vertical: [{width, count, glyphs}], skipped: [{glyph, reason}], warnings: [text] (a direction with more than 12 standard stems uses the first 12, as a CFF font keeps them), text: the report as text}",
            has_doc,
            report_stems
        ),
        cmd!(
            query "autohint.ps.reportZones",
            "Report Alignment Zones",
            [],
            None,
            "{glyphs?: [names] (default: every exporting glyph), excludeGlyphs?: [names], master?: id (default: the first master)} the top and bottom edges the autohinter finds (each glyph's extent and its horizontal stems), without hinting: per position (whole units) how often it occurs and the glyphs; top edges highest first, bottom edges lowest first → {master, top: [{height, count, glyphs}], bottom: [{height, count, glyphs}], skipped: [{glyph, reason}], warnings: [text] (as for the stem report), text}",
            has_doc,
            report_zones
        ),
        cmd!(
            query "autohint.ps.listFdDicts",
            "List Hint Dictionaries",
            [],
            None,
            "{fontinfo?: text (override sets: FDDict and GlyphSet blocks), master?: id (default: the first master), allInstances?: bool (every master)} the zone-and-stem sets the autohinter uses and the glyphs each one hints; with override sets, glyphs no set takes get the font's set with its zones moved out of reach (`No Alignment Zones`) → {dicts: [{master, name, fontName?, blueValues: [[bottom, top]], otherBlues, familyBlues, familyOtherBlues, blueFuzz, dominantH, dominantV, flexOK, languageGroup, vCounterChars, hCounterChars, glyphs, glyphCount}], final?: {the FinalFont set, same fields without glyphs}, warnings: [text] (a master with more than 12 standard stems in a direction: only the first 12 are used, as a CFF font keeps them)}",
            has_doc,
            list_fd_dicts
        ),
    ]
}

fn master_id(p: &Params, font: &Font) -> Result<String> {
    Ok(match p.opt_str("master")? {
        Some(m) => font.master(m)?.id.clone(),
        None => font.default_master()?.id.clone(),
    })
}

/// A master's zones in the autohinter's form, split as the CFF Private dict splits them: a zone
/// at 0 or with a positive size is a BlueValues zone, any other an OtherBlues zone.
fn zone_set(settings: &HintSettings) -> ZoneSet {
    let split = |zones: &[fontcraft_hinting::PsZone]| {
        let mut sorted: Vec<_> = zones.iter().filter(|z| z.position.is_finite() && z.size.is_finite()).collect();
        sorted.sort_by(|a, b| a.position.total_cmp(&b.position).then(a.size.total_cmp(&b.size)));
        let (mut blues, mut others) = (Vec::new(), Vec::new());
        for z in sorted {
            let zone = if z.size < 0.0 {
                Zone { bottom: z.position + z.size, top: z.position, is_bottom: true, name: String::new() }
            } else {
                Zone { bottom: z.position, top: z.position + z.size, is_bottom: false, name: String::new() }
            };
            if z.position == 0.0 || z.size > 0.0 { blues.push(zone) } else { others.push(zone) }
        }
        (blues, others)
    };
    let (blue_values, other_blues) = split(&settings.zones);
    let (family_blues, family_other_blues) = split(&settings.family_zones);
    ZoneSet { blue_values, other_blues, family_blues, family_other_blues, blue_fuzz: settings.blue_fuzz.unwrap_or(DEFAULT_BLUE_FUZZ) }
}

/// The font's set for `master` as otfautohint reads it from a font: zones and stems not
/// required (a direction without stems gets the em size), the em size and master name kept.
/// Also a warning for each direction with more stems than [`MAX_STEM_SNAP`], whose rest is not
/// used.
fn base_dict(p: &Params, font: &Font, master: &str) -> Result<(FdDict, Vec<String>)> {
    let settings = HintSettings::for_master(font, master).map_err(|e| p.bad(e.to_string()))?;
    let upm = f64::from(font.units_per_em);
    let mut warnings = Vec::new();
    let mut stems = |list: &[f64], direction: &str| {
        let finite: Vec<f64> = list.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.len() > MAX_STEM_SNAP {
            warnings.push(format!(
                "master `{master}` has {} {direction} stems; a CFF font keeps at most {MAX_STEM_SNAP}, so only the first {MAX_STEM_SNAP} are used",
                finite.len()
            ));
        }
        let list: Vec<f64> = finite.into_iter().take(MAX_STEM_SNAP).collect();
        if list.is_empty() { vec![upm] } else { list }
    };
    let stems = StemSet { h: stems(&settings.stems_h, "horizontal"), v: stems(&settings.stems_v, "vertical") };
    let name = font.master(master)?.name.clone();
    let mut dict = FdDict::new(&name, &zone_set(&settings), &stems, true).map_err(|e| p.bad(e.to_string()))?;
    dict.em_units = Some(upm);
    dict.font_name = Some(name);
    Ok((dict, warnings))
}

/// The closed contours of `path`, quadratic segments raised to cubics (the autohinter reads
/// cubic outlines only).
fn closed_cubic_contours(path: &BezPath) -> Vec<BezPath> {
    let mut out = Vec::new();
    let mut current: Vec<PathEl> = Vec::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                current.clear();
                current.push(PathEl::MoveTo(p));
                (start, last) = (p, p);
            }
            PathEl::LineTo(p) => {
                current.push(PathEl::LineTo(p));
                last = p;
            }
            PathEl::QuadTo(q, p) => {
                current.push(PathEl::CurveTo(last.lerp(q, 2.0 / 3.0), p.lerp(q, 2.0 / 3.0), p));
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                current.push(PathEl::CurveTo(a, b, p));
                last = p;
            }
            PathEl::ClosePath => {
                if current.len() > 1 {
                    current.push(PathEl::ClosePath);
                    out.push(BezPath::from_vec(std::mem::take(&mut current)));
                }
                current.clear();
                last = start;
            }
        }
    }
    out
}

/// The glyphs a report covers: those named, else every exporting glyph, in font order, less the
/// excluded ones.
fn glyph_list(p: &Params, font: &Font) -> Result<(Vec<String>, bool)> {
    let excluded: HashSet<&str> = p.opt_strings("excludeGlyphs")?.unwrap_or_default().into_iter().collect();
    let named = p.opt_glyphs("glyphs", font)?;
    let explicit = named.is_some();
    let list = named.unwrap_or_else(|| font.glyphs.iter().filter(|g| g.export).map(|g| g.name.clone()).collect());
    Ok((list.into_iter().filter(|g| !excluded.contains(&g.as_str())).collect(), explicit))
}

/// What the reports read: the master, each glyph's contours, and the glyphs left out with why.
struct Prepared {
    master: String,
    dict: FdDict,
    options: Options,
    glyphs: Vec<(String, Vec<BezPath>)>,
    skipped: Vec<Value>,
    warnings: Vec<String>,
}

fn prepare(p: &Params, font: &Font, include_curved: bool) -> Result<Prepared> {
    let master = master_id(p, font)?;
    let (names, explicit) = glyph_list(p, font)?;
    let (dict, warnings) = base_dict(p, font, &master)?;
    let options = Options { report_all_stems: include_curved, explicit_glyphs: explicit, ..Options::default() };
    let mut glyphs = Vec::with_capacity(names.len());
    let mut skipped = Vec::new();
    for name in names {
        match font.outline(&name, &master) {
            Ok(path) => glyphs.push((name, closed_cubic_contours(&path))),
            Err(e) => skipped.push(json!({"glyph": name, "reason": e.to_string()})),
        }
    }
    Ok(Prepared { master, dict, options, glyphs, skipped, warnings })
}

impl Prepared {
    fn inputs(&self) -> Vec<HintInput<'_>> {
        self.glyphs
            .iter()
            .map(|(name, contours)| HintInput { name, contours, without_overlap: None, dict: &self.dict, options: &self.options })
            .collect()
    }
}

fn stem_rows(rows: &[report::StemRow]) -> Value {
    Value::Array(rows.iter().map(|r| json!({"width": r.width, "count": r.count, "glyphs": r.glyphs})).collect())
}

fn zone_rows(rows: &[report::ZoneRow]) -> Value {
    Value::Array(rows.iter().map(|r| json!({"height": r.position, "count": r.count, "glyphs": r.glyphs})).collect())
}

fn report_stems(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("autohint.ps.reportStems", p)?;
    let include_curved = p.bool_or("includeCurved", false)?;
    let prepared = prepare(&p, s.font()?, include_curved)?;
    let r: StemReport = report::report_stems(&prepared.inputs()).map_err(|e| p.bad(e.to_string()))?;
    Ok(json!({
        "master": prepared.master,
        "horizontal": stem_rows(&r.h),
        "vertical": stem_rows(&r.v),
        "skipped": prepared.skipped,
        "warnings": prepared.warnings,
        "text": report::write_stem_report(&r),
    }))
}

fn report_zones(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("autohint.ps.reportZones", p)?;
    let prepared = prepare(&p, s.font()?, false)?;
    let r: ZoneReport = report::report_zones(&prepared.inputs()).map_err(|e| p.bad(e.to_string()))?;
    Ok(json!({
        "master": prepared.master,
        "top": zone_rows(&r.top),
        "bottom": zone_rows(&r.bottom),
        "skipped": prepared.skipped,
        "warnings": prepared.warnings,
        "text": report::write_zone_report(&r),
    }))
}

fn pairs(list: &[BlueValue]) -> Value {
    Value::Array(list.iter().map(|z| json!([z.bottom, z.top])).collect())
}

fn dict_json(master: &str, d: &FdDict) -> Value {
    json!({
        "master": master,
        "name": d.dict_name,
        "fontName": d.font_name,
        "blueValues": pairs(&d.blue_pairs),
        "otherBlues": pairs(&d.other_blue_pairs),
        "familyBlues": pairs(&d.family_pairs),
        "familyOtherBlues": pairs(&d.other_family_pairs),
        "blueFuzz": d.blue_fuzz,
        "dominantH": d.dominant_h,
        "dominantV": d.dominant_v,
        "flexOK": d.flex_ok,
        "languageGroup": d.language_group,
        "vCounterChars": d.v_counter_chars,
        "hCounterChars": d.h_counter_chars,
    })
}

fn list_fd_dicts(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("autohint.ps.listFdDicts", p)?;
    let font = s.font()?;
    let text = p.opt_str("fontinfo")?.unwrap_or("");
    let masters: Vec<String> =
        if p.bool_or("allInstances", false)? { font.masters.iter().map(|m| m.id.clone()).collect() } else { vec![master_id(&p, font)?] };
    let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
    let upm = f64::from(font.units_per_em);
    let mut dicts = Vec::new();
    let mut final_dict = None;
    let mut warnings = Vec::new();
    for master in &masters {
        let (base, cut) = base_dict(&p, font, master)?;
        warnings.extend(cut);
        // The extremes otfautohint takes for a source font: one em below the baseline, two above.
        let info: FontInfo = fontinfo::parse(text, &base, &names, 2.0 * upm, -upm).map_err(|e| p.bad(e.to_string()))?;
        for (i, d) in info.dicts.iter().enumerate() {
            let glyphs: Vec<&String> = names.iter().filter(|g| info.glyph_dict.get(*g).copied().unwrap_or(0) == i).collect();
            let mut entry = dict_json(master, d);
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("glyphCount".into(), json!(glyphs.len()));
                obj.insert("glyphs".into(), json!(glyphs));
            }
            dicts.push(entry);
        }
        if final_dict.is_none() {
            final_dict = info.final_dict.as_ref().map(|d| dict_json(master, d));
        }
    }
    let mut out = json!({"dicts": dicts, "warnings": warnings});
    if let (Some(f), Some(obj)) = (final_dict, out.as_object_mut()) {
        obj.insert("final".into(), f);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadratic_contours_are_raised_and_open_ones_dropped() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.quad_to((30.0, 60.0), (90.0, 0.0));
        path.close_path();
        path.move_to((200.0, 0.0));
        path.line_to((300.0, 0.0));
        let contours = closed_cubic_contours(&path);
        assert_eq!(contours.len(), 1, "the open contour is left out");
        let els = contours.first().map(|c| c.elements().to_vec()).unwrap_or_default();
        let Some(PathEl::CurveTo(a, b, c)) = els.get(1).copied() else { panic!("not a curve: {els:?}") };
        let near = |p: Point, x: f64, y: f64| (p.x - x).abs() < 1e-9 && (p.y - y).abs() < 1e-9;
        assert!(near(a, 20.0, 40.0) && near(b, 50.0, 40.0) && near(c, 90.0, 0.0), "{els:?}");
    }
}
