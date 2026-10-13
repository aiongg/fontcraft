//! The engine's side of the audit services (`fontcraft_audit::AuditServices`) and the three
//! shortcut commands that reproduce the reference tools with their own option sets
//! (`docs/spec/21-qa.md` § 6): `audit.checkOutlines`, `audit.checkInterpolation` and
//! `audit.compareFamily`. Each shortcut turns its options into one `audit.run` call.
//!
//! Services: the in-memory compile of a source's instances, with the source map that lets
//! binary findings name source glyphs and Font Info fields, and the master-compatibility report
//! (`fontcraft_audit::structural_report`). The TTX round trip, shaping and rasterising answer
//! "not available", which makes the checks that need them skip with the reason.

use fontcraft_audit::run::MAX_SOURCE_INSTANCES;
use fontcraft_audit::services::CompiledInstance;
use fontcraft_audit::{AuditServices, CompatibilityReport, SourceMap, Unavailable};
use fontcraft_compile::var::VariableOptions;
use fontcraft_compile::{Flavour, Options};
use fontcraft_model::{Font, Target};
use serde_json::{Map, Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// The services the engine gives the audit.
#[derive(Clone, Copy, Debug, Default)]
pub struct EngineServices;

impl AuditServices for EngineServices {
    fn compile_instances(&self, font: &Font) -> std::result::Result<Vec<CompiledInstance>, Unavailable> {
        let (instances, notes) = self.compile_all(font);
        if instances.is_empty() {
            return Err(Unavailable::new(notes.first().cloned().unwrap_or_else(|| "the source has nothing to compile".into())));
        }
        Ok(instances)
    }

    fn compatibility(&self, font: &Font) -> std::result::Result<CompatibilityReport, Unavailable> {
        Ok(fontcraft_audit::structural_report(font))
    }
}

impl EngineServices {
    /// Compiles each exporting static instance of `font` (the default master when it has none),
    /// at most [`MAX_SOURCE_INSTANCES`], as TrueType, and the variable font when the source has
    /// several masters. Returns what compiled and one note per thing that did not.
    pub fn compile_all(&self, font: &Font) -> (Vec<CompiledInstance>, Vec<String>) {
        let options = Options::default();
        let mut out = Vec::new();
        let mut notes = Vec::new();
        let mut targets: Vec<(String, Target)> = font
            .instances
            .iter()
            .enumerate()
            .filter(|(_, i)| i.kind.is_none() && i.exports)
            .map(|(n, i)| (if i.name.is_empty() { format!("#{n}") } else { i.name.clone() }, Target::Instance(n)))
            .collect();
        if targets.is_empty()
            && let Ok(m) = font.default_master()
        {
            targets.push((m.name.clone(), Target::Master(m.id.clone())));
        }
        if targets.len() > MAX_SOURCE_INSTANCES {
            notes.push(format!("the source exports {} instances; the binary checks read the first {MAX_SOURCE_INSTANCES}", targets.len()));
            targets.truncate(MAX_SOURCE_INSTANCES);
        }
        for (name, target) in targets {
            match fontcraft_compile::compile(font, &target, Flavour::TrueType, &options) {
                Ok(c) => out.push(CompiledInstance {
                    name: name.clone(),
                    format: "ttf",
                    variable: false,
                    bytes: c.bytes,
                    map: source_map(Some(name), c.glyph_order),
                }),
                Err(e) => notes.push(format!("the instance {name} could not be compiled, so the binary checks skip it: {e}")),
            }
        }
        if font.masters.len() > 1 {
            match fontcraft_compile::var::compile_variable(font, None, &options, &VariableOptions::default()) {
                Ok(v) => out.push(CompiledInstance {
                    name: v.file_stem.clone(),
                    format: "ttf",
                    variable: true,
                    bytes: v.compiled.bytes,
                    map: source_map(None, v.compiled.glyph_order),
                }),
                Err(e) => notes.push(format!("the variable font could not be compiled, so the binary checks skip it: {e}")),
            }
        }
        (out, notes)
    }
}

/// Compiled fields and the source field or custom parameter that produces each
/// (`audit.scopeResolution`). Keys are `table.field`; name records are `name.<ID>`.
const SOURCE_FIELDS: &[(&str, &str)] = &[
    ("OS/2.usWinAscent", "master.customParameters.winAscent"),
    ("OS/2.usWinDescent", "master.customParameters.winDescent"),
    ("OS/2.sTypoAscender", "master.customParameters.typoAscender"),
    ("OS/2.sTypoDescender", "master.customParameters.typoDescender"),
    ("OS/2.sTypoLineGap", "master.customParameters.typoLineGap"),
    ("OS/2.sxHeight", "master.xHeight"),
    ("OS/2.sCapHeight", "master.capHeight"),
    ("OS/2.yStrikeoutSize", "master.customParameters.strikeoutSize"),
    ("OS/2.yStrikeoutPosition", "master.customParameters.strikeoutPosition"),
    ("OS/2.usWeightClass", "instance.weightClass"),
    ("OS/2.usWidthClass", "instance.widthClass"),
    ("OS/2.achVendID", "font.customParameters.vendorID"),
    ("OS/2.fsType", "font.customParameters.fsType"),
    ("OS/2.fsSelection", "instance.customParameters.fsSelection"),
    ("OS/2.ulCodePageRange1", "font.customParameters.codePageRanges"),
    ("OS/2.ulCodePageRange2", "font.customParameters.codePageRanges"),
    ("OS/2.ulUnicodeRange1", "font.customParameters.unicodeRanges"),
    ("OS/2.panose", "instance.customParameters.panose"),
    ("hhea.ascender", "master.customParameters.hheaAscender"),
    ("hhea.descender", "master.customParameters.hheaDescender"),
    ("hhea.lineGap", "master.customParameters.hheaLineGap"),
    ("head.unitsPerEm", "font.unitsPerEm"),
    ("head.fontRevision", "font.versionMajor"),
    ("post.italicAngle", "master.italicAngle"),
    ("post.underlinePosition", "font.customParameters.underlinePosition"),
    ("post.underlineThickness", "font.customParameters.underlineThickness"),
    ("post.isFixedPitch", "font.customParameters.isFixedPitch"),
    ("name.0", "font.copyrights"),
    ("name.1", "font.familyNames"),
    ("name.2", "instance.name"),
    ("name.3", "font.customParameters.uniqueID"),
    ("name.4", "instance.fullName"),
    ("name.5", "font.versionString"),
    ("name.6", "instance.postscriptFontName"),
    ("name.7", "font.trademarks"),
    ("name.8", "font.manufacturers"),
    ("name.9", "font.designers"),
    ("name.10", "font.descriptions"),
    ("name.11", "font.manufacturerURL"),
    ("name.12", "font.designerURL"),
    ("name.13", "font.licenses"),
    ("name.14", "font.licenseURL"),
    ("name.16", "instance.preferredFamilyNames"),
    ("name.17", "instance.preferredSubfamilyNames"),
];

/// The source map of one compiled font: its instance, its glyph order by source glyph name and
/// [`SOURCE_FIELDS`].
fn source_map(instance: Option<String>, glyphs: Vec<String>) -> SourceMap {
    SourceMap { instance, glyphs, fields: SOURCE_FIELDS.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect() }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "audit.checkOutlines",
            "Check Outlines",
            [],
            None,
            "{target?: \"document\" | path | [paths] (default the open document), glyphs?: [names, indexes, first-last ranges, /CID], glyphFile?: path, fix?: bool = false, minArea?: number = 25, tolerance?: number = 0, decimal?: bool, overlapChecks?: bool = true, basicChecks?: bool = true, keepContourOrder?: bool = true, keepOriginal?: \"backupLayer\" | \"none\" = \"backupLayer\", all?: bool, quiet?: bool} the classic outline audit: overlapping contours, coincident points, flat curves, collinear lines, tiny contours and glyphs without contours. With `fix` the open document's reported layers are repaired as one undo step (every repair, not only the safe ones) → the result document of audit.run, with `fix`: the audit.fix result when fixing",
            always,
            check_outlines
        )
        .files(const { &[FileParam::read("target").keywords(&["document"]), FileParam::read("glyphFile")] }),
        cmd!(
            query "audit.checkInterpolation",
            "Check Interpolation",
            [],
            None,
            "{target?: \"document\" | path (default the open document), glyphs?: [names, indexes, ranges, /CID], masters?: [ids], tolerance?: number = 0.95 (10 or more is read as a percentage), kinkiness?: number = 0.5, ignoreMissing?: bool, showAll?: bool} the master-compatibility audit of a multi-master source or designspace: missing glyphs, open paths, path, node and node-type mismatches, contour order, start points, kinks and thin interpolations, each master compared with its neighbour toward the default master. Results keep the problem-type order → the result document of audit.run; with showAll, `glyphsWithoutProblems` lists the other glyphs",
            always,
            check_interpolation
        )
        .files(const { &[FileParam::read("target").keywords(&["document"])] }),
        cmd!(
            query "audit.compareFamily",
            "Check Family",
            [],
            None,
            "{fonts?: [paths], directory?: path, singleTests?: [1-34], familyTests?: [1-21], tolerance?: number = 0, noHints?: bool} the family audit of compiled fonts: the single-face and family checks behind the reference tool's numbered tests (all of them, or the numbers given; a number whose checks are not built is listed in `warnings`), on fonts grouped by name ID 1 and by name ID 16. `tolerance` sets the `tolerance` threshold of the checks that have one; `noHints` leaves out the stem-hint checks → the result document of audit.run",
            always,
            compare_family
        )
        .files(const { &[FileParam::read("fonts"), FileParam::read("directory")] }),
    ]
}

/// A number parameter within `min..=max`.
fn number(p: &Params, key: &str, min: f64, max: f64) -> Result<Option<f64>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() && (min..=max).contains(&n) => Ok(Some(n)),
            _ => Err(p.bad(format!("`{key}` must be a number from {min} to {max}"))),
        },
    }
}

/// Copies the keys of `keys` present in the call into `out`.
fn pass(p: &Params, out: &mut Map<String, Value>, keys: &[&str]) {
    for k in keys {
        if let Some(v) = p.get(k) {
            out.insert((*k).to_string(), v.clone());
        }
    }
}

/// Registered ids among `ids`.
fn registered<'a>(ids: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    ids.into_iter().filter(|id| fontcraft_audit::find(id).is_some()).collect()
}

// ── audit.checkOutlines ─────────────────────────────────────────────────────────────────────────

const BASIC_CHECKS: [&str; 3] = ["check.outlines.coincidentPoints", "check.outlines.flatCurve", "check.outlines.collinearLines"];
const OVERLAP_CHECKS: [&str; 2] = ["check.outlines.overlap", "check.outlines.tinyContour"];
const EMPTY_GLYPHS: &str = "check.outlines.noContours";

fn check_outlines(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.checkOutlines", v)?;
    let fix = p.bool_or("fix", false)?;
    let basic = p.bool_or("basicChecks", true)?;
    let overlap = p.bool_or("overlapChecks", true)?;
    for key in ["decimal", "keepContourOrder", "all", "quiet"] {
        p.bool_or(key, false)?;
    }
    let min_area = number(&p, "minArea", 0.0, 1_000_000.0)?.unwrap_or(25.0);
    let tolerance = number(&p, "tolerance", 0.0, 100.0)?.unwrap_or(0.0);
    let keep = p.opt_str("keepOriginal")?.unwrap_or("backupLayer");
    if !matches!(keep, "backupLayer" | "none") {
        return Err(p.bad("`keepOriginal` must be \"backupLayer\" or \"none\""));
    }
    let on_document =
        matches!(p.get("target"), None | Some(Value::String(_))) && p.get("target").and_then(Value::as_str).is_none_or(|t| t == "document");
    if fix && !on_document {
        return Err(EngineError::Unsupported("fixing a file by path; open it, fix the open document and save it".into()));
    }
    let mut checks: Vec<&str> = Vec::new();
    if basic {
        checks.extend(BASIC_CHECKS);
    }
    if overlap {
        checks.extend(OVERLAP_CHECKS);
    }
    checks.push(EMPTY_GLYPHS);
    let config = json!({
        "check.outlines.flatCurve": {"tolerance": tolerance},
        "check.outlines.collinearLines": {"tolerance": tolerance},
        "check.outlines.tinyContour": {"minArea": min_area},
    });
    let mut q = Map::new();
    pass(&p, &mut q, &["target", "glyphs", "glyphFile"]);
    q.insert("profile".into(), json!("strict"));
    q.insert("checks".into(), json!(checks));
    q.insert("config".into(), config.clone());
    let mut report = s.execute("audit.run", &Value::Object(q))?;
    if fix {
        let mut f = Map::new();
        f.insert("profile".into(), json!("strict"));
        f.insert("checks".into(), json!(checks));
        f.insert("config".into(), config);
        f.insert("safeOnly".into(), json!(false));
        f.insert("keepOriginal".into(), json!(keep));
        if p.get("glyphs").is_some() || p.get("glyphFile").is_some() {
            let names = glyphs_of(&report);
            f.insert("glyphs".into(), json!(names));
        }
        let fixed = s.execute("audit.fix", &Value::Object(f))?;
        if let Some(o) = report.as_object_mut() {
            o.insert("fix".into(), fixed);
        }
    }
    Ok(report)
}

/// The glyphs a result document's results name.
fn glyphs_of(report: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in report.get("results").and_then(Value::as_array).into_iter().flatten() {
        if let Some(g) = r.pointer("/location/glyph").and_then(Value::as_str)
            && !g.is_empty()
            && !out.iter().any(|x| x == g)
        {
            out.push(g.to_string());
        }
    }
    out
}

// ── audit.checkInterpolation ────────────────────────────────────────────────────────────────────

/// The reference's problem types, in its order, with the check of each.
const INTERPOLATION_ORDER: [&str; 9] = [
    "check.variations.glyphMissingInMaster",
    "check.variations.openPathInMaster",
    "check.variations.pathCount",
    "check.variations.nodeCount",
    "check.variations.nodeTypeMismatch",
    "check.variations.contourOrder",
    "check.variations.wrongStartPoint",
    "check.variations.kink",
    "check.variations.underweight",
];

fn check_interpolation(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.checkInterpolation", v)?;
    if matches!(p.get("target"), Some(Value::Array(_))) {
        return Err(p.bad("give one variable font, designspace or multi-master source; several master files are not combined"));
    }
    let tolerance = number(&p, "tolerance", 0.0, 100.0)?.map(|t| if t >= 10.0 { t / 100.0 } else { t }).filter(|t| *t <= 1.0);
    if p.get("tolerance").is_some() && tolerance.is_none() {
        return Err(p.bad("`tolerance` must be from 0 to 1, or a percentage from 10 to 100"));
    }
    let kinkiness = number(&p, "kinkiness", 0.01, 10.0)?;
    let ignore_missing = p.bool_or("ignoreMissing", false)?;
    let show_all = p.bool_or("showAll", false)?;
    let checks = registered(INTERPOLATION_ORDER);
    let mut config = Map::new();
    for id in &checks {
        let Some(spec) = fontcraft_audit::find(id) else { continue };
        let mut values = Map::new();
        if let (Some(t), Some(_)) = (tolerance, spec.threshold("tolerance")) {
            values.insert("tolerance".into(), json!(t));
        }
        if let (Some(k), Some(_)) = (kinkiness, spec.threshold("kinkiness")) {
            values.insert("kinkiness".into(), json!(k));
        }
        if ignore_missing && spec.threshold("ignoreMissing").is_some() {
            values.insert("ignoreMissing".into(), json!(1));
        }
        if !values.is_empty() {
            config.insert((*id).to_string(), Value::Object(values));
        }
    }
    let mut q = Map::new();
    pass(&p, &mut q, &["target", "glyphs", "masters"]);
    q.insert("profile".into(), json!("strict"));
    q.insert("checks".into(), json!(checks));
    q.insert("config".into(), Value::Object(config));
    q.insert("includePassed".into(), json!(show_all));
    let mut report = s.execute("audit.run", &Value::Object(q))?;
    if let Some(Value::Array(results)) = report.get_mut("results") {
        let rank = |r: &Value| {
            let check = r.get("check").and_then(Value::as_str).unwrap_or("");
            INTERPOLATION_ORDER.iter().position(|c| *c == check).unwrap_or(INTERPOLATION_ORDER.len())
        };
        results.sort_by_key(rank);
    }
    if show_all {
        let names = glyph_names(s, &p)?;
        let with: Vec<String> = glyphs_of(&report);
        let without: Vec<String> = names.into_iter().filter(|n| !with.contains(n)).collect();
        if let Some(o) = report.as_object_mut() {
            o.insert("glyphsWithoutProblems".into(), json!(without));
        }
    }
    Ok(report)
}

/// The glyph names of the target, in font order.
fn glyph_names(s: &Session, p: &Params) -> Result<Vec<String>> {
    match p.get("target").and_then(Value::as_str) {
        None | Some("document") => Ok(s.font()?.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect()),
        Some(path) => target_glyph_names(path),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn target_glyph_names(path: &str) -> Result<Vec<String>> {
    let (font, _) = crate::io::open(std::path::Path::new(path), None)?;
    Ok(font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect())
}

#[cfg(target_arch = "wasm32")]
fn target_glyph_names(_: &str) -> Result<Vec<String>> {
    Err(EngineError::Unsupported("reading files by path in the browser".into()))
}

// ── audit.compareFamily ─────────────────────────────────────────────────────────────────────────

/// The reference's single-face tests and the checks behind each (`21-qa.md` § 6). Test 29 is
/// empty in the reference.
const SINGLE_TESTS: [(u8, &[&str]); 34] = [
    (1, &["check.names.menuNameLimits"]),
    (2, &["check.names.menuNameLimits"]),
    (3, &["check.names.fullNameStartsWithFamily"]),
    (4, &["check.names.versionString"]),
    (5, &["check.names.postScriptNameConsistency"]),
    (6, &["check.names.legalAndCredits"]),
    (7, &["check.tables.cffDeprecatedOperators"]),
    (8, &["check.names.subfamilyMatchesStyleBits"]),
    (9, &["check.tables.weightClassRange"]),
    (10, &["check.tables.weightClassRange", "check.hinting.forceBold"]),
    (11, &["check.tables.baseTable", "check.layout.validScriptTags"]),
    (12, &["check.metrics.italicAngle", "check.metrics.italicBitVsAngle"]),
    (13, &["check.metrics.monospaceFlags"]),
    (14, &["check.tables.fsSelection"]),
    (15, &["check.metrics.fontBBox"]),
    (
        16,
        &[
            "check.metrics.unitsPerEm",
            "check.metrics.typoMetricsVsEm",
            "check.metrics.hheaMatchesTypo",
            "check.metrics.winAscentDescent",
            "check.metrics.useTypoMetrics",
            "check.family.unitsPerEm",
        ],
    ),
    (17, &["check.metrics.tabularWidths"]),
    (18, &["check.tables.cffCharstringSize", "check.hinting.missingStemHints", "check.hinting.stemHintSanity"]),
    (19, &["check.glyphset.unicodeCmapMissing", "check.glyphset.doubleEncodedGlyphs"]),
    (20, &["check.names.whitespace"]),
    (21, &["check.names.whitespace"]),
    (22, &["check.metrics.ligatureWidths"]),
    (23, &["check.metrics.accentedWidths"]),
    (24, &["check.layout.sizeFeature"]),
    (25, &["check.tables.cffUniqueIds"]),
    (
        26,
        &[
            "check.glyphset.notdef",
            "check.glyphset.truetypeLeadingGlyphs",
            "check.glyphset.glyphNameBufferSize",
            "check.glyphset.productionNames",
            "check.metrics.tabularWidths",
            "check.metrics.fixedFractionWidths",
        ],
    ),
    (27, &["check.metrics.strikeoutAndScripts"]),
    (28, &["check.tables.codePageRanges"]),
    (29, &[]),
    (30, &["check.hinting.blueValuesCount"]),
    (31, &["check.hinting.blueValuesCount"]),
    (32, &["check.hinting.blueValuesOrder"]),
    (33, &["check.hinting.blueZoneSeparation", "check.hinting.zonesOverlapInSource"]),
    (34, &["check.hinting.blueZoneHeight"]),
];

/// The reference's family tests and the checks behind each.
const FAMILY_TESTS: [(u8, &[&str]); 21] = [
    (1, &["check.family.maxFourFontsPerFamilyName"]),
    (2, &["check.family.nameRecordSets"]),
    (3, &["check.family.consistentFamilyName"]),
    (4, &["check.family.sizeFeatureGroups"]),
    (5, &["check.family.uniqueStyleLinking"]),
    (6, &["check.family.styleLinkedGroupShape", "check.family.boldLinkWeights"]),
    (7, &["check.family.copyrightAndTrademark"]),
    (8, &["check.family.boldLinkWeights"]),
    (9, &["check.family.widthClass"]),
    (10, &["check.metrics.monospaceFlags", "check.family.panoseFamilyType"]),
    (11, &["check.family.macWindowsMenuNames"]),
    (
        12,
        &["check.layout.validScriptTags", "check.layout.validLanguageTags", "check.layout.defaultLanguageSystems", "check.family.layoutConsistency"],
    ),
    (13, &["check.family.wwsBit"]),
    (14, &["check.tables.fsType", "check.family.embeddingPermissions"]),
    (15, &["check.family.underlineConsistency"]),
    (16, &["check.family.glyphWidthRatio"]),
    (17, &["check.tables.os2Version"]),
    (18, &["check.hinting.blueValuesCount", "check.hinting.blueZonesOutsideBBox", "check.family.blueValueArraySizes"]),
    (19, &["check.family.familyBlues"]),
    (20, &["check.family.blueScaleAndShift"]),
    (21, &["check.family.blueScaleAndShift"]),
];

/// The stem-hint checks `noHints` leaves out.
const HINT_CHECKS: [&str; 2] = ["check.hinting.missingStemHints", "check.hinting.stemHintSanity"];

/// The test numbers of `key` (all of `table` when absent), each within the table.
fn tests<'t>(p: &Params, key: &str, table: &'t [(u8, &'t [&'t str])]) -> Result<Vec<&'t (u8, &'t [&'t str])>> {
    let Some(list) = p.opt_array(key)? else { return Ok(table.iter().collect()) };
    let max = table.len();
    let mut out = Vec::new();
    for v in list {
        let n = v.as_u64().filter(|n| (1..=max as u64).contains(n)).ok_or_else(|| p.bad(format!("`{key}` holds test numbers from 1 to {max}")))?;
        if let Some(entry) = table.iter().find(|(t, _)| u64::from(*t) == n)
            && !out.contains(&entry)
        {
            out.push(entry);
        }
    }
    Ok(out)
}

fn compare_family(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.compareFamily", v)?;
    let mut target: Vec<Value> = Vec::new();
    if let Some(fonts) = p.opt_strings("fonts")? {
        target.extend(fonts.into_iter().map(|f| json!(f)));
    }
    if let Some(dir) = p.opt_str("directory")? {
        target.push(json!(dir));
    }
    if target.is_empty() {
        return Err(p.bad("give `fonts` or `directory`"));
    }
    let tolerance = number(&p, "tolerance", 0.0, 10_000.0)?;
    let no_hints = p.bool_or("noHints", false)?;
    let mut wanted: Vec<&str> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for (kind, (n, ids)) in tests(&p, "singleTests", &SINGLE_TESTS)?
        .into_iter()
        .map(|t| ("single-face", t))
        .chain(tests(&p, "familyTests", &FAMILY_TESTS)?.into_iter().map(|t| ("family", t)))
    {
        let built = registered(ids.iter().copied());
        if built.is_empty() && !ids.is_empty() {
            missing.push(format!("{kind} test {n}"));
        }
        for id in built {
            if !(no_hints && HINT_CHECKS.contains(&id)) && !wanted.contains(&id) {
                wanted.push(id);
            }
        }
    }
    if wanted.is_empty() {
        return Err(p.bad("none of the tests asked for has a check built yet"));
    }
    let mut config = Map::new();
    if let Some(t) = tolerance {
        for id in &wanted {
            if let Some(th) = fontcraft_audit::find(id).and_then(|c| c.threshold("tolerance")) {
                config.insert((*id).to_string(), json!({"tolerance": t.clamp(th.min, th.max)}));
            }
        }
    }
    let q = json!({"target": target, "profile": "strict", "checks": wanted, "config": config});
    let mut report = s.execute("audit.run", &q)?;
    if !missing.is_empty()
        && let Some(o) = report.as_object_mut()
    {
        let note = format!("no check is built yet for {}", missing.join(", "));
        match o.get_mut("warnings") {
            Some(Value::Array(w)) => w.push(json!(note)),
            _ => {
                o.insert("warnings".into(), json!([note]));
            }
        }
    }
    Ok(report)
}
